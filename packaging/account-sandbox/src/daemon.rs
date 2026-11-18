//! The daemon's state machine: accounts, the fence, credential generations
//! and unit ownership. Every account operation runs under that account's lock
//! and persists `meta.json` before it acknowledges anything; every path inside
//! an account tree is opened beneath the state root descriptor (R3-2).

use std::collections::{HashMap, HashSet};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use serde_json::{Value, json};

use crate::config::Config;
use crate::host::Host;
use crate::ids;
use crate::meta::{self, AccountMeta, Cleanup, CredentialState, LogoutRecord, UnitRecord};
use crate::protocol::{Code, Mode, Outcome, Request, fail};
use crate::safefs::{self, STRICT};
use crate::store;
use crate::units::{
    self, AccountPaths, GEMINI_CONFIG_DIR, GEMINI_DIR, GEMINI_STATE_DIR, HOOKS_FILE, SETTINGS_FILE,
    TOKEN_FILE,
};

pub const ACCOUNTS_DIR: &str = "accounts";
pub const UNITS_DIR: &str = "units";
const POLL: Duration = Duration::from_millis(5);

/// A spawned unit's exit as seen by its waiter thread.
#[derive(Debug, Clone, Copy)]
enum ExitSlot {
    Running,
    Exited(Option<i32>),
}

/// One client connection.
#[derive(Debug, Clone)]
pub struct Conn {
    pub id: String,
    pub peer_uid: u32,
}

/// A response value and the descriptors sent with it.
pub struct Reply {
    pub value: Value,
    pub fds: Vec<OwnedFd>,
}

impl Reply {
    fn value(value: Value) -> Self {
        Self {
            value,
            fds: Vec::new(),
        }
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Owner of root-held daemon files: root when running as root, the current
/// user in unprivileged tests.
pub fn root_owner() -> Option<(u32, u32)> {
    rustix::process::geteuid().is_root().then_some((0, 0))
}

pub struct Daemon {
    pub config: Config,
    pub(crate) host: Arc<dyn Host>,
    pub(crate) root: OwnedFd,
    pub(crate) run: OwnedFd,
    instance: String,
    pub(crate) boot_id: String,
    pub(crate) mask_var_run: bool,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub(crate) store_lock: RwLock<()>,
    live_connections: Mutex<HashSet<String>>,
    pub(crate) owned: Mutex<HashMap<String, Vec<(String, String)>>>,
    exits: Mutex<HashMap<String, ExitSlot>>,
    exit_signal: Condvar,
    next_connection: AtomicU64,
}

impl Daemon {
    pub fn new(
        config: Config,
        host: Arc<dyn Host>,
        mask_var_run: bool,
    ) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&config.state_root)?;
        std::fs::create_dir_all(&config.run_dir)?;
        let root = safefs::open_root(&config.state_root)?;
        safefs::set_owner_mode(&root, 0o711, root_owner())?;
        let run = safefs::open_root(&config.run_dir)?;
        safefs::ensure_dir(root.as_fd(), ACCOUNTS_DIR, 0o711, root_owner())?;
        let boot_id = host.boot_id()?;
        let daemon = Arc::new(Self {
            config,
            host,
            root,
            run,
            instance: safefs::nonce(4)?,
            boot_id,
            mask_var_run,
            locks: Mutex::new(HashMap::new()),
            store_lock: RwLock::new(()),
            live_connections: Mutex::new(HashSet::new()),
            owned: Mutex::new(HashMap::new()),
            exits: Mutex::new(HashMap::new()),
            exit_signal: Condvar::new(),
            next_connection: AtomicU64::new(1),
        });
        Ok(daemon)
    }

    /// Register a new connection.
    pub fn connect(&self, peer_uid: u32) -> Conn {
        let id = format!(
            "{}-{}",
            self.instance,
            self.next_connection.fetch_add(1, Ordering::Relaxed)
        );
        guard(&self.live_connections).insert(id.clone());
        Conn { id, peer_uid }
    }

    /// The connection closed: stop and confirm every unit it owns (R2-9).
    pub fn disconnect(&self, conn: &Conn) {
        guard(&self.live_connections).remove(&conn.id);
        let owned = guard(&self.owned).remove(&conn.id).unwrap_or_default();
        for (key, unit) in owned {
            let result = self.with_account(&key, |dir, meta| {
                if meta
                    .units
                    .iter()
                    .any(|record| record.unit == unit && record.connection == conn.id)
                {
                    self.stop_and_confirm(dir, meta, &unit)?;
                    Self::save(dir, meta)?;
                }
                Ok(())
            });
            if let Err(error) = result {
                eprintln!("cowboy-account-sandbox: stopping {unit} after disconnect: {error}");
            }
        }
    }

    pub(crate) fn lock(&self, key: &str) -> Arc<Mutex<()>> {
        Arc::clone(guard(&self.locks).entry(key.to_owned()).or_default())
    }

    pub(crate) fn accounts(&self) -> anyhow::Result<OwnedFd> {
        safefs::open_dir(self.root.as_fd(), ACCOUNTS_DIR, STRICT)
    }

    pub(crate) fn paths(&self, key: &str) -> AccountPaths {
        AccountPaths::new(&self.config.state_root, key)
    }

    /// Run `f` on the account's directory and meta under the account lock.
    pub(crate) fn with_account<T>(
        &self,
        key: &str,
        f: impl FnOnce(&OwnedFd, &mut AccountMeta) -> Outcome<T>,
    ) -> Outcome<T> {
        let lock = self.lock(key);
        let _guard = guard(&lock);
        let accounts = self.accounts()?;
        if !safefs::exists(accounts.as_fd(), key)? {
            return Err(fail(Code::NotFound, "unknown account"));
        }
        let dir = safefs::open_dir(accounts.as_fd(), key, STRICT)?;
        let mut meta = meta::load(dir.as_fd())?
            .ok_or_else(|| fail(Code::NotFound, "account has no metadata"))?;
        f(&dir, &mut meta)
    }

    pub(crate) fn save(dir: &OwnedFd, meta: &AccountMeta) -> Outcome<()> {
        meta::save(dir.as_fd(), meta)?;
        Ok(())
    }

    pub fn handle(
        self: &Arc<Self>,
        conn: &Conn,
        request: Request,
        payload: &mut dyn std::io::Read,
    ) -> Outcome<Reply> {
        match request {
            Request::EnsureAccount {
                service_id,
                account_id,
            } => self.ensure_account(&service_id, &account_id),
            Request::RegisterRuntime { sha256, size } => {
                self.register_runtime(&sha256, size, payload)
            }
            Request::Start {
                key,
                mode,
                credential_generation,
                runtime_sha256,
                session_id,
                descriptor_b64,
            } => self.start(
                conn,
                &crate::start::StartRequest {
                    key: &key,
                    mode,
                    credential_generation,
                    runtime_sha256: &runtime_sha256,
                    session_id: session_id.as_deref(),
                    descriptor_b64: descriptor_b64.as_deref(),
                },
            ),
            Request::StopUnit { unit } => self.stop_unit(&unit),
            Request::StopAccount { key } => self.stop_account(&key),
            Request::AdmitAccount { key } => self.admit_account(&key),
            Request::ListUnits { key } => self.list_units(&key),
            Request::UnitState { unit } => self.unit_state(&unit),
            Request::LogoutCleanup { key } => self.logout_cleanup(&key),
            Request::RemoveAccount { key } => self.remove_account(&key),
        }
    }

    pub(crate) fn check_space(&self) -> Outcome<()> {
        let free = self.host.free_bytes(&self.config.state_root)?;
        if free < self.config.free_space_reserve_bytes {
            return Err(fail(
                Code::InsufficientSpace,
                format!("{free} bytes free is below the reserve"),
            ));
        }
        Ok(())
    }

    fn ensure_account(&self, service_id: &str, account_id: &str) -> Outcome<Reply> {
        let key = ids::derive_key(service_id, account_id);
        let user = ids::user_name(&key);
        let lock = self.lock(&key);
        let _guard = guard(&lock);
        let accounts = self.accounts()?;
        let dir = safefs::ensure_dir(accounts.as_fd(), &key, 0o711, root_owner())?;
        let mut meta = if let Some(meta) = meta::load(dir.as_fd())? {
            if meta.service_id != service_id || meta.account_id != account_id {
                return Err(fail(Code::Conflict, "account key collision"));
            }
            meta
        } else {
            self.check_space()?;
            let owner = self.host.ensure_user(&user)?;
            let meta = AccountMeta::new(&key, service_id, account_id, &user, owner);
            Self::save(&dir, &meta)?;
            meta
        };
        let owner = self.host.ensure_user(&user)?;
        if owner != meta.owner() {
            meta.uid = owner.0;
            meta.gid = owner.1;
            Self::save(&dir, &meta)?;
        }
        if !meta.image_ready {
            self.check_space()?;
            self.create_image(&dir, &key, &meta)?;
            meta.image_ready = true;
            Self::save(&dir, &meta)?;
        }
        let home = self.mount_home(&dir, &key, &meta)?;
        Self::prepare_home(&dir, &home, &meta)?;
        Ok(Reply::value(json!({ "key": key, "user": user })))
    }

    fn create_image(&self, dir: &OwnedFd, key: &str, meta: &AccountMeta) -> Outcome<()> {
        safefs::remove_file(dir.as_fd(), "home.img")?;
        let file = safefs::create_new(dir.as_fd(), "home.img", 0o600)?;
        // Sparse: blocks are allocated only as the account writes.
        file.set_len(self.config.home_image_bytes)
            .context("size home image")?;
        file.sync_all()?;
        drop(file);
        let paths = self.paths(key);
        self.host
            .format_image(&paths.image, self.config.home_image_inodes, meta.owner())?;
        Ok(())
    }

    /// Mount the account's home image when needed and open its root.
    pub(crate) fn mount_home(
        &self,
        dir: &OwnedFd,
        key: &str,
        meta: &AccountMeta,
    ) -> Outcome<OwnedFd> {
        let paths = self.paths(key);
        if !self.host.is_mounted(&paths.home)? {
            if !meta.image_ready {
                return Err(fail(Code::Precondition, "home image is not ready"));
            }
            safefs::ensure_dir(dir.as_fd(), "home", 0o700, root_owner())?;
            self.host.mount_image(&paths.image, &paths.home)?;
        }
        let home = safefs::open_dir(dir.as_fd(), "home", safefs::MOUNT_ROOT)?;
        safefs::set_owner_mode(&home, 0o700, Some(meta.owner()))?;
        Ok(home)
    }

    /// Open the mounted home's root, refusing an unmounted mount point.
    pub(crate) fn open_home(&self, dir: &OwnedFd, key: &str) -> Outcome<OwnedFd> {
        if !self.host.is_mounted(&self.paths(key).home)? {
            return Err(fail(Code::Precondition, "home image is not mounted"));
        }
        Ok(safefs::open_dir(dir.as_fd(), "home", safefs::MOUNT_ROOT)?)
    }

    /// The account-owned `~/.gemini/antigravity-cli` state directory with its
    /// `settings.json` bind target, and the root-owned assembled `~/.gemini`
    /// tree bound read-only over it (R3-14).
    pub(crate) fn prepare_home(dir: &OwnedFd, home: &OwnedFd, meta: &AccountMeta) -> Outcome<()> {
        let owner = Some(meta.owner());
        let gemini = safefs::ensure_dir(home.as_fd(), GEMINI_DIR, 0o700, owner)?;
        let state = safefs::ensure_dir(gemini.as_fd(), GEMINI_STATE_DIR, 0o700, owner)?;
        safefs::ensure_file(state.as_fd(), SETTINGS_FILE, 0o600, owner)?;
        let tree = safefs::ensure_dir(dir.as_fd(), "gemini", 0o755, root_owner())?;
        let config = safefs::ensure_dir(tree.as_fd(), GEMINI_CONFIG_DIR, 0o755, root_owner())?;
        safefs::ensure_file(config.as_fd(), HOOKS_FILE, 0o644, root_owner())?;
        safefs::ensure_dir(tree.as_fd(), GEMINI_STATE_DIR, 0o755, root_owner())?;
        Ok(())
    }

    pub(crate) fn token_present(home: &OwnedFd) -> Outcome<bool> {
        Ok(safefs::exists(
            home.as_fd(),
            &format!("{GEMINI_DIR}/{GEMINI_STATE_DIR}/{TOKEN_FILE}"),
        )?)
    }

    /// Remove only the vendor token file; `true` when it existed.
    pub(crate) fn remove_token(home: &OwnedFd) -> Outcome<bool> {
        let path = format!("{GEMINI_DIR}/{GEMINI_STATE_DIR}");
        if !safefs::exists(home.as_fd(), &path)? {
            return Ok(false);
        }
        let state = safefs::open_dir(home.as_fd(), &path, STRICT)?;
        Ok(safefs::remove_file(state.as_fd(), TOKEN_FILE)?)
    }

    fn register_runtime(
        &self,
        sha256: &str,
        size: u64,
        payload: &mut dyn std::io::Read,
    ) -> Outcome<Reply> {
        if size > self.config.max_runtime_archive_bytes {
            return Err(fail(Code::Limit, "runtime archive exceeds the size limit"));
        }
        let store = store::open_store(self.root.as_fd())?;
        let live = || self.live_digests();
        let present = store::register(
            store.as_fd(),
            &self.config,
            sha256,
            size,
            &mut &mut *payload,
            &self.store_lock,
            &live,
        )?;
        Ok(Reply::value(
            json!({ "sha256": sha256, "already_present": present }),
        ))
    }

    /// Runtime digests referenced by any persisted unit.
    fn live_digests(&self) -> Outcome<HashSet<String>> {
        let accounts = self.accounts()?;
        let mut live = HashSet::new();
        for key in safefs::list(accounts.as_fd())? {
            let dir = safefs::open_dir(accounts.as_fd(), &key, STRICT)?;
            if let Some(meta) = meta::load(dir.as_fd())? {
                live.extend(meta.units.into_iter().map(|record| record.runtime_sha256));
            }
        }
        Ok(live)
    }

    /// Record a unit's exit from its waiter thread.
    pub(crate) fn record_exit(&self, unit: &str, exit: Option<i32>) {
        if let Some(slot) = guard(&self.exits).get_mut(unit) {
            *slot = ExitSlot::Exited(exit);
        }
        self.exit_signal.notify_all();
    }

    pub(crate) fn expect_exit(&self, unit: &str) {
        guard(&self.exits).insert(unit.to_owned(), ExitSlot::Running);
    }

    /// The exit status of a unit this instance spawned, waiting up to
    /// `timeout`; `None` when it was killed, is unknown or still runs.
    pub(crate) fn wait_exit(&self, unit: &str, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        let mut exits = guard(&self.exits);
        loop {
            match exits.get(unit) {
                None => return None,
                Some(ExitSlot::Exited(exit)) => return *exit,
                Some(ExitSlot::Running) => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            exits = self
                .exit_signal
                .wait_timeout(exits, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    pub(crate) fn exited(&self, unit: &str) -> bool {
        matches!(guard(&self.exits).get(unit), Some(ExitSlot::Exited(_)))
    }

    /// Stop `unit`, wait for its process and confirm systemd no longer runs
    /// it; then settle its record.
    pub(crate) fn stop_and_confirm(
        &self,
        dir: &OwnedFd,
        meta: &mut AccountMeta,
        unit: &str,
    ) -> Outcome<()> {
        let timeout = Duration::from_secs(self.config.stop_timeout_secs);
        if self
            .host
            .unit_status(unit)?
            .is_some_and(|status| status.is_live())
        {
            self.host.stop_unit(unit)?;
        }
        let exit = self.wait_exit(unit, timeout);
        self.confirm_gone(unit, timeout)?;
        if let Some(index) = meta.units.iter().position(|record| record.unit == unit) {
            let record = meta.units.remove(index);
            self.finish_unit(dir, meta, &record, exit);
        }
        Ok(())
    }

    fn confirm_gone(&self, unit: &str, timeout: Duration) -> Outcome<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if !self
                .host
                .unit_status(unit)?
                .is_some_and(|status| status.is_live())
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(fail(Code::Internal, format!("{unit} did not stop")));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Settle a finished unit's record: credential transitions, its runtime
    /// files and in-memory ownership. The caller saves `meta`.
    pub(crate) fn finish_unit(
        &self,
        dir: &OwnedFd,
        meta: &mut AccountMeta,
        record: &UnitRecord,
        exit: Option<i32>,
    ) {
        if let Ok(units_dir) = safefs::open_dir(self.run.as_fd(), UNITS_DIR, STRICT) {
            let _ = safefs::remove_tree(units_dir.as_fd(), &record.nonce);
        }
        guard(&self.exits).remove(&record.unit);
        if let Some(owned) = guard(&self.owned).get_mut(&record.connection) {
            owned.retain(|(_, unit)| unit != &record.unit);
        }
        let credential = &mut meta.credential;
        let same_generation = credential.generation == record.credential_generation;
        match record.mode {
            Mode::Login if same_generation && credential.state == CredentialState::LoginRunning => {
                let signed_in = exit == Some(0)
                    && self
                        .open_home(dir, &meta.key)
                        .and_then(|home| Self::token_present(&home))
                        .unwrap_or(false);
                if signed_in {
                    meta.credential.state = CredentialState::SignedIn;
                } else {
                    meta.credential.state = CredentialState::LoginInterrupted;
                    meta.cleanups.push(Cleanup::PartialToken {
                        generation: record.credential_generation,
                    });
                }
            }
            Mode::Logout
                if same_generation && credential.state == CredentialState::LogoutRunning =>
            {
                meta.credential.state = CredentialState::LogoutPending;
                meta.cleanups.push(Cleanup::LogoutToken {
                    generation: record.credential_generation,
                });
            }
            _ => {}
        }
        self.run_cleanups(dir, meta);
    }

    /// Retry interrupted-login cleanups; each applies only to the generation
    /// it owns. Failures stay recorded.
    pub(crate) fn run_cleanups(&self, dir: &OwnedFd, meta: &mut AccountMeta) {
        let current = meta.credential.clone();
        let key = meta.key.clone();
        meta.cleanups.retain(|cleanup| match cleanup {
            Cleanup::PartialToken { generation } => {
                if *generation != current.generation
                    || current.state != CredentialState::LoginInterrupted
                {
                    return false;
                }
                let removed = self
                    .open_home(dir, &key)
                    .and_then(|home| Self::remove_token(&home));
                removed.is_err()
            }
            Cleanup::LogoutToken { generation } => *generation == current.generation,
        });
    }

    /// The waiter thread saw `unit` finish: settle it unless a stop already did.
    pub(crate) fn unit_finished(&self, key: &str, unit: &str, exit: Option<i32>) {
        self.record_exit(unit, exit);
        let result = self.with_account(key, |dir, meta| {
            if let Some(index) = meta.units.iter().position(|record| record.unit == unit) {
                if meta.units[index].phase == meta::UnitPhase::Starting {
                    // The starter still holds the lock; it settles failures.
                    return Ok(());
                }
                let record = meta.units.remove(index);
                self.finish_unit(dir, meta, &record, exit);
                Self::save(dir, meta)?;
            }
            Ok(())
        });
        if let Err(error) = result {
            eprintln!("cowboy-account-sandbox: settling {unit}: {error}");
        }
    }

    fn stop_unit(&self, unit: &str) -> Outcome<Reply> {
        let key = crate::protocol::validate_unit_name(unit)
            .map_err(|e| fail(Code::InvalidRequest, e.to_string()))?;
        self.with_account(key, |dir, meta| {
            self.stop_and_confirm(dir, meta, unit)?;
            Self::save(dir, meta)?;
            Ok(Reply::value(json!({ "unit": unit, "stopped": true })))
        })
    }

    /// Stop every recorded and every live unit of the account and confirm
    /// none remains; returns the stopped unit names.
    pub(crate) fn stop_all(
        &self,
        dir: &OwnedFd,
        meta: &mut AccountMeta,
        filter: impl Fn(Mode) -> bool,
    ) -> Outcome<Vec<String>> {
        let mut names: Vec<String> = meta
            .units
            .iter()
            .filter(|record| filter(record.mode))
            .map(|record| record.unit.clone())
            .collect();
        for unit in self
            .host
            .list_units(&format!("{}-*", ids::user_name(&meta.key)))?
        {
            let mode = crate::protocol::validate_unit_name(&unit)
                .ok()
                .and_then(|_| unit.split('-').nth(2).map(str::to_owned));
            let wanted = match mode.as_deref() {
                Some("runtime") => filter(Mode::Runtime),
                Some("probe") => filter(Mode::Probe),
                Some("login") => filter(Mode::Login),
                Some("logout") => filter(Mode::Logout),
                _ => true,
            };
            if wanted && !names.contains(&unit) {
                names.push(unit);
            }
        }
        for unit in &names {
            self.stop_and_confirm(dir, meta, unit)?;
        }
        Ok(names)
    }

    fn stop_account(&self, key: &str) -> Outcome<Reply> {
        self.with_account(key, |dir, meta| {
            meta.fence_epoch += 1;
            meta.fenced = Some(meta.fence_epoch);
            Self::save(dir, meta)?;
            let stopped = self.stop_all(dir, meta, |_| true);
            Self::save(dir, meta)?;
            let stopped = stopped?;
            let slice = units::slice_name(key);
            if self
                .host
                .unit_status(&slice)?
                .is_some_and(|status| status.is_live())
            {
                self.host.stop_unit(&slice)?;
            }
            let remaining = self
                .host
                .list_units(&format!("{}-*", ids::user_name(key)))?;
            let live: Vec<String> = remaining
                .into_iter()
                .filter(|unit| {
                    self.host
                        .unit_status(unit)
                        .ok()
                        .flatten()
                        .is_some_and(|status| status.is_live())
                })
                .collect();
            if !live.is_empty() {
                return Err(fail(
                    Code::Internal,
                    format!("units still running: {}", live.join(" ")),
                ));
            }
            Ok(Reply::value(
                json!({ "key": key, "fence_epoch": meta.fence_epoch, "stopped": stopped }),
            ))
        })
    }

    fn admit_account(&self, key: &str) -> Outcome<Reply> {
        self.with_account(key, |dir, meta| {
            meta.fenced = None;
            Self::save(dir, meta)?;
            Ok(Reply::value(
                json!({ "key": key, "fence_epoch": meta.fence_epoch, "fenced": false }),
            ))
        })
    }

    fn list_units(&self, key: &str) -> Outcome<Reply> {
        self.with_account(key, |_, meta| {
            let mut units = Vec::new();
            for record in &meta.units {
                units.push(self.describe(record)?);
            }
            let recorded: HashSet<&str> = meta
                .units
                .iter()
                .map(|record| record.unit.as_str())
                .collect();
            let unrecorded: Vec<String> = self
                .host
                .list_units(&format!("{}-*", ids::user_name(key)))?
                .into_iter()
                .filter(|unit| !recorded.contains(unit.as_str()))
                .collect();
            Ok(Reply::value(json!({
                "key": key,
                "boot_id": self.boot_id,
                "fenced": meta.fenced,
                "credential": meta.credential,
                "units": units,
                "unrecorded": unrecorded,
            })))
        })
    }

    fn describe(&self, record: &UnitRecord) -> Outcome<Value> {
        let status = self.host.unit_status(&record.unit)?;
        let live = status
            .as_ref()
            .is_some_and(crate::host::UnitStatus::is_live);
        let same_invocation = status
            .as_ref()
            .and_then(|status| status.invocation_id.as_ref())
            == record.invocation_id.as_ref();
        Ok(json!({
            "unit": record.unit,
            "mode": record.mode,
            "invocation_id": record.invocation_id,
            "boot_id": record.boot_id,
            "credential_generation": record.credential_generation,
            "session_id": record.session_id,
            "state": status.map_or_else(|| "gone".to_owned(), |status| status.active_state),
            "alive": live && same_invocation && record.boot_id == self.boot_id,
            "owned": guard(&self.live_connections).contains(&record.connection),
        }))
    }

    fn unit_state(&self, unit: &str) -> Outcome<Reply> {
        let key = crate::protocol::validate_unit_name(unit)
            .map_err(|e| fail(Code::InvalidRequest, e.to_string()))?;
        self.with_account(key, |_, meta| {
            if let Some(record) = meta.units.iter().find(|record| record.unit == unit) {
                let mut value = self.describe(record)?;
                value["recorded"] = json!(true);
                value["current_boot_id"] = json!(self.boot_id);
                return Ok(Reply::value(value));
            }
            let status = self.host.unit_status(unit)?;
            Ok(Reply::value(json!({
                "unit": unit,
                "recorded": false,
                "state": status.map_or_else(|| "gone".to_owned(), |status| status.active_state),
                "alive": false,
                "owned": false,
                "current_boot_id": self.boot_id,
            })))
        })
    }

    fn logout_cleanup(&self, key: &str) -> Outcome<Reply> {
        self.with_account(key, |dir, meta| {
            if !meta.units.is_empty() {
                return Err(fail(
                    Code::Conflict,
                    "units of the account are still running",
                ));
            }
            let home = self.mount_home(dir, key, meta)?;
            let removed = Self::remove_token(&home)?;
            let generation = meta.credential.generation;
            meta.credential.state = CredentialState::LoggedOut;
            meta.cleanups
                .retain(|cleanup| !matches!(cleanup, Cleanup::LogoutToken { .. }));
            meta.last_logout = Some(LogoutRecord {
                generation,
                credential_cleared: true,
                history_retained: true,
            });
            Self::save(dir, meta)?;
            Ok(Reply::value(json!({
                "key": key,
                "credential_generation": generation,
                "token_removed": removed,
                "credential_cleared": true,
                "history_retained": true,
            })))
        })
    }

    fn remove_account(&self, key: &str) -> Outcome<Reply> {
        let result = self.with_account(key, |dir, meta| {
            if meta.fenced.is_none() {
                return Err(fail(
                    Code::Precondition,
                    "remove_account requires a fenced account",
                ));
            }
            if !matches!(
                meta.credential.state,
                CredentialState::SignedOut
                    | CredentialState::LoggedOut
                    | CredentialState::LoginInterrupted
            ) {
                return Err(fail(
                    Code::Precondition,
                    "remove_account requires a logged-out account",
                ));
            }
            self.stop_all(dir, meta, |_| true)?;
            Self::save(dir, meta)?;
            let paths = self.paths(key);
            if self.host.is_mounted(&paths.home)? {
                let home = self.open_home(dir, key)?;
                if Self::token_present(&home)? {
                    return Err(fail(
                        Code::Precondition,
                        "a token is present; run logout_cleanup first",
                    ));
                }
                drop(home);
                self.host.unmount(&paths.home)?;
            }
            for name in safefs::list(dir.as_fd())? {
                if name != meta::META_FILE {
                    safefs::remove_tree(dir.as_fd(), &name)?;
                }
            }
            let accounts = self.accounts()?;
            safefs::remove_tree(accounts.as_fd(), key)?;
            self.host.remove_user(&meta.user)?;
            Ok(Reply::value(json!({ "key": key, "removed": true })))
        });
        if result.is_ok() {
            guard(&self.locks).remove(key);
        }
        result
    }

    /// Activation reconciliation (R3-12): remount images, stop every unit no
    /// connection of this instance owns (all of them, after a restart), settle
    /// their records and retry unresolved cleanups.
    pub fn reconcile(&self) -> anyhow::Result<()> {
        let store = store::open_store(self.root.as_fd())?;
        store::sweep(store.as_fd())?;
        if safefs::exists(self.run.as_fd(), UNITS_DIR)? {
            safefs::remove_tree(self.run.as_fd(), UNITS_DIR)?;
        }
        safefs::ensure_dir(self.run.as_fd(), UNITS_DIR, 0o711, root_owner())?;
        let accounts = self.accounts()?;
        for key in safefs::list(accounts.as_fd())? {
            if ids::validate_key(&key).is_err() {
                continue;
            }
            let result = self.with_account(&key, |dir, meta| {
                if meta.image_ready {
                    self.mount_home(dir, &key, meta)?;
                }
                for record in meta.units.clone() {
                    self.stop_and_confirm(dir, meta, &record.unit)?;
                }
                self.stop_all(dir, meta, |_| true)?;
                self.run_cleanups(dir, meta);
                Self::save(dir, meta)
            });
            if let Err(error) = result {
                eprintln!("cowboy-account-sandbox: reconciling account {key}: {error}");
            }
        }
        for unit in self.host.list_units(&format!("{}*", ids::USER_PREFIX))? {
            if self
                .host
                .unit_status(&unit)?
                .is_some_and(|status| status.is_live())
            {
                eprintln!("cowboy-account-sandbox: stopping unclaimed {unit}");
                self.host.stop_unit(&unit)?;
            }
        }
        Ok(())
    }

    pub(crate) fn run_fd(&self) -> BorrowedFd<'_> {
        self.run.as_fd()
    }
}

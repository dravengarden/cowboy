//! `start`: admission checks (fence, authentication exclusivity, credential
//! generation, unit limits, free space), the unit's private runtime files and
//! the `systemd-run` launch whose ownership is persisted before the reply.

use std::io::Write as _;
use std::os::fd::{AsFd as _, OwnedFd};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use rustix::fs::{AtFlags, Gid, Mode as FileMode, Uid};
use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
use rustix::pipe::PipeFlags;
use serde_json::json;

use crate::daemon::{Conn, Daemon, Reply, UNITS_DIR, root_owner};
use crate::host::UnitStdio;
use crate::meta::{AccountMeta, CredentialState, UnitPhase, UnitRecord};
use crate::protocol::{Code, Mode, Outcome, decode_base64, fail};
use crate::safefs;
use crate::store;
use crate::units::{self, UnitSpec};

const EXEC_SOCKET: &str = "exec.sock";
const DESCRIPTOR_FILE: &str = "descriptor";
const POLL: Duration = Duration::from_millis(5);
#[cfg(not(test))]
const INVOCATION_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const INVOCATION_TIMEOUT: Duration = Duration::from_millis(300);

pub struct StartRequest<'a> {
    pub key: &'a str,
    pub mode: Mode,
    pub credential_generation: u64,
    pub runtime_sha256: &'a str,
    pub session_id: Option<&'a str>,
    pub descriptor_b64: Option<&'a str>,
}

/// Refuse a start the account's state does not admit (R3-4, R3-8, R3-16).
pub fn admit(
    meta: &AccountMeta,
    request: &StartRequest<'_>,
    max_runtime: Option<usize>,
) -> Outcome<()> {
    let mode = request.mode;
    if mode != Mode::Logout && meta.fenced.is_some() {
        return Err(fail(Code::Fenced, "the account is fenced"));
    }
    if meta.auth_unit().is_some() {
        return Err(if mode.is_auth() {
            fail(Code::Limit, "an authentication unit is already running")
        } else {
            fail(
                Code::Fenced,
                "an authentication transition holds the account",
            )
        });
    }
    let credential = &meta.credential;
    let generation = request.credential_generation;
    match mode {
        // A quota report reads the same credential a runtime uses.
        Mode::Runtime | Mode::Usage => {
            if generation < credential.generation {
                return Err(fail(Code::Precondition, "older credential generation"));
            }
            if generation != credential.generation || credential.state != CredentialState::SignedIn
            {
                return Err(fail(
                    Code::Precondition,
                    "the account is not signed in at this generation",
                ));
            }
        }
        Mode::Login => {
            if generation <= credential.generation {
                return Err(fail(
                    Code::Precondition,
                    "login needs a new credential generation",
                ));
            }
        }
        Mode::Logout => {
            if generation != credential.generation {
                return Err(fail(
                    Code::Precondition,
                    "logout must name the current credential generation",
                ));
            }
            if !matches!(
                credential.state,
                CredentialState::SignedIn | CredentialState::LogoutPending
            ) {
                return Err(fail(Code::Precondition, "the account is not signed in"));
            }
        }
        Mode::Probe => {}
    }
    if !mode.is_auth() && max_runtime.is_some_and(|max| meta.runtime_units() >= max) {
        return Err(fail(
            Code::Limit,
            "the account already runs its maximum number of units",
        ));
    }
    Ok(())
}

impl Daemon {
    pub(crate) fn start(
        self: &Arc<Self>,
        conn: &Conn,
        request: &StartRequest<'_>,
    ) -> Outcome<Reply> {
        let key = request.key;
        self.with_account(key, |dir, meta| {
            admit(meta, request, self.config.max_runtime_units)?;
            self.check_space()?;
            let store_guard = self
                .store_lock
                .read()
                .unwrap_or_else(PoisonError::into_inner);
            let store_dir = store::open_store(self.root.as_fd())?;
            let manifest = store::load(
                store_dir.as_fd(),
                request.runtime_sha256,
                &self.config.agy_allow_list,
            )?;
            let home = self.mount_home(dir, key, meta)?;
            self.admit_home_capacity(dir, key, request.mode)?;
            Self::prepare_home(dir, &home, meta)?;
            if request.mode == Mode::Login && Self::token_present(&home)? {
                return Err(fail(
                    Code::Precondition,
                    "a token is present; login requires a signed-out account",
                ));
            }
            drop(home);
            if request.mode.is_auth() {
                // R3-8: the authentication transition is exclusive.
                self.stop_all(dir, meta, |mode| !mode.is_auth())?;
                Self::save(dir, meta)?;
            }

            // The pipes are allocated before the unit's files and its durable
            // `Starting` record, so their failure strands neither.
            let (stdin_read, stdin_write) =
                rustix::pipe::pipe_with(PipeFlags::CLOEXEC).context("pipe")?;
            let (stdout_read, stdout_write) =
                rustix::pipe::pipe_with(PipeFlags::CLOEXEC).context("pipe")?;
            let (stderr_read, stderr_write) =
                rustix::pipe::pipe_with(PipeFlags::CLOEXEC).context("pipe")?;
            let nonce = safefs::nonce(8)?;
            let unit = units::unit_name(key, request.mode, &nonce);
            let unit_dir = self.unit_dir(&nonce, meta.gid)?;
            let descriptor = match request.descriptor_b64 {
                Some(text) => {
                    let mut file = safefs::create_new(unit_dir.as_fd(), DESCRIPTOR_FILE, 0o600)?;
                    file.write_all(&decode_base64(text)?)?;
                    file.sync_all()?;
                    Some(
                        self.config
                            .run_dir
                            .join(UNITS_DIR)
                            .join(&nonce)
                            .join(DESCRIPTOR_FILE),
                    )
                }
                None => None,
            };
            let exec_path = self
                .config
                .run_dir
                .join(UNITS_DIR)
                .join(&nonce)
                .join(EXEC_SOCKET);
            let listener = if request.mode == Mode::Runtime {
                Some(exec_listener(
                    &unit_dir,
                    &exec_path,
                    conn.peer_uid,
                    meta.gid,
                )?)
            } else {
                None
            };
            let paths = self.paths(key);
            let runtime_dir = self
                .config
                .state_root
                .join(store::STORE_DIR)
                .join(request.runtime_sha256);
            let spec = UnitSpec {
                key,
                mode: request.mode,
                nonce: &nonce,
                paths: &paths,
                runtime_dir: &runtime_dir,
                manifest: &manifest,
                exec_socket: listener.as_ref().map(|_| exec_path.as_path()),
                descriptor: descriptor.as_deref(),
                mask_var_run: self.mask_var_run,
            };
            let argv = units::systemd_run_argv(&self.config, &spec);

            match request.mode {
                Mode::Login => {
                    meta.credential.generation = request.credential_generation;
                    meta.credential.state = CredentialState::LoginRunning;
                }
                Mode::Logout => meta.credential.state = CredentialState::LogoutRunning,
                Mode::Runtime | Mode::Probe | Mode::Usage => {}
            }
            meta.units.push(UnitRecord {
                unit: unit.clone(),
                nonce: nonce.clone(),
                mode: request.mode,
                phase: UnitPhase::Starting,
                connection: conn.id.clone(),
                invocation_id: None,
                boot_id: self.boot_id.clone(),
                credential_generation: request.credential_generation,
                runtime_sha256: request.runtime_sha256.to_owned(),
                session_id: request.session_id.map(str::to_owned),
            });
            // R3-12: ownership is durable before systemd-run exists.
            Self::save(dir, meta)?;
            drop(store_guard);

            self.expect_exit(&unit);
            let stdio = UnitStdio {
                stdin: stdin_read,
                stdout: stdout_write,
                stderr: stderr_write,
            };
            let mut child = match self.host.spawn_unit(&argv, stdio) {
                Ok(child) => child,
                Err(error) => {
                    self.settle_failed(dir, meta, &unit, None);
                    return Err(error.into());
                }
            };
            let cancel_submission = child.canceller();
            {
                let daemon = Arc::clone(self);
                let (key, unit) = (key.to_owned(), unit.clone());
                std::thread::spawn(move || {
                    let exit = child.wait().ok().flatten();
                    daemon.unit_finished(&key, &unit, exit);
                });
            }
            let invocation_id = match self.await_invocation(&unit) {
                Ok(invocation) => invocation,
                Err(failure) => {
                    // The client may still submit the unit: end it first and
                    // stop the unit only once it is gone, so no submission
                    // can follow the confirmation. Until then the record
                    // stays `Starting` and is never confirmed stopped.
                    cancel_submission();
                    let timeout = Duration::from_secs(self.config.stop_timeout_secs);
                    let _ = self.wait_exit(&unit, timeout);
                    if self.exited(&unit) {
                        let _ = self.stop_and_confirm(dir, meta, &unit);
                    }
                    let _ = Self::save(dir, meta);
                    return Err(failure);
                }
            };
            if let Some(record) = meta.units.iter_mut().find(|record| record.unit == unit) {
                record.phase = UnitPhase::Running;
                record.invocation_id.clone_from(&invocation_id);
            }
            // `systemd-run` ending is not the unit ending (it may die after
            // submitting): settle only once systemd confirms the invocation
            // is gone; otherwise the connection owns it, so teardown stops it.
            let gone = self.exited(&unit)
                && meta
                    .units
                    .iter()
                    .find(|record| record.unit == unit)
                    .is_some_and(|record| matches!(self.invocation_live(record), Ok(false)));
            if gone {
                let exit = self.wait_exit(&unit, Duration::ZERO);
                if let Some(index) = meta.units.iter().position(|record| record.unit == unit) {
                    let record = meta.units.remove(index);
                    self.finish_unit(dir, meta, &record, exit);
                }
            } else {
                self.owned
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .entry(conn.id.clone())
                    .or_default()
                    .push((key.to_owned(), unit.clone()));
            }
            Self::save(dir, meta)?;
            let mut fds: Vec<OwnedFd> = vec![stdin_write, stdout_read, stderr_read];
            let mut names = vec!["stdin", "stdout", "stderr"];
            if let Some(listener) = listener {
                fds.push(listener);
                names.push("exec_listener");
            }
            Ok(Reply {
                value: json!({
                    "unit": unit,
                    "mode": request.mode,
                    "invocation_id": invocation_id,
                    "boot_id": self.boot_id,
                    "fds": names,
                }),
                fds,
            })
        })
    }

    /// Settle a unit that never ran.
    fn settle_failed(&self, dir: &OwnedFd, meta: &mut AccountMeta, unit: &str, exit: Option<i32>) {
        if let Some(index) = meta.units.iter().position(|record| record.unit == unit) {
            let record = meta.units.remove(index);
            self.finish_unit(dir, meta, &record, exit);
        }
        let _ = Self::save(dir, meta);
    }

    /// Wait for the unit's `InvocationID`. `None` when the unit already
    /// finished before it could be observed (its output stays readable).
    fn await_invocation(&self, unit: &str) -> Outcome<Option<String>> {
        let deadline = Instant::now() + INVOCATION_TIMEOUT;
        loop {
            if let Some(status) = self.host.unit_status(unit)?
                && status.invocation_id.is_some()
            {
                return Ok(status.invocation_id);
            }
            if self.exited(unit) {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err(fail(Code::Internal, format!("{unit} did not start")));
            }
            std::thread::sleep(POLL);
        }
    }

    /// `<run_dir>/units/<nonce>/`, root:`gid` 0710.
    fn unit_dir(&self, nonce: &str, gid: u32) -> Outcome<OwnedFd> {
        let units_dir = safefs::ensure_dir(self.run_fd(), UNITS_DIR, 0o711, root_owner())?;
        let owner = root_owner().map(|(uid, _)| (uid, gid));
        Ok(safefs::ensure_dir(units_dir.as_fd(), nonce, 0o710, owner)?)
    }
}

/// The unit's execution socket (R3-11): created and listened on by the
/// daemon, owned by the requesting worker and connectable only by the
/// account's group; the listening descriptor is handed to the worker.
fn exec_listener(
    unit_dir: &OwnedFd,
    path: &std::path::Path,
    owner: u32,
    gid: u32,
) -> Outcome<OwnedFd> {
    let socket = rustix::net::socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .context("socket")?;
    let address = SocketAddrUnix::new(path).context("socket path")?;
    rustix::net::bind(&socket, &address).context("bind execution socket")?;
    rustix::net::listen(&socket, 16).context("listen")?;
    if root_owner().is_some() {
        rustix::fs::chownat(
            unit_dir,
            EXEC_SOCKET,
            Some(Uid::from_raw(owner)),
            Some(Gid::from_raw(gid)),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .context("chown execution socket")?;
    }
    rustix::fs::chmodat(
        unit_dir,
        EXEC_SOCKET,
        FileMode::from_raw_mode(0o660),
        AtFlags::empty(),
    )
    .context("chmod execution socket")?;
    Ok(socket)
}

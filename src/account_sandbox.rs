//! Client of the root `cowboy-account-sandbox` daemon, protocol
//! `cowboy-account-sandbox-v1` (`packaging/account-sandbox/README.md`).
//!
//! A Provider whose signed manifest requires `account_sandbox_v1` never runs
//! as the Machine user: every runtime, login, logout and probe process is an
//! account unit the daemon starts as that account's own system identity. The
//! Machine registers the generation's runtime archive and drives login and
//! logout; the worker starts its runtime unit and relays ACP over the pipes
//! the daemon returns. Units are owned by the connection that started them:
//! the daemon stops and confirms them when it closes, so a [`Client`] must
//! stay open for as long as its units should run.
//!
//! The protocol is newline-delimited JSON with one response line per
//! request; the descriptors of a started unit arrive as `SCM_RIGHTS` on its
//! response line. Every read is bounded in size and time.

use std::io::{IoSliceMut, Read, Write as _};
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Linux receives descriptors close-on-exec atomically; macOS has no
/// MSG_CMSG_CLOEXEC, so `receive` sets the flag on each descriptor instead.
#[cfg(any(target_os = "linux", target_os = "android"))]
const RECV_FLAGS: RecvFlags = RecvFlags::CMSG_CLOEXEC;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const RECV_FLAGS: RecvFlags = RecvFlags::empty();

pub use crate::runtime_wire::AccountUnitHandle as UnitHandle;

pub const PROTOCOL: &str = "cowboy-account-sandbox-v1";
/// The control socket the OVH host configuration installs.
pub const DEFAULT_SOCKET: &str = "/run/cowboy-account-sandbox/control.sock";
/// Worker launch wiring: the JSON [`Launch`] for a sandboxed Provider.
pub const LAUNCH_ENV: &str = "COWBOY_ACCOUNT_SANDBOX_LAUNCH";
/// Bound of every request round trip except a runtime upload.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest accepted response line.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// Most descriptors one response may carry (stdio plus the exec listener).
const MAX_FDS: usize = 8;

/// What an account unit runs; the daemon fixes each mode's command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Runtime,
    Login,
    Logout,
    Probe,
    /// The account's quota report; started only by [`Client::collect_usage`].
    Usage,
}

/// A refusal the daemon reported, with its closed `code`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "account sandbox refused ({}): {}",
            self.code, self.message
        )
    }
}

impl std::error::Error for DaemonError {}

impl DaemonError {
    /// The daemon's code of `error`, when the daemon itself refused.
    #[must_use]
    pub fn code_of(error: &anyhow::Error) -> Option<&str> {
        error.downcast_ref::<Self>().map(|e| e.code.as_str())
    }
}

/// The exact sandboxed launch a Machine hands its worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub socket: PathBuf,
    pub service_id: String,
    pub account_id: String,
    /// The account's native credential generation the unit must run on.
    pub credential_generation: u64,
    pub runtime_sha256: String,
    /// The generation's runtime archive, registered again if the daemon
    /// evicted it from its bounded store.
    pub runtime_archive: PathBuf,
}

impl Launch {
    /// The launch the Machine placed in [`LAUNCH_ENV`], if any.
    pub fn from_env() -> Result<Option<Self>> {
        std::env::var(LAUNCH_ENV)
            .ok()
            .map(|value| serde_json::from_str(&value).context("invalid account sandbox launch"))
            .transpose()
    }
}

/// Worker wiring: the Machine-owned durable ledger file of a sandboxed
/// session with an execution binding. It keeps only the session's uncertain
/// effect across workers (design "Revision 5 simplification").
pub const EXECUTION_LEDGER_ENV: &str = "COWBOY_EXECUTION_LEDGER";
/// Directory of the ledger files under the Machine state root.
pub const EXECUTION_LEDGER_DIR: &str = "execution-ledgers";

/// The ledger file of `session_id` under the Machine state root.
#[must_use]
pub fn execution_ledger_file(state_dir: &Path, session_id: &str) -> Option<PathBuf> {
    (!session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte)))
    .then(|| {
        state_dir
            .join(EXECUTION_LEDGER_DIR)
            .join(format!("{session_id}.json"))
    })
}

/// The unit's path of its execution socket (the daemon binds it there).
pub const UNIT_EXEC_SOCKET: &str = "/run/cowboy-exec.sock";

/// How a sandboxed runtime unit reaches its worker's execution endpoint
/// (design amendment R3-11): the descriptor the daemon hands the unit as its
/// `descriptor` credential, and where the worker's endpoint takes the
/// listening `exec.sock` each started unit returns.
#[derive(Debug, Clone)]
pub struct ExecutionBridge {
    pub descriptor: Vec<u8>,
    pub listeners: tokio::sync::mpsc::UnboundedSender<OwnedFd>,
    /// The endpoint's relays still owed to the unit: its stop waits for
    /// them (bounded), so no reply is cut off by the unit's own teardown.
    pub relays: std::sync::Arc<RelayQuiet>,
}

/// Relays of one unit's execution endpoint whose reply has not yet been
/// written to the unit or recorded undelivered (design L1c).
#[derive(Debug, Default)]
pub struct RelayQuiet {
    count: std::sync::atomic::AtomicUsize,
    idle: tokio::sync::Notify,
    /// The unit is being stopped: only cleanup reads and stops relay.
    draining: std::sync::atomic::AtomicBool,
}

/// One relay in flight; dropped once its reply was handled.
#[derive(Debug)]
pub struct RelayGuard(std::sync::Arc<RelayQuiet>);

impl Drop for RelayGuard {
    fn drop(&mut self) {
        if self
            .0
            .count
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
            == 1
        {
            self.0.idle.notify_waiters();
        }
    }
}

impl RelayQuiet {
    /// Count one relay until the returned guard is dropped.
    #[must_use]
    pub fn enter(self: &std::sync::Arc<Self>) -> RelayGuard {
        self.count.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        RelayGuard(std::sync::Arc::clone(self))
    }

    /// The unit is being stopped: from now on only cleanup requests relay.
    pub fn begin_drain(&self) {
        self.draining
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether `method` may still relay: anything before the drain, then
    /// only reading a process and stopping one.
    #[must_use]
    pub fn admits(&self, method: &str) -> bool {
        !self.draining.load(std::sync::atomic::Ordering::Acquire)
            || matches!(method, "process/read" | "process/terminate")
    }

    /// Relays in flight now.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.count.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Wait until no relay is in flight, at most `bound`; `false` when the
    /// bound ended the wait.
    pub async fn drained(&self, bound: Duration) -> bool {
        tokio::time::timeout(bound, async {
            loop {
                let idle = self.idle.notified();
                tokio::pin!(idle);
                idle.as_mut().enable();
                if self.in_flight() == 0 {
                    return;
                }
                idle.await;
            }
        })
        .await
        .is_ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Account {
    pub key: String,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Registered {
    pub sha256: String,
    pub already_present: bool,
}

/// One `start` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest<'a> {
    pub key: &'a str,
    pub mode: Mode,
    pub credential_generation: u64,
    pub runtime_sha256: &'a str,
    pub session_id: Option<&'a str>,
    pub descriptor: Option<&'a [u8]>,
}

/// A started unit and the caller's ends of its pipes.
#[derive(Debug)]
pub struct Started {
    pub handle: UnitHandle,
    pub mode: Mode,
    /// Write end of the unit's stdin.
    pub stdin: OwnedFd,
    /// Read end of the unit's stdout.
    pub stdout: OwnedFd,
    /// Read end of the unit's stderr.
    pub stderr: OwnedFd,
    /// The listening execution socket of a `runtime` unit.
    pub exec_listener: Option<OwnedFd>,
}

/// The daemon's credential record of an account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Credential {
    pub generation: u64,
    pub state: String,
}

impl Credential {
    /// Whether `generation` is the account's current signed-in generation.
    #[must_use]
    pub fn signed_in(&self, generation: u64) -> bool {
        self.generation == generation && self.state == "signed_in"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UnitRecord {
    pub unit: String,
    pub mode: Mode,
    pub invocation_id: Option<String>,
    pub boot_id: String,
    pub credential_generation: u64,
    pub session_id: Option<String>,
    pub state: String,
    pub alive: bool,
    pub owned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AccountUnits {
    pub key: String,
    pub boot_id: String,
    pub fenced: Option<u64>,
    pub credential: Credential,
    pub units: Vec<UnitRecord>,
    pub unrecorded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UnitState {
    pub unit: String,
    pub recorded: bool,
    pub state: String,
    pub alive: bool,
    pub owned: bool,
    /// systemd runs this unit without a daemon record (stop it).
    #[serde(default)]
    pub requires_reconciliation: bool,
    #[serde(default)]
    pub invocation_id: Option<String>,
    #[serde(default)]
    pub boot_id: Option<String>,
    pub current_boot_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AccountFence {
    pub key: String,
    pub fence_epoch: u64,
    /// Whether the account's fence is durably set after the operation.
    pub fenced: bool,
    /// Units `stop_account` stopped and confirmed gone.
    #[serde(default)]
    pub stopped: Vec<String>,
    /// The operation repeated the applied disposition.
    #[serde(default)]
    pub replayed: bool,
    /// `false` when the fence is held by a tombstone of an account the
    /// daemon has not created; `ensure_account` adopts it.
    #[serde(default = "present")]
    pub account_present: bool,
}

fn present() -> bool {
    true
}

/// The Controller's ordering of one fence operation: the account control's
/// revision and the operation's idempotent id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FenceOrder<'a> {
    pub revision: u64,
    pub operation_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LogoutCleanup {
    pub key: String,
    pub credential_generation: u64,
    pub token_removed: bool,
    pub credential_cleared: bool,
    pub history_retained: bool,
    /// A newer credential generation exists; nothing was removed.
    #[serde(default)]
    pub superseded: bool,
}

/// The daemon's receipt of `remove_account`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Removed {
    pub key: String,
    pub removed: bool,
    #[serde(default)]
    pub home_removed: bool,
    #[serde(default)]
    pub user_removed: bool,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub replayed: bool,
}

/// The daemon's probe of a registered runtime in its probe identity's unit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RuntimeProbe {
    pub runtime_sha256: String,
    pub unit: String,
    pub exit_code: Option<i32>,
    pub passed: bool,
}

/// The daemon's validated quota report of an account (`collect_usage`):
/// `outcome` is `available` (with `quota`, the daemon's projection),
/// `authentication_required` or `location_unsupported`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UsageReport {
    pub unit: String,
    pub outcome: String,
    #[serde(default)]
    pub quota: Option<Value>,
}

/// One sequential connection to the daemon.
#[derive(Debug)]
pub struct Client {
    stream: UnixStream,
    /// Bytes received after the last response line.
    pending: Vec<u8>,
}

impl Client {
    /// Connect to the daemon's control socket.
    pub fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket).with_context(|| {
            format!(
                "the account sandbox daemon is unreachable at {}",
                socket.display()
            )
        })?;
        stream.set_read_timeout(Some(REQUEST_TIMEOUT))?;
        stream.set_write_timeout(Some(REQUEST_TIMEOUT))?;
        Ok(Self {
            stream,
            pending: Vec::new(),
        })
    }

    /// Create (or find) the account's static identity and tree.
    pub fn ensure_account(&mut self, service_id: &str, account_id: &str) -> Result<Account> {
        let request =
            json!({"op": "ensure_account", "service_id": service_id, "account_id": account_id});
        let account: Account = self.call(&request)?;
        ensure!(
            account.key == account_key(service_id, account_id),
            "the account sandbox derived another account key"
        );
        Ok(account)
    }

    /// Upload a runtime archive of `size` bytes whose SHA-256 is `sha256`.
    /// A refused upload closes the daemon's side of the connection.
    pub fn register_runtime(
        &mut self,
        archive: &mut impl Read,
        size: u64,
        sha256: &str,
    ) -> Result<Registered> {
        let request = json!({"op": "register_runtime", "sha256": sha256, "size": size});
        self.send(&request)?;
        let copied = std::io::copy(&mut archive.take(size), &mut self.stream)
            .context("uploading the runtime archive")?;
        ensure!(copied == size, "runtime archive is shorter than declared");
        let (value, fds) = self.receive()?;
        ensure!(fds.is_empty(), "unexpected descriptors on a registration");
        Ok(serde_json::from_value(value)?)
    }

    /// Run the registered runtime's probe command in a confined unit of the
    /// daemon's own probe identity (never an account).
    pub fn probe_runtime(&mut self, runtime_sha256: &str) -> Result<RuntimeProbe> {
        let probe: RuntimeProbe =
            self.call(&json!({"op": "probe_runtime", "runtime_sha256": runtime_sha256}))?;
        ensure!(
            probe.runtime_sha256 == runtime_sha256,
            "the account sandbox probed another runtime"
        );
        Ok(probe)
    }

    /// Run the account's read-only quota report on its current signed-in
    /// `credential_generation`; the daemon answers only its projection.
    pub fn collect_usage(
        &mut self,
        key: &str,
        credential_generation: u64,
        runtime_sha256: &str,
    ) -> Result<UsageReport> {
        let report: UsageReport = self.call(&json!({
            "op": "collect_usage",
            "key": key,
            "credential_generation": credential_generation,
            "runtime_sha256": runtime_sha256,
        }))?;
        ensure!(
            report.unit.starts_with(&format!("cga-{key}-usage-")),
            "the account sandbox reported another account's unit"
        );
        ensure!(
            matches!(
                report.outcome.as_str(),
                "available" | "authentication_required" | "location_unsupported"
            ) && (report.outcome == "available") == report.quota.is_some(),
            "malformed account sandbox usage report"
        );
        Ok(report)
    }

    /// Start one account unit and receive its pipes.
    pub fn start(&mut self, request: &StartRequest<'_>) -> Result<Started> {
        let mut line = json!({
            "op": "start",
            "key": request.key,
            "mode": request.mode,
            "credential_generation": request.credential_generation,
            "runtime_sha256": request.runtime_sha256,
        });
        if let Some(session_id) = request.session_id {
            line["session_id"] = json!(session_id);
        }
        if let Some(descriptor) = request.descriptor {
            use base64::Engine as _;
            line["descriptor_b64"] =
                json!(base64::engine::general_purpose::STANDARD.encode(descriptor));
        }
        self.send(&line)?;
        let (value, fds) = self.receive()?;
        started(value, fds)
    }

    /// Stop `unit` and wait until the daemon confirms it is gone.
    pub fn stop_unit(&mut self, unit: &str) -> Result<bool> {
        let value: Value = self.call(&json!({"op": "stop_unit", "unit": unit}))?;
        Ok(value.get("stopped").and_then(Value::as_bool) == Some(true))
    }

    /// Fence the account and stop every unit of it. A success is durable
    /// fence evidence, also for an account the daemon has not created.
    pub fn stop_account(
        &mut self,
        key: &str,
        order: Option<FenceOrder<'_>>,
    ) -> Result<AccountFence> {
        let fence: AccountFence = self.call(&fence_request("stop_account", key, order))?;
        ensure!(
            fence.fenced,
            "the account sandbox did not report the fence set"
        );
        Ok(fence)
    }

    /// Clear the account's fence; an older `order` than the applied one is
    /// refused (`conflict`).
    pub fn admit_account(
        &mut self,
        key: &str,
        order: Option<FenceOrder<'_>>,
    ) -> Result<AccountFence> {
        let fence: AccountFence = self.call(&fence_request("admit_account", key, order))?;
        ensure!(!fence.fenced, "the account sandbox kept the fence set");
        Ok(fence)
    }

    pub fn list_units(&mut self, key: &str) -> Result<AccountUnits> {
        self.call(&json!({"op": "list_units", "key": key}))
    }

    pub fn unit_state(&mut self, unit: &str) -> Result<UnitState> {
        self.call(&json!({"op": "unit_state", "unit": unit}))
    }

    /// Remove only the vendor token of credential `generation` after its
    /// logout unit; history stays. The daemon checks the generation under
    /// the same lock as the removal, so a newer credential is never cleared.
    pub fn logout_cleanup(
        &mut self,
        key: &str,
        generation: u64,
        operation_id: Option<&str>,
    ) -> Result<LogoutCleanup> {
        let mut request =
            json!({"op": "logout_cleanup", "key": key, "credential_generation": generation});
        if let Some(operation_id) = operation_id {
            request["operation_id"] = json!(operation_id);
        }
        let cleanup: LogoutCleanup = self.call(&request)?;
        ensure!(
            cleanup.credential_generation == generation,
            "the account sandbox cleaned another credential generation"
        );
        Ok(cleanup)
    }

    /// Remove the account's persistent home and its system user under the
    /// caller's idempotent `operation_id`. The daemon requires the account's
    /// fence and a logged-out credential; an account it has no metadata for
    /// is confirmed removed.
    pub fn remove_account(&mut self, key: &str, operation_id: &str) -> Result<Removed> {
        let removed: Removed =
            self.call(&json!({"op": "remove_account", "key": key, "operation_id": operation_id}))?;
        ensure!(
            removed.key == key && removed.operation_id.as_deref() == Some(operation_id),
            "the account sandbox removed another account or operation"
        );
        Ok(removed)
    }

    fn call<T: serde::de::DeserializeOwned>(&mut self, request: &Value) -> Result<T> {
        self.send(request)?;
        let (value, fds) = self.receive()?;
        ensure!(fds.is_empty(), "unexpected descriptors on a response");
        serde_json::from_value(value).context("malformed account sandbox response")
    }

    fn send(&mut self, request: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(request)?;
        line.push(b'\n');
        self.stream
            .write_all(&line)
            .context("writing to the account sandbox daemon")
    }

    /// One response line and the descriptors attached to it.
    fn receive(&mut self) -> Result<(Value, Vec<OwnedFd>)> {
        let mut fds = Vec::new();
        loop {
            if let Some(end) = self.pending.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                return Ok((response(&line[..end])?, fds));
            }
            ensure!(
                self.pending.len() <= MAX_RESPONSE_BYTES,
                "account sandbox response exceeds {MAX_RESPONSE_BYTES} bytes"
            );
            let mut buffer = [0_u8; 8192];
            let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(MAX_FDS))];
            let mut control = RecvAncillaryBuffer::new(&mut space);
            let received = rustix::net::recvmsg(
                &self.stream,
                &mut [IoSliceMut::new(&mut buffer)],
                &mut control,
                RECV_FLAGS,
            )
            .map_err(|error| match error {
                rustix::io::Errno::AGAIN => {
                    anyhow::anyhow!("the account sandbox daemon did not answer in time")
                }
                error => anyhow::Error::from(error).context("reading from the account sandbox"),
            })?;
            for message in control.drain() {
                if let RecvAncillaryMessage::ScmRights(received) = message {
                    for fd in received {
                        // Without MSG_CMSG_CLOEXEC (macOS) close-on-exec is
                        // set right after receipt instead of atomically.
                        #[cfg(not(any(target_os = "linux", target_os = "android")))]
                        rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)
                            .context("marking a received descriptor close-on-exec")?;
                        fds.push(fd);
                    }
                }
            }
            ensure!(
                !received.flags.contains(ReturnFlags::CTRUNC),
                "account sandbox descriptors were truncated"
            );
            ensure!(fds.len() <= MAX_FDS, "too many descriptors received");
            if received.bytes == 0 {
                bail!("the account sandbox daemon closed the connection");
            }
            self.pending.extend_from_slice(&buffer[..received.bytes]);
        }
    }
}

/// Decode one response line into its result or a [`DaemonError`].
fn response(line: &[u8]) -> Result<Value> {
    #[derive(Deserialize)]
    struct Envelope {
        ok: bool,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<ErrorBody>,
    }
    #[derive(Deserialize)]
    struct ErrorBody {
        code: String,
        message: String,
    }
    let envelope: Envelope =
        serde_json::from_slice(line).context("malformed account sandbox response")?;
    if envelope.ok {
        return envelope
            .result
            .context("account sandbox response has no result");
    }
    let error = envelope
        .error
        .context("account sandbox refusal has no error")?;
    Err(DaemonError {
        code: error.code,
        message: error.message,
    }
    .into())
}

/// Bind the received descriptors to the names the response lists.
fn started(value: Value, fds: Vec<OwnedFd>) -> Result<Started> {
    #[derive(Deserialize)]
    struct Reply {
        unit: String,
        mode: Mode,
        invocation_id: Option<String>,
        boot_id: String,
        fds: Vec<String>,
    }
    let reply: Reply = serde_json::from_value(value).context("malformed start response")?;
    ensure!(
        reply.fds.len() == fds.len(),
        "start response names {} descriptors but carried {}",
        reply.fds.len(),
        fds.len()
    );
    let mut named: std::collections::HashMap<String, OwnedFd> =
        reply.fds.into_iter().zip(fds).collect();
    let mut take = |name: &str| {
        named
            .remove(name)
            .with_context(|| format!("start response has no {name} descriptor"))
    };
    let (stdin, stdout, stderr) = (take("stdin")?, take("stdout")?, take("stderr")?);
    let exec_listener = take("exec_listener").ok();
    ensure!(
        exec_listener.is_some() == (reply.mode == Mode::Runtime),
        "only a runtime unit carries an execution listener"
    );
    Ok(Started {
        handle: UnitHandle {
            unit: reply.unit,
            invocation_id: reply.invocation_id,
            boot_id: reply.boot_id,
        },
        mode: reply.mode,
        stdin,
        stdout,
        stderr,
        exec_listener,
    })
}

/// The daemon's account key: `hex(sha256("cowboy-account-v1" || 0x00 ||
/// service_id || 0x00 || account_id))[..16]`. Evidence queries use it
/// without `ensure_account`, which would create a missing account.
#[must_use]
fn fence_request(op: &str, key: &str, order: Option<FenceOrder<'_>>) -> Value {
    let mut request = json!({"op": op, "key": key});
    if let Some(order) = order {
        request["revision"] = json!(order.revision);
        request["operation_id"] = json!(order.operation_id);
    }
    request
}

pub fn account_key(service_id: &str, account_id: &str) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(b"cowboy-account-v1");
    hasher.update([0]);
    hasher.update(service_id.as_bytes());
    hasher.update([0]);
    hasher.update(account_id.as_bytes());
    let mut key = format!("{:x}", hasher.finalize());
    key.truncate(16);
    key
}

/// Start a unit; a runtime the daemon evicted from its bounded store is
/// registered again from `archive` and the start retried once.
pub fn start_registering(
    client: &mut Client,
    request: &StartRequest<'_>,
    archive: &Path,
) -> Result<Started> {
    match client.start(request) {
        Err(error) if DaemonError::code_of(&error) == Some("not_found") => {
            register_file(client, archive, request.runtime_sha256)?;
            client.start(request)
        }
        result => result,
    }
}

/// Register the runtime archive stored at `path`.
pub fn register_file(client: &mut Client, path: &Path, sha256: &str) -> Result<Registered> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("opening runtime archive {}", path.display()))?;
    let size = file.metadata()?.len();
    let registered = client.register_runtime(&mut file, size, sha256)?;
    ensure!(
        registered.sha256 == sha256,
        "the account sandbox registered another runtime"
    );
    Ok(registered)
}

#[cfg(test)]
#[path = "account_sandbox_tests.rs"]
mod tests;

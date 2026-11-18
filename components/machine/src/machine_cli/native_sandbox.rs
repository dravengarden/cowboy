//! Native login and logout of an `account_sandbox_v1` Provider. The daemon
//! runs the signed launcher's fixed `--cowboy-login` / `--cowboy-logout`
//! command in the account's own unit; the Machine relays the same I/O as for
//! a local authentication process and takes every outcome from the daemon:
//! a login succeeds exactly when the daemon records the reserved credential
//! generation `signed_in`, and a logout clears only the vendor token while
//! the account's history stays in its persistent home.
use super::*;
use crate::account_sandbox as daemon;
use std::os::unix::process::ExitStatusExt as _;
use std::process::ExitStatus;

/// Bound for a stopped or finished unit to disappear from systemd.
#[cfg(not(test))]
const UNIT_END_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const UNIT_END_TIMEOUT: Duration = Duration::from_millis(600);
const UNIT_END_POLL: Duration = Duration::from_millis(250);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(120);

/// Why a start produced no unit handle: a definitive daemon refusal (no
/// unit runs), or an unknown outcome the daemon must reconcile.
struct StartFailure {
    refused: bool,
    detail: String,
}

/// Whether `error` is the daemon's definitive refusal. Transport failures,
/// lost replies and `internal` failures leave the outcome unknown.
fn refused(error: &anyhow::Error) -> bool {
    daemon::DaemonError::code_of(error).is_some_and(|code| code != "internal")
}

/// A started authentication unit and the connection that owns it.
struct Unit {
    connection: crate::machine_plugins::AccountConnection,
    handle: daemon::UnitHandle,
}

/// Start `mode` on the account's reserved credential `generation`.
async fn start(
    providers: &MachinePluginStore,
    provider: &str,
    digest: &str,
    mode: daemon::Mode,
    generation: u64,
) -> Result<(Unit, daemon::Started), StartFailure> {
    let (providers, provider, digest) = (providers.clone(), provider.to_owned(), digest.to_owned());
    tokio::task::spawn_blocking(move || {
        let mut connection = providers.account_sandbox_connection(&provider, &digest)?;
        let started = daemon::start_registering(
            &mut connection.client,
            &daemon::StartRequest {
                key: &connection.key,
                mode,
                credential_generation: generation,
                runtime_sha256: &connection.runtime_sha256,
                session_id: None,
                descriptor: None,
            },
            &connection.runtime_archive,
        )?;
        let handle = started.handle.clone();
        anyhow::Ok((Unit { connection, handle }, started))
    })
    .await
    .map_err(|_| StartFailure {
        refused: false,
        detail: "account sandbox start task failed".to_owned(),
    })?
    .map_err(|error| StartFailure {
        refused: refused(&error),
        detail: format!("the account sandbox did not start the authentication unit: {error:#}"),
    })
}

/// Reconcile the exact credential `generation` with the daemon after an
/// unknown start or end of its `mode` unit: wait (bounded, retrying transport
/// failures) until the daemon finalized every such unit, then return its
/// record.
/// `None` when the daemon could not confirm it in time.
async fn settle(
    providers: &MachinePluginStore,
    mode: daemon::Mode,
    generation: u64,
) -> Option<daemon::AccountUnits> {
    let providers = providers.clone();
    tokio::task::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + UNIT_END_TIMEOUT;
        loop {
            match providers.account_sandbox_units() {
                // Finalized: the daemon settled every unit record of the
                // generation (its exit observed and the credential state
                // committed), not merely systemd reporting it gone.
                Ok(units) if finalized(&units, mode, generation) => return Some(units),
                Ok(_) => {}
                Err(error) if refused(&error) => return None,
                Err(_) => {}
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(UNIT_END_POLL);
        }
    })
    .await
    .ok()
    .flatten()
}

/// Whether the daemon finalized every `mode` unit of `generation`: no record
/// of one remains and the credential is not mid-transition for it.
fn finalized(units: &daemon::AccountUnits, mode: daemon::Mode, generation: u64) -> bool {
    let recorded = units
        .units
        .iter()
        .any(|unit| unit.mode == mode && unit.credential_generation == generation);
    let running = units.credential.generation == generation
        && matches!(
            units.credential.state.as_str(),
            "login_running" | "logout_running"
        );
    !recorded && !running
}

fn login_status(signed_in: bool) -> ExitStatus {
    ExitStatus::from_raw(if signed_in { 0 } else { 1 << 8 })
}

fn pipes(
    started: daemon::Started,
) -> std::io::Result<(
    tokio::net::unix::pipe::Sender,
    tokio::net::unix::pipe::Receiver,
    tokio::net::unix::pipe::Receiver,
)> {
    Ok((
        tokio::net::unix::pipe::Sender::from_owned_fd(started.stdin)?,
        tokio::net::unix::pipe::Receiver::from_owned_fd(started.stdout)?,
        tokio::net::unix::pipe::Receiver::from_owned_fd(started.stderr)?,
    ))
}

impl Unit {
    /// Stop the unit when `stop`, then wait until the daemon confirms it is
    /// gone. Returns the connection for the outcome queries.
    async fn end(self, stop: bool) -> Result<crate::machine_plugins::AccountConnection, String> {
        let Self {
            mut connection,
            handle,
        } = self;
        tokio::task::spawn_blocking(move || {
            if stop {
                connection.client.stop_unit(&handle.unit)?;
            }
            let deadline = std::time::Instant::now() + UNIT_END_TIMEOUT;
            while connection.client.unit_state(&handle.unit)?.alive {
                anyhow::ensure!(
                    std::time::Instant::now() < deadline,
                    "{} did not end",
                    handle.unit
                );
                std::thread::sleep(UNIT_END_POLL);
            }
            anyhow::Ok(connection)
        })
        .await
        .map_err(|_| "account sandbox unit task failed".to_owned())?
        .map_err(|error| format!("account sandbox unit end unconfirmed: {error:#}"))
    }
}

/// Run the login unit with the caller's challenge relay. The status is a
/// success exactly when the unit ended and the daemon records `generation`
/// as the account's signed-in credential; an unconfirmed end is `Cleanup`.
pub(super) async fn login(
    providers: &MachinePluginStore,
    provider: &str,
    digest: &str,
    (generation, request): (u64, &str),
    login: &mut LoginIo,
    deadline: tokio::time::Instant,
    on_line: impl FnMut(String),
) -> Result<Result<ExitStatus, login_process::Failure>, String> {
    let (unit, started) = match start(providers, provider, digest, daemon::Mode::Login, generation)
        .await
    {
        Ok(started) => started,
        Err(StartFailure {
            refused: true,
            detail,
        }) => {
            // A refused start started no unit. Record the failed exit so
            // the reservation can be retired.
            providers
                .record_native_login_tree_exit(provider, generation, request, false)
                .map_err(|_| "native login exit persistence failed".to_owned())?;
            return Err(detail);
        }
        Err(StartFailure { detail, .. }) => {
            // The daemon may have started the unit before the reply was
            // lost; its connection closed, so the daemon stops it. Only
            // the daemon's record of this exact generation decides.
            tracing::warn!(provider, generation, %detail, "account sandbox login start unknown");
            return Ok(
                match settle(providers, daemon::Mode::Login, generation).await {
                    Some(units) => Ok(login_status(units.credential.signed_in(generation))),
                    None => Err(login_process::Failure::Cleanup),
                },
            );
        }
    };
    let (stdin, stdout, stderr) =
        pipes(started).map_err(|_| "account sandbox login pipes are unusable".to_owned())?;
    let io = login_process::drive_pipes(
        stdin,
        stdout,
        stderr,
        &mut login.cancel,
        &mut login.input,
        deadline,
        on_line,
    )
    .await;
    let Ok(connection) = unit.end(io.is_err()).await else {
        return Ok(Err(login_process::Failure::Cleanup));
    };
    if let Err(failure) = io {
        return Ok(Err(failure));
    }
    drop(connection);
    // The daemon records `signed_in` only after the unit exited 0 with its
    // token present; any other confirmed end is a failed login of this
    // generation. A query the daemon could not answer stays unconfirmed.
    Ok(
        match settle(providers, daemon::Mode::Login, generation).await {
            Some(units) => Ok(login_status(units.credential.signed_in(generation))),
            None => Err(login_process::Failure::Cleanup),
        },
    )
}

/// The outcome of a sandboxed logout: the unit's confirmed end and the
/// daemon's token cleanup receipt.
pub(super) struct Logout {
    pub succeeded: bool,
    pub receipt: Option<daemon::LogoutCleanup>,
}

/// Run the logout unit, then let the daemon remove only the vendor token of
/// `generation`. A refused or unknown start is reconciled with the daemon's
/// record ([`reconcile_logout`]): a generation holding no signed-in token is
/// cleaned up, a signed-in one stays pending (`Err`), and only the daemon's
/// refusal of the cleanup itself is a failed logout.
pub(super) async fn logout(
    providers: &MachinePluginStore,
    provider: &str,
    digest: &str,
    (generation, operation): (u64, &str),
) -> Result<Logout, String> {
    let (unit, started) = match start(
        providers,
        provider,
        digest,
        daemon::Mode::Logout,
        generation,
    )
    .await
    {
        Ok(started) => started,
        Err(StartFailure {
            refused: true,
            detail,
        }) => {
            // The daemon refuses a logout unit of a generation that holds no
            // signed-in credential (a failed first login, or one already
            // superseded): its token cleanup still settles the logout, so a
            // revocation can finish. A signed-in generation stays pending.
            tracing::warn!(provider, generation, %detail, "account sandbox logout refused; reconciling");
            return reconcile_logout(providers, provider, digest, (generation, operation), false)
                .await;
        }
        Err(StartFailure { detail, .. }) => {
            tracing::warn!(provider, generation, %detail, "account sandbox logout start unknown");
            return reconcile_logout(providers, provider, digest, (generation, operation), false)
                .await;
        }
    };
    let pipes = pipes(started);
    let io = match pipes {
        Ok((stdin, stdout, stderr)) => {
            let (_cancel_owner, mut cancel) = tokio::sync::watch::channel(false);
            let (_input_owner, mut input) = tokio::sync::mpsc::channel(1);
            login_process::drive_pipes(
                stdin,
                stdout,
                stderr,
                &mut cancel,
                &mut input,
                tokio::time::Instant::now() + LOGOUT_TIMEOUT,
                |_| {},
            )
            .await
        }
        Err(_) => Err(login_process::Failure::Read),
    };
    let connection = unit.end(io.is_err()).await?;
    drop(connection);
    Ok(cleaned(cleanup(providers, generation, operation).await?))
}

/// Settle a logout of `generation` whose outcome is unknown against the
/// daemon's record: once no logout unit of it runs, a cleared or superseded
/// credential is a completed logout, a finished logout unit is completed by
/// its cleanup, and a credential still signed in reruns the same logout when
/// `rerun` (a retry of the operation) or stays pending. An unanswered daemon
/// keeps it pending (`Err`); nothing is retired.
pub(super) async fn reconcile_logout(
    providers: &MachinePluginStore,
    provider: &str,
    digest: &str,
    (generation, operation): (u64, &str),
    rerun: bool,
) -> Result<Logout, String> {
    let units = settle(providers, daemon::Mode::Logout, generation)
        .await
        .ok_or_else(|| "account sandbox logout outcome unconfirmed; it stays pending".to_owned())?;
    let credential = &units.credential;
    if credential.generation == generation && credential.state == "signed_in" {
        // The logout unit never ran: run it again under the same operation.
        if !rerun {
            return Err("account sandbox logout did not start; it stays pending".to_owned());
        }
        return Box::pin(logout(providers, provider, digest, (generation, operation))).await;
    }
    Ok(cleaned(cleanup(providers, generation, operation).await?))
}

fn cleaned(receipt: Option<daemon::LogoutCleanup>) -> Logout {
    Logout {
        succeeded: receipt
            .as_ref()
            .is_some_and(|receipt| receipt.credential_cleared),
        receipt,
    }
}

/// The daemon's recorded token cleanup of a finished logout `operation`
/// (its replay); an unknown outcome is `Err`.
pub(super) async fn replay_cleanup(
    providers: &MachinePluginStore,
    generation: u64,
    operation: &str,
) -> Result<Option<daemon::LogoutCleanup>, String> {
    cleanup(providers, generation, operation).await
}

/// The daemon's token cleanup for `generation`. The daemon verifies the
/// generation under the lock it removes the token with, so a later sign-in is
/// never cleared (a superseded generation removes nothing). `None` is a
/// definitive refusal; an unknown outcome is `Err`.
async fn cleanup(
    providers: &MachinePluginStore,
    generation: u64,
    operation: &str,
) -> Result<Option<daemon::LogoutCleanup>, String> {
    let (providers, operation) = (providers.clone(), operation.to_owned());
    tokio::task::spawn_blocking(move || {
        let (socket, key) = providers.account_sandbox_address()?;
        daemon::Client::connect(&socket)?.logout_cleanup(&key, generation, Some(&operation))
    })
    .await
    .map_err(|_| "account sandbox cleanup task failed".to_owned())?
    .map(Some)
    .or_else(|error| {
        if refused(&error) {
            tracing::warn!(generation, error = %format!("{error:#}"), "account sandbox refused logout cleanup");
            Ok(None)
        } else {
            Err(format!("account sandbox logout cleanup unconfirmed: {error:#}"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, Write as _};

    /// A fake daemon answering one request per connection with `replies`;
    /// it returns the requests it received.
    fn daemon(
        socket: &Path,
        replies: Vec<serde_json::Value>,
    ) -> std::thread::JoinHandle<Vec<serde_json::Value>> {
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                std::io::BufReader::new(&stream)
                    .read_line(&mut line)
                    .unwrap();
                requests.push(serde_json::from_str(&line).unwrap());
                writeln!(stream, "{reply}").unwrap();
            }
            requests
        })
    }

    fn units(generation: u64, state: &str) -> serde_json::Value {
        serde_json::json!({"ok": true, "result": {
            "key": "k", "boot_id": "b", "fenced": null,
            "credential": {"generation": generation, "state": state},
            "units": [], "unrecorded": [],
        }})
    }

    fn store(root: &Path, socket: &Path) -> MachinePluginStore {
        let account: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let store = MachinePluginStore::new(
            root,
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
        .for_account(&account)
        .unwrap();
        store.set_account_sandbox(socket.to_owned(), Some("service".into()));
        store
    }

    #[test]
    fn only_typed_daemon_refusals_are_definitive() {
        let refusal = |code: &str| {
            anyhow::Error::new(daemon::DaemonError {
                code: code.into(),
                message: String::new(),
            })
        };
        assert!(refused(&refusal("precondition")));
        assert!(refused(&refusal("fenced")));
        assert!(!refused(&refusal("internal")));
        assert!(!refused(&anyhow::anyhow!("connection reset")));
    }

    #[test]
    fn an_outcome_waits_for_the_daemon_to_finalize_the_unit() {
        let record = |state: &str, units: serde_json::Value| -> daemon::AccountUnits {
            serde_json::from_value(serde_json::json!({
                "key": "k", "boot_id": "b", "fenced": null,
                "credential": {"generation": 4, "state": state},
                "units": units, "unrecorded": [],
            }))
            .unwrap()
        };
        let dead = serde_json::json!([{"unit": "cga-k-login-1.service", "mode": "login",
            "invocation_id": "i", "boot_id": "b", "credential_generation": 4,
            "session_id": null, "state": "gone", "alive": false, "owned": true}]);
        // systemd reports the unit gone, but its record is not settled yet.
        assert!(!finalized(
            &record("login_running", dead),
            daemon::Mode::Login,
            4
        ));
        assert!(!finalized(
            &record("login_running", serde_json::json!([])),
            daemon::Mode::Login,
            4
        ));
        assert!(finalized(
            &record("signed_in", serde_json::json!([])),
            daemon::Mode::Login,
            4
        ));
    }

    #[tokio::test]
    async fn a_pending_logout_is_completed_from_the_daemon_record() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let server = daemon(
            &socket,
            vec![
                units(3, "logout_pending"),
                serde_json::json!({"ok": true, "result": {
                    "key": "k", "credential_generation": 3, "token_removed": true,
                    "credential_cleared": true, "history_retained": true,
                }}),
            ],
        );
        let store = store(root.path(), &socket);
        let logout = reconcile_logout(&store, "antigravity", "d", (3, "op-1"), false)
            .await
            .unwrap();
        assert!(logout.succeeded);
        let requests = server.join().unwrap();
        assert_eq!(requests[1]["op"], "logout_cleanup");
        assert_eq!(requests[1]["credential_generation"], 3);
        assert_eq!(requests[1]["operation_id"], "op-1");
    }

    #[tokio::test]
    async fn an_unstarted_or_unanswered_logout_stays_pending() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let server = daemon(&socket, vec![units(3, "signed_in")]);
        let store = store(root.path(), &socket);
        // The logout unit never ran; only a retry of the operation reruns it.
        let pending = reconcile_logout(&store, "antigravity", "d", (3, "op"), false).await;
        assert!(pending.is_err());
        server.join().unwrap();
        // An unreachable daemon is never a failed logout.
        let gone = reconcile_logout(&store, "antigravity", "d", (3, "op"), true).await;
        assert!(gone.err().unwrap().contains("unconfirmed"));
    }

    #[tokio::test]
    async fn a_login_outcome_is_the_daemon_record_of_its_generation() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let server = daemon(&socket, vec![units(4, "signed_in"), units(4, "signed_in")]);
        let store = store(root.path(), &socket);
        let signed_in = settle(&store, daemon::Mode::Login, 4).await.unwrap();
        assert!(signed_in.credential.signed_in(4));
        let older = settle(&store, daemon::Mode::Login, 3).await.unwrap();
        assert!(!older.credential.signed_in(3));
        server.join().unwrap();
        // A daemon that cannot answer leaves the login unconfirmed.
        assert!(settle(&store, daemon::Mode::Login, 4).await.is_none());
    }
}

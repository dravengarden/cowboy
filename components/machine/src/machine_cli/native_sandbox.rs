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
const UNIT_END_TIMEOUT: Duration = Duration::from_secs(60);
const UNIT_END_POLL: Duration = Duration::from_millis(250);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(120);

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
) -> Result<(Unit, daemon::Started), String> {
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
    .map_err(|_| "account sandbox start task failed".to_owned())?
    .map_err(|error| format!("the account sandbox refused the authentication unit: {error:#}"))
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
    let (unit, started) =
        match start(providers, provider, digest, daemon::Mode::Login, generation).await {
            Ok(started) => started,
            Err(detail) => {
                // No unit runs: a refused start started none, and a lost reply
                // closed its connection, whose units the daemon stops. Record
                // the failed exit so the reservation can be retired.
                providers
                    .record_native_login_tree_exit(provider, generation, request, false)
                    .map_err(|_| "native login exit persistence failed".to_owned())?;
                return Err(detail);
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
    let Ok(mut connection) = unit.end(io.is_err()).await else {
        return Ok(Err(login_process::Failure::Cleanup));
    };
    if let Err(failure) = io {
        return Ok(Err(failure));
    }
    let signed_in = tokio::task::spawn_blocking(move || {
        let key = connection.key.clone();
        connection.client.list_units(&key)
    })
    .await
    .is_ok_and(|units| units.is_ok_and(|units| units.credential.signed_in(generation)));
    // The daemon records `signed_in` only after the unit exited 0 with its
    // token present; any other end is a failed login of this generation.
    Ok(Ok(ExitStatus::from_raw(if signed_in { 0 } else { 1 << 8 })))
}

/// The outcome of a sandboxed logout: the unit's confirmed end and the
/// daemon's token cleanup receipt.
pub(super) struct Logout {
    pub succeeded: bool,
    pub receipt: Option<daemon::LogoutCleanup>,
}

/// Run the logout unit, then let the daemon remove only the vendor token.
/// A refused start and a failed cleanup are failed logouts of a unit that is
/// not running; only an unconfirmed unit end is an error.
pub(super) async fn logout(
    providers: &MachinePluginStore,
    provider: &str,
    digest: &str,
    generation: u64,
) -> Result<Logout, String> {
    let failed = |detail: String| {
        tracing::warn!(provider, generation, %detail, "account sandbox logout failed");
        Logout {
            succeeded: false,
            receipt: None,
        }
    };
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
        Err(detail) => return Ok(failed(detail)),
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
    Ok(match cleanup(connection, generation).await {
        Ok(receipt) => Logout {
            succeeded: receipt
                .as_ref()
                .is_some_and(|receipt| receipt.credential_cleared),
            receipt,
        },
        Err(detail) => failed(detail),
    })
}

/// The daemon's token cleanup for `generation`, only while it is still the
/// account's current credential: a later sign-in is never cleared.
async fn cleanup(
    mut connection: crate::machine_plugins::AccountConnection,
    generation: u64,
) -> Result<Option<daemon::LogoutCleanup>, String> {
    tokio::task::spawn_blocking(move || {
        let key = connection.key.clone();
        if connection.client.list_units(&key)?.credential.generation != generation {
            return anyhow::Ok(None);
        }
        connection.client.logout_cleanup(&key).map(Some)
    })
    .await
    .map_err(|_| "account sandbox cleanup task failed".to_owned())?
    .map_err(|error| format!("account sandbox logout cleanup unconfirmed: {error:#}"))
}

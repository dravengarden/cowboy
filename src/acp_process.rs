//! The Provider process behind one ACP session: a local child of this
//! worker, or, for a Provider whose signed manifest requires
//! `account_sandbox_v1`, an account unit the `cowboy-account-sandbox` daemon
//! runs as that account's own identity (see [`crate::account_sandbox`]).
//! Either way the session relays ACP over the process's stdio pipes.

use super::{
    AgentSink, Arc, Command, Context, Duration, LaunchSpec, Mutex, PathBuf, Result, Stdio, cgroup,
    deepseek_session_environment,
};

use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::account_sandbox;

/// How often a sandboxed unit's liveness is confirmed with the daemon.
const UNIT_POLL: Duration = Duration::from_secs(2);
/// Bound of the daemon's stop confirmation at teardown.
const UNIT_STOP_TIMEOUT: Duration = Duration::from_secs(45);
/// Delay before the watchdog retries an unconfirmed account-unit stop.
const UNIT_STOP_RETRY: Duration = Duration::from_secs(5);

pub(super) struct AgentProcess {
    /// The cgroup containing a local process tree, when contained.
    pub cgroup: Option<PathBuf>,
    owner: Owner,
}

/// The session's ends of the Provider process's stdio.
pub(super) struct Pipes {
    pub stdin: Box<dyn AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn AsyncRead + Send + Unpin>,
    pub stderr: Box<dyn AsyncRead + Send + Unpin>,
}

enum Owner {
    Local(tokio::process::Child),
    Sandbox(SandboxUnit),
}

/// A daemon unit and the connection that owns it: the daemon stops the unit
/// when this connection closes.
struct SandboxUnit {
    client: Arc<Mutex<account_sandbox::Client>>,
    handle: account_sandbox::UnitHandle,
    /// The unit's execution relays still owed a reply.
    relays: Option<Arc<account_sandbox::RelayQuiet>>,
}

/// How long a unit's stop waits for its in-flight execution relays to be
/// answered (design L1c); within the worker's teardown settlement.
const UNIT_RELAY_DRAIN: Duration = Duration::from_secs(10);

/// The daemon bounds the runtime units of one account (`limit`, its
/// `max_runtime_units`): that many sessions of the account already run.
/// Explain that instead of a bare refusal.
fn at_limit(error: anyhow::Error, account_id: &str) -> anyhow::Error {
    if account_sandbox::DaemonError::code_of(&error) == Some("limit") {
        return anyhow::anyhow!("{}", at_limit_message(account_id));
    }
    error
}

/// The session error of a runtime beyond the account's bound.
fn at_limit_message(account_id: &str) -> String {
    let short: String = account_id.chars().take(8).collect();
    format!(
        "Antigravity \u{b7} account {short} already runs its maximum number of sessions; hibernate one first"
    )
}

impl AgentProcess {
    pub(super) async fn spawn(
        spec: &LaunchSpec,
        session_id: &str,
        cwd: &Path,
        sink: &dyn AgentSink,
    ) -> Result<(Self, Pipes)> {
        match &spec.account_sandbox {
            Some(launch) => {
                let bridge = spec.execution_bridge.clone();
                Self::start_sandboxed(launch.clone(), bridge, session_id, sink).await
            }
            None => Self::spawn_local(spec, session_id, cwd),
        }
    }

    fn spawn_local(spec: &LaunchSpec, session_id: &str, cwd: &Path) -> Result<(Self, Pipes)> {
        let mut command = Command::new(&spec.command);
        command.args(&spec.args);
        for (key, _) in std::env::vars_os() {
            if key
                .to_str()
                .is_some_and(|name| spec.removes_inherited_env(name))
            {
                command.env_remove(key);
            }
        }
        command.envs(&spec.env);
        if let Some((key, value)) = deepseek_session_environment(
            &spec.id,
            session_id,
            spec.env.get("ANTHROPIC_CUSTOM_HEADERS").map(String::as_str),
            std::env::var(crate::deepseek_cache::SESSION_POLICY_ENV)
                .ok()
                .as_deref(),
        ) {
            command.env(key, value);
        }
        // Every return from `agent_main` ends this session owner. Do not rely
        // on a Provider noticing stdin EOF promptly: a wedged adapter could
        // otherwise outlive its worker on macOS, where Linux cgroup cleanup is
        // unavailable.
        command.kill_on_drop(true);
        let mut child = command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("spawning provider {} ({})", spec.id, spec.command))?;
        // Contain the agent + everything it forks in its own cgroup so a
        // wedged turn's orphaned subprocesses (e.g. a detached `until …; do
        // sleep; done` poll loop) can be reaped wholesale on teardown /
        // recycle. Best-effort: None ⇒ the agent runs uncontained (see
        // crate::cgroup). Done before the agent forks anything.
        let cgroup = child.id().and_then(|pid| {
            let dir = cgroup::create(session_id)?;
            cgroup::add_pid(&dir, pid);
            Some(dir)
        });
        let pipes = Pipes {
            stdin: Box::new(child.stdin.take().context("child stdin")?),
            stdout: Box::new(child.stdout.take().context("child stdout")?),
            stderr: Box::new(child.stderr.take().context("child stderr")?),
        };
        let owner = Owner::Local(child);
        Ok((Self { cgroup, owner }, pipes))
    }

    /// Ask the daemon for the account's runtime unit on the exact native
    /// credential generation. An unreachable or refusing daemon fails the
    /// session; there is no local fallback.
    async fn start_sandboxed(
        launch: account_sandbox::Launch,
        bridge: Option<account_sandbox::ExecutionBridge>,
        session_id: &str,
        sink: &dyn AgentSink,
    ) -> Result<(Self, Pipes)> {
        let session = session_id.to_owned();
        let descriptor = bridge.as_ref().map(|bridge| bridge.descriptor.clone());
        let account_id = launch.account_id.clone();
        let (client, started) = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut client = account_sandbox::Client::connect(&launch.socket)?;
            let account = client.ensure_account(&launch.service_id, &launch.account_id)?;
            let started = account_sandbox::start_registering(
                &mut client,
                &account_sandbox::StartRequest {
                    key: &account.key,
                    mode: account_sandbox::Mode::Runtime,
                    credential_generation: launch.credential_generation,
                    runtime_sha256: &launch.runtime_sha256,
                    session_id: Some(&session),
                    descriptor: descriptor.as_deref(),
                },
                &launch.runtime_archive,
            )?;
            Ok((client, started))
        })
        .await
        .context("account sandbox start task failed")?
        .map_err(|error| at_limit(error, &account_id))
        .context("starting the Provider in the account sandbox")?;
        tracing::info!(
            session = session_id,
            unit = %started.handle.unit,
            invocation = started.handle.invocation_id.as_deref().unwrap_or(""),
            boot = %started.handle.boot_id,
            "account sandbox unit started"
        );
        sink.set_account_unit(session_id, Some(started.handle.clone()));
        // The unit reaches the worker's execution endpoint only through the
        // listening socket the daemon bound into it (design R3-11). Without
        // an execution binding nothing serves it and it is closed here.
        if let (Some(bridge), Some(listener)) = (&bridge, started.exec_listener) {
            // A replacement unit relays everything again, from its first
            // request on.
            bridge.relays.end_drain();
            bridge
                .listeners
                .send(listener)
                .map_err(|_| anyhow::anyhow!("the execution endpoint is gone"))?;
        }
        let pipes = Pipes {
            stdin: Box::new(tokio::net::unix::pipe::Sender::from_owned_fd(
                started.stdin,
            )?),
            stdout: Box::new(tokio::net::unix::pipe::Receiver::from_owned_fd(
                started.stdout,
            )?),
            stderr: Box::new(tokio::net::unix::pipe::Receiver::from_owned_fd(
                started.stderr,
            )?),
        };
        let owner = Owner::Sandbox(SandboxUnit {
            client: Arc::new(Mutex::new(client)),
            handle: started.handle,
            relays: bridge.as_ref().map(|bridge| Arc::clone(&bridge.relays)),
        });
        Ok((
            Self {
                cgroup: None,
                owner,
            },
            pipes,
        ))
    }

    /// How the prompt watchdog hard-recycles this Provider process.
    pub(super) fn recycler(&self) -> Recycler {
        match (&self.owner, &self.cgroup) {
            (Owner::Sandbox(unit), _) => Recycler::Unit {
                client: Arc::clone(&unit.client),
                unit: unit.handle.unit.clone(),
            },
            (Owner::Local(_), Some(dir)) => Recycler::Cgroup(dir.clone()),
            (Owner::Local(_), None) => Recycler::Uncontained,
        }
    }

    /// Resolves once the Provider process is gone, describing how.
    pub(super) async fn exited(&mut self) -> String {
        match &mut self.owner {
            Owner::Local(child) => match child.wait().await {
                Ok(status) => status.to_string(),
                Err(error) => format!("wait failed: {error}"),
            },
            Owner::Sandbox(unit) => unit.ended().await,
        }
    }

    /// End the Provider process. A sandboxed unit is stopped and confirmed
    /// gone by the daemon before this returns; closing the connection is the
    /// daemon's own fallback for every other exit path.
    pub(super) async fn shutdown(self) {
        match self.owner {
            // Dropping the child lets the agent see stdin EOF and exit (a no-op
            // if it already exited); `kill_on_drop` ends a wedged one.
            Owner::Local(child) => drop(child),
            Owner::Sandbox(unit) => unit.stop().await,
        }
        if let Some(dir) = &self.cgroup {
            cgroup::kill_and_remove(dir);
        }
    }
}

impl SandboxUnit {
    /// Poll the daemon until the unit is no longer this invocation alive. A
    /// lost control connection also ends the session: the daemon stops a
    /// unit whose owner connection closed.
    async fn ended(&self) -> String {
        loop {
            tokio::time::sleep(UNIT_POLL).await;
            let (client, unit) = (Arc::clone(&self.client), self.handle.unit.clone());
            let state = tokio::task::spawn_blocking(move || client.lock().unit_state(&unit)).await;
            match state {
                Ok(Ok(state)) if state.alive && state.owned => {}
                Ok(Ok(state)) => {
                    return format!(
                        "account sandbox unit {} ended ({})",
                        state.unit, state.state
                    );
                }
                Ok(Err(error)) => return format!("account sandbox connection lost: {error:#}"),
                Err(error) => return format!("account sandbox poll failed: {error}"),
            }
        }
    }

    async fn stop(self) {
        // Replies the unit is still owed reach it before it stops: a reply
        // cut off by the stop would leave its effect unknown (design L1c).
        if let Some(relays) = &self.relays {
            relays.begin_drain();
            let started = tokio::time::Instant::now();
            // Only the unit's own acknowledgement proves its replies were
            // read; an exit, a disconnect or the bound does not.
            let acknowledged = relays.drained(UNIT_RELAY_DRAIN).await
                && relays
                    .wait_acknowledged(UNIT_RELAY_DRAIN.saturating_sub(started.elapsed()))
                    .await;
            if !acknowledged {
                relays.forced();
                tracing::warn!(
                    unit = %self.handle.unit,
                    in_flight = relays.in_flight(),
                    "account sandbox unit stopped before its execution replies were confirmed"
                );
            }
        }
        let (client, unit) = (Arc::clone(&self.client), self.handle.unit.clone());
        let stop = tokio::task::spawn_blocking(move || client.lock().stop_unit(&unit));
        match tokio::time::timeout(UNIT_STOP_TIMEOUT, stop).await {
            Ok(Ok(Ok(_))) => {
                tracing::info!(unit = %self.handle.unit, "account sandbox unit stopped");
            }
            outcome => tracing::warn!(
                unit = %self.handle.unit,
                ?outcome,
                "account sandbox unit stop unconfirmed; closing its owner connection"
            ),
        }
    }
}

/// The prompt watchdog's hard recycle of one Provider process tree.
pub(super) enum Recycler {
    /// SIGKILL the local agent's containment cgroup.
    Cgroup(PathBuf),
    /// Ask the daemon to stop the account unit and wait for its confirmation.
    Unit {
        client: Arc<Mutex<account_sandbox::Client>>,
        unit: String,
    },
    /// A local agent the host could not contain.
    Uncontained,
}

impl Recycler {
    /// End the wedged process tree. `None` when it is done (or nothing can be
    /// done and the caller must wait for the prompt to fail on its own);
    /// `Some(retry_at)` when the daemon did not confirm an account unit's
    /// stop, which the caller retries then rather than treating it as ended.
    pub(super) async fn hard_recycle(&self, session_id: &str) -> Option<std::time::Instant> {
        match self {
            Self::Cgroup(dir) => {
                tracing::warn!(
                    session = session_id,
                    "prompt watchdog: agent will not yield to cancel — recycling"
                );
                cgroup::kill_and_remove(dir);
                None
            }
            Self::Unit { client, unit } => {
                tracing::warn!(
                    session = session_id,
                    %unit,
                    "prompt watchdog: agent will not yield to cancel — stopping its account unit"
                );
                let (client, name) = (Arc::clone(client), unit.clone());
                let stop = tokio::task::spawn_blocking(move || client.lock().stop_unit(&name));
                match tokio::time::timeout(UNIT_STOP_TIMEOUT, stop).await {
                    Ok(Ok(Ok(true))) => {
                        tracing::info!(session = session_id, %unit, "account unit stop confirmed");
                        None
                    }
                    outcome => {
                        tracing::warn!(
                            session = session_id,
                            %unit,
                            ?outcome,
                            "account unit stop unconfirmed; retrying"
                        );
                        Some(std::time::Instant::now() + UNIT_STOP_RETRY)
                    }
                }
            }
            Self::Uncontained => {
                tracing::warn!(
                    session = session_id,
                    "prompt watchdog: agent wedged and uncontained — waiting for prompt to fail"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead as _, Write as _};

    #[tokio::test]
    async fn an_unconfirmed_unit_stop_is_retried() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("control.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            for reply in [
                r#"{"ok":false,"error":{"code":"internal","message":"did not stop"}}"#,
                r#"{"ok":true,"result":{"unit":"u","stopped":true}}"#,
            ] {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                writeln!(stream, "{reply}").unwrap();
            }
        });
        let recycler = Recycler::Unit {
            client: Arc::new(Mutex::new(
                account_sandbox::Client::connect(&socket).unwrap(),
            )),
            unit: "cga-0123456789abcdef-runtime-0011223344556677.service".into(),
        };
        let retry = recycler.hard_recycle("session").await;
        assert!(retry.is_some_and(|at| at > std::time::Instant::now()));
        assert!(recycler.hard_recycle("session").await.is_none());
        server.join().unwrap();
    }

    #[test]
    fn a_runtime_beyond_the_account_bound_is_explained() {
        let limit = anyhow::Error::new(account_sandbox::DaemonError {
            code: "limit".into(),
            message: "runtime unit limit reached".into(),
        });
        let explained = at_limit(limit, "11111111-1111-4111-8111-111111111111").to_string();
        assert!(
            explained.contains("account 11111111 already runs its maximum number of sessions"),
            "{explained}"
        );
        let other = anyhow::Error::new(account_sandbox::DaemonError {
            code: "fenced".into(),
            message: String::new(),
        });
        assert!(at_limit(other, "a").to_string().contains("fenced"));
    }
}

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
}

impl AgentProcess {
    pub(super) async fn spawn(
        spec: &LaunchSpec,
        session_id: &str,
        cwd: &Path,
        sink: &dyn AgentSink,
    ) -> Result<(Self, Pipes)> {
        match &spec.account_sandbox {
            Some(launch) => Self::start_sandboxed(launch.clone(), session_id, sink).await,
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
        session_id: &str,
        sink: &dyn AgentSink,
    ) -> Result<(Self, Pipes)> {
        let session = session_id.to_owned();
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
                    descriptor: None,
                },
                &launch.runtime_archive,
            )?;
            Ok((client, started))
        })
        .await
        .context("account sandbox start task failed")?
        .context("starting the Provider in the account sandbox")?;
        tracing::info!(
            session = session_id,
            unit = %started.handle.unit,
            invocation = started.handle.invocation_id.as_deref().unwrap_or(""),
            boot = %started.handle.boot_id,
            "account sandbox unit started"
        );
        sink.set_account_unit(session_id, Some(started.handle.clone()));
        // TODO(3b): bridge the unit's execution listener to the worker's
        // execution endpoint (docs/antigravity-real-accounts-design.md
        // [R3-11]); until then the listener is closed and execution bindings
        // are refused for sandboxed Providers.
        drop(started.exec_listener);
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
        });
        Ok((
            Self {
                cgroup: None,
                owner,
            },
            pipes,
        ))
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

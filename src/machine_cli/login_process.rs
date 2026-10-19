//! One deadline owns authentication I/O, process completion and cancellation.
use std::process::ExitStatus;
use std::time::Duration;

use tokio::io::AsyncWriteExt as _;
use tokio::process::Child;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

pub(super) const TIMEOUT: Duration = Duration::from_secs(15 * 60);

mod output;
pub(super) use crate::cgroup::Containment;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Cancelled,
    Expired,
    Read,
    Write,
    Wait,
    Cleanup,
}

impl Failure {
    pub(super) fn detail(&self) -> &'static str {
        match self {
            Self::Cancelled => "login cancelled",
            Self::Expired => "login expired; start a new authorization",
            Self::Read => "could not read provider login output",
            Self::Write => "could not send the authorization value",
            Self::Wait => "could not wait for provider login completion",
            Self::Cleanup => "provider login process exit is unconfirmed; cleanup is required",
        }
    }
}

/// Synchronous admission before native spawn. Call while holding the login
/// registry lock so connection teardown cannot revoke between check and spawn.
pub(super) fn check(cancel: &watch::Receiver<bool>, deadline: Instant) -> Result<(), Failure> {
    if *cancel.borrow() || cancel.has_changed().is_err() {
        Err(Failure::Cancelled)
    } else if Instant::now() >= deadline {
        Err(Failure::Expired)
    } else {
        Ok(())
    }
}

/// A later cancellation cannot turn missing exit evidence into a clean stop.
pub(super) fn reconcile(
    status: Result<ExitStatus, Failure>,
    registered: bool,
    cancel: &watch::Receiver<bool>,
    deadline: Instant,
) -> Result<ExitStatus, Failure> {
    if matches!(status, Err(Failure::Cleanup)) {
        return status;
    }
    if !registered {
        return Err(Failure::Cancelled);
    }
    check(cancel, deadline).and(status)
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() || cancel.changed().await.is_err() {
            return;
        }
    }
}

/// Caller holds the login registry lock until publication has finished.
pub(super) fn publish(
    registered: bool,
    cancel: &watch::Receiver<bool>,
    deadline: Instant,
    send: impl FnOnce() -> Result<(), ()>,
) -> Result<(), Failure> {
    if !registered {
        return Err(Failure::Cancelled);
    }
    check(cancel, deadline)?;
    send().map_err(|()| Failure::Write)
}

pub(super) async fn input(
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::Receiver<String>,
    deadline: Instant,
) -> Result<String, Failure> {
    tokio::select! {
        biased;
        () = cancelled(cancel) => Err(Failure::Cancelled),
        () = tokio::time::sleep_until(deadline) => Err(Failure::Expired),
        value = input.recv() => value.ok_or(Failure::Cancelled),
    }
}

/// The caller must spawn a fresh process group. Cancellation also covers a
/// blocked stdin write and a child that closed its output but has not exited.
pub(super) async fn run(
    child: Child,
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::Receiver<String>,
    deadline: Instant,
    on_line: impl FnMut(String),
) -> Result<ExitStatus, Failure> {
    let process_group = child
        .id()
        .map(crate::plugin_process::PluginProcessGroup::new);
    run_owned(child, cancel, input, deadline, on_line, process_group).await
}

/// The pre-spawn guard already owns containment; never move this child into
/// the legacy post-spawn group, which would invalidate its exit observation.
pub(super) async fn run_contained(
    child: Child,
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::Receiver<String>,
    deadline: Instant,
    on_line: impl FnMut(String),
    containment: Containment,
) -> Result<ExitStatus, Failure> {
    let result = run_owned(child, cancel, input, deadline, on_line, None).await;
    containment.finish().await.map_err(|_| Failure::Cleanup)?;
    result
}

pub(super) async fn run_optional_containment(
    child: Child,
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::Receiver<String>,
    deadline: Instant,
    on_line: impl FnMut(String),
    containment: Option<Containment>,
) -> Result<ExitStatus, Failure> {
    match containment {
        Some(guard) => run_contained(child, cancel, input, deadline, on_line, guard).await,
        None => run(child, cancel, input, deadline, on_line).await,
    }
}

async fn run_owned(
    mut child: Child,
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::Receiver<String>,
    deadline: Instant,
    on_line: impl FnMut(String),
    process_group: Option<crate::plugin_process::PluginProcessGroup>,
) -> Result<ExitStatus, Failure> {
    let result = tokio::select! {
        biased;
        () = cancelled(cancel) => Err(Failure::Cancelled),
        () = tokio::time::sleep_until(deadline) => Err(Failure::Expired),
        result = drive(&mut child, input, on_line) => result,
    };
    // Signal the owned process group and delegated cgroup when available,
    // before waiting for the leader. Dropping this future uses the same guard.
    drop(process_group);
    if result.is_err() {
        let _ = child.start_kill();
        confirm_leader_exit(child.wait(), Duration::from_secs(2)).await?;
    }
    result
}

/// A kill signal is not exit evidence. This confirms only the owned leader;
/// process-group/cgroup cleanup remains a separate containment obligation.
async fn confirm_leader_exit(
    wait: impl std::future::Future<Output = std::io::Result<ExitStatus>>,
    timeout: Duration,
) -> Result<(), Failure> {
    match tokio::time::timeout(timeout, wait).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(Failure::Cleanup),
    }
}

async fn drive(
    child: &mut Child,
    input: &mut mpsc::Receiver<String>,
    mut on_line: impl FnMut(String),
) -> Result<ExitStatus, Failure> {
    let mut stdin = child.stdin.take().expect("piped login stdin");
    let mut stdout = output::Output::new(child.stdout.take().expect("piped login stdout"));
    let mut stderr = output::Output::new(child.stderr.take().expect("piped login stderr"));
    let mut stdout_open = true;
    let mut stderr_open = true;
    loop {
        let line = tokio::select! {
            status = child.wait() => return status.map_err(|_| Failure::Wait),
            line = stdout.next_line(), if stdout_open => {
                if matches!(line, Ok(None)) { stdout_open = false; }
                line
            },
            line = stderr.next_line(), if stderr_open => {
                if matches!(line, Ok(None)) { stderr_open = false; }
                line
            },
            value = input.recv() => {
                let value = value.ok_or(Failure::Cancelled)?;
                stdin.write_all(value.as_bytes()).await.map_err(|_| Failure::Write)?;
                stdin.write_all(b"\n").await.map_err(|_| Failure::Write)?;
                continue;
            },
        };
        if let Some(line) = line.map_err(|_| Failure::Read)? {
            on_line(line);
        }
    }
}

#[cfg(all(test, unix))]
mod tests;

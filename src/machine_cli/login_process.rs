//! One deadline owns authentication I/O, process completion and cancellation.
use std::process::ExitStatus;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

pub(super) const TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Cancelled,
    Expired,
    Read,
    Write,
    Wait,
}

impl Failure {
    pub(super) fn detail(&self) -> &'static str {
        match self {
            Self::Cancelled => "login cancelled",
            Self::Expired => "login expired; start a new authorization",
            Self::Read => "could not read provider login output",
            Self::Write => "could not send the authorization value",
            Self::Wait => "could not wait for provider login completion",
        }
    }
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() || cancel.changed().await.is_err() {
            return;
        }
    }
}

pub(super) async fn input(
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::UnboundedReceiver<String>,
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
    mut child: Child,
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::UnboundedReceiver<String>,
    deadline: Instant,
    on_line: impl FnMut(String),
) -> Result<ExitStatus, Failure> {
    let process_group = child
        .id()
        .map(crate::plugin_process::PluginProcessGroup::new);
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
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    }
    result
}

async fn drive(
    child: &mut Child,
    input: &mut mpsc::UnboundedReceiver<String>,
    mut on_line: impl FnMut(String),
) -> Result<ExitStatus, Failure> {
    let mut stdin = child.stdin.take().expect("piped login stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped login stdout")).lines();
    let mut stderr = BufReader::new(child.stderr.take().expect("piped login stderr")).lines();
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

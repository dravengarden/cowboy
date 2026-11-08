//! Client side of the launcher control protocol (`cowboy-browser-control-v1`):
//! newline-delimited JSON over the instance's control socket. The launcher
//! reports readiness facts the Machine re-verifies from outside; the Machine
//! sends the closed command set and treats every reply and event as data,
//! never as something to execute.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub const PROTOCOL: &str = cowboy_plugin_sdk::MACHINE_BROWSER_CONTROL_PROTOCOL;
pub const CONTROL_SOCKET: &str = "control.sock";
pub const RFB_SOCKET: &str = "rfb.sock";
/// Longest control line accepted in either direction.
const MAX_LINE_BYTES: usize = 256 * 1024;
const CONNECT_POLL: Duration = Duration::from_millis(100);

/// Namespace-local pids as the init sees them (init is 1); `supervisor` is
/// the host pid of the unit's main process.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIds {
    #[serde(default)]
    pub supervisor: u32,
    pub init: u32,
    pub display_server: u32,
    pub window_manager: u32,
    pub browser: u32,
}

/// `/proc/self/ns/*` identities of the namespace init, e.g. `user:[4026532641]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespaces {
    pub user: String,
    pub mnt: String,
    pub pid: String,
    pub ipc: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labels {
    pub init: String,
    #[serde(default)]
    pub sealed: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountEntry {
    pub target: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub fstype: String,
    #[serde(default)]
    pub options: String,
}

/// Facts the launcher reports at readiness and on `status`. The Machine
/// verifies every one of them from outside before trusting the instance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyReport {
    pub pids: ProcessIds,
    #[serde(default)]
    pub start_times: BTreeMap<String, u64>,
    pub namespaces: Namespaces,
    pub uid_map: String,
    pub gid_map: String,
    pub labels: Labels,
    #[serde(default)]
    pub mounts: Vec<MountEntry>,
    #[serde(default)]
    pub snapshot_in_progress: bool,
}

/// Failure class reported by the launcher; it decides the reason the
/// Controller learns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Sandbox,
    Host,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum LauncherEvent {
    Ready(Box<ReadyReport>),
    Exited {
        process: String,
        #[serde(default)]
        code: Option<i32>,
    },
    SnapshotDone {
        generation: u64,
        #[serde(default)]
        bytes: u64,
        #[serde(default)]
        entries: u64,
    },
    Error {
        #[serde(default)]
        kind: Option<FailureKind>,
        message: String,
    },
    #[serde(other)]
    Unknown,
}

/// The launcher refused or failed a command. Distinct from a transport
/// failure so callers can tell "the launcher said no" from "the launcher is
/// gone".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherRefusal {
    pub kind: Option<FailureKind>,
    pub message: String,
}

impl std::fmt::Display for LauncherRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "launcher refused: {}", self.message)
    }
}

impl std::error::Error for LauncherRefusal {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotOutcome {
    pub bytes: u64,
    pub entries: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopOutcome {
    /// Every browser process exited on `SIGTERM` within the grace, so the
    /// profile copy (if any) is flushed, not crash-consistent.
    pub graceful: bool,
    /// A stop-time copy was made into the requested staging generation.
    pub copied: bool,
}

pub struct Control {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next_id: u64,
    events: VecDeque<LauncherEvent>,
    /// Bytes of a control line not yet terminated. `read_until` appends what
    /// it consumed before a timeout cancels it, so the prefix survives here
    /// and the next read completes the same line.
    partial: Vec<u8>,
}

impl Control {
    /// Connect to `control.sock` below `directory`, waiting up to `timeout`
    /// for the launcher to create it.
    ///
    /// # Errors
    /// Returns when the socket does not appear or refuses within `timeout`.
    pub async fn connect(directory: &Path, timeout: Duration) -> Result<Self> {
        let path = directory.join(CONTROL_SOCKET);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match UnixStream::connect(&path).await {
                Ok(stream) => {
                    let (reader, writer) = stream.into_split();
                    return Ok(Self {
                        reader: BufReader::new(reader),
                        writer,
                        next_id: 1,
                        events: VecDeque::new(),
                        partial: Vec::new(),
                    });
                }
                Err(error) if tokio::time::Instant::now() >= deadline => {
                    return Err(error).with_context(|| format!("connecting {}", path.display()));
                }
                Err(_) => tokio::time::sleep(CONNECT_POLL).await,
            }
        }
    }

    /// Read until the launcher reports readiness.
    ///
    /// # Errors
    /// Returns the launcher's own failure, or a transport failure or timeout.
    pub async fn wait_ready(&mut self, timeout: Duration) -> Result<ReadyReport> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            ensure!(!remaining.is_zero(), "launcher readiness timed out");
            match self.read_message(remaining).await? {
                Message::Event(LauncherEvent::Ready(report)) => return Ok(*report),
                Message::Event(LauncherEvent::Error { kind, message }) => {
                    return Err(LauncherRefusal { kind, message }.into());
                }
                Message::Event(LauncherEvent::Exited { process, code }) => {
                    bail!("launcher process {process} exited before readiness (code {code:?})")
                }
                Message::Event(other) => self.events.push_back(other),
                Message::Reply(_) => {}
            }
        }
    }

    /// Events received while waiting for replies, oldest first.
    pub fn take_events(&mut self) -> Vec<LauncherEvent> {
        self.events.drain(..).collect()
    }

    /// Wait for the next event, draining buffered ones first.
    ///
    /// # Errors
    /// Returns on a transport failure; `Ok(None)` when `timeout` elapses.
    pub async fn next_event(&mut self, timeout: Duration) -> Result<Option<LauncherEvent>> {
        if let Some(event) = self.events.pop_front() {
            return Ok(Some(event));
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            match tokio::time::timeout(remaining, self.read_line()).await {
                Ok(message) => match parse(&message?)? {
                    Message::Event(event) => return Ok(Some(event)),
                    Message::Reply(_) => {}
                },
                Err(_) => return Ok(None),
            }
        }
    }

    /// # Errors
    /// Returns the launcher's refusal or a transport failure.
    pub async fn navigate(&mut self, url: &str, timeout: Duration) -> Result<()> {
        self.request("navigate", json!({ "url": url }), timeout)
            .await
            .map(|_| ())
    }

    /// # Errors
    /// Returns the launcher's refusal or a transport failure.
    pub async fn resize(&mut self, width: u32, height: u32, timeout: Duration) -> Result<()> {
        self.request(
            "resize",
            json!({ "width": width, "height": height }),
            timeout,
        )
        .await
        .map(|_| ())
    }

    /// # Errors
    /// Returns the launcher's refusal or a transport failure.
    pub async fn attach(&mut self, attachment_id: &str, timeout: Duration) -> Result<()> {
        self.request("attach", json!({ "attachment_id": attachment_id }), timeout)
            .await
            .map(|_| ())
    }

    /// # Errors
    /// Returns the launcher's refusal or a transport failure.
    pub async fn detach(&mut self, attachment_id: &str, timeout: Duration) -> Result<()> {
        self.request("detach", json!({ "attachment_id": attachment_id }), timeout)
            .await
            .map(|_| ())
    }

    /// Ask the launcher to copy the live profile into the staging generation
    /// the Machine created. The reply arrives when the copy is complete.
    ///
    /// # Errors
    /// Returns the launcher's refusal or a transport failure.
    pub async fn snapshot(
        &mut self,
        generation: u64,
        timeout: Duration,
    ) -> Result<SnapshotOutcome> {
        let reply = self
            .request("snapshot", json!({ "generation": generation }), timeout)
            .await?;
        Ok(SnapshotOutcome {
            bytes: reply.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            entries: reply.get("entries").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    /// Graceful stop: `SIGTERM` to the browser main process, bounded wait,
    /// an optional stop-time copy of the profile into the staging generation
    /// the Machine created, then display shutdown and exit.
    ///
    /// # Errors
    /// Returns the launcher's refusal or a transport failure.
    pub async fn stop(
        &mut self,
        graceful: Duration,
        snapshot_generation: Option<u64>,
        timeout: Duration,
    ) -> Result<StopOutcome> {
        let mut arguments =
            json!({ "graceful_ms": u64::try_from(graceful.as_millis()).unwrap_or(u64::MAX) });
        if let Some(generation) = snapshot_generation {
            arguments["snapshot_generation"] = json!(generation);
        }
        let reply = self.request("stop", arguments, timeout).await?;
        if let Some(error) = reply.get("snapshot_error").and_then(Value::as_str) {
            tracing::warn!(error, "launcher stop-time copy failed");
        }
        Ok(StopOutcome {
            graceful: reply
                .get("graceful")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            copied: reply.get("snapshot").is_some_and(Value::is_object),
        })
    }

    /// The readiness facts again, for adoption after a Machine restart.
    ///
    /// # Errors
    /// Returns the launcher's refusal or a transport failure.
    pub async fn status(&mut self, timeout: Duration) -> Result<ReadyReport> {
        let reply = self.request("status", json!({}), timeout).await?;
        serde_json::from_value(reply).context("decoding launcher status")
    }

    async fn request(
        &mut self,
        command: &str,
        mut arguments: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let object = arguments
            .as_object_mut()
            .context("launcher command arguments must be an object")?;
        object.insert("id".to_owned(), json!(id));
        object.insert("command".to_owned(), json!(command));
        let mut line = serde_json::to_vec(&arguments)?;
        ensure!(line.len() < MAX_LINE_BYTES, "launcher command is too large");
        line.push(b'\n');
        let deadline = tokio::time::Instant::now() + timeout;
        tokio::time::timeout(timeout, self.writer.write_all(&line))
            .await
            .context("launcher command write timed out")??;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            ensure!(!remaining.is_zero(), "launcher {command} timed out");
            match self.read_message(remaining).await? {
                Message::Reply(reply) => {
                    if reply.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(reply);
                    }
                    let kind = reply
                        .get("kind")
                        .cloned()
                        .and_then(|kind| serde_json::from_value(kind).ok());
                    let message = reply
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unspecified failure")
                        .to_owned();
                    return Err(LauncherRefusal { kind, message }.into());
                }
                Message::Event(event) => self.events.push_back(event),
            }
        }
    }

    async fn read_message(&mut self, timeout: Duration) -> Result<Message> {
        let line = tokio::time::timeout(timeout, self.read_line())
            .await
            .context("launcher control read timed out")??;
        parse(&line)
    }

    /// One complete control line. Cancel-safe: bytes read before a timeout
    /// stay in `partial`, and the bound applies to the whole line.
    async fn read_line(&mut self) -> Result<Vec<u8>> {
        loop {
            if self.partial.last() == Some(&b'\n') {
                return Ok(std::mem::take(&mut self.partial));
            }
            ensure!(
                self.partial.len() <= MAX_LINE_BYTES,
                "launcher control line exceeds the bound"
            );
            let budget = (MAX_LINE_BYTES + 1 - self.partial.len()) as u64;
            let read = (&mut self.reader)
                .take(budget)
                .read_until(b'\n', &mut self.partial)
                .await
                .context("reading launcher control socket")?;
            ensure!(read > 0, "launcher control socket closed");
            ensure!(
                self.partial.len() <= MAX_LINE_BYTES,
                "launcher control line exceeds the bound"
            );
        }
    }
}

enum Message {
    Reply(Value),
    Event(LauncherEvent),
}

fn parse(line: &[u8]) -> Result<Message> {
    let value: Value = serde_json::from_slice(line).context("decoding launcher control line")?;
    ensure!(value.is_object(), "launcher control line is not an object");
    if value.get("id").is_some() {
        return Ok(Message::Reply(value));
    }
    let event: LauncherEvent =
        serde_json::from_value(value).context("decoding launcher control event")?;
    Ok(Message::Event(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_and_replies_are_told_apart_and_unknown_events_decode() {
        let ready = br#"{"event":"ready","pids":{"init":1,"display_server":7,"window_manager":9,"browser":12},
            "namespaces":{"user":"user:[1]","mnt":"mnt:[2]","pid":"pid:[3]","ipc":"ipc:[4]"},
            "uid_map":"1000 1000 1","gid_map":"1000 1000 1","labels":{"init":"cowboy-browser-bootstrap (enforce)"}}"#;
        let Message::Event(LauncherEvent::Ready(report)) = parse(ready).unwrap() else {
            panic!("ready event expected");
        };
        assert_eq!(report.pids.browser, 12);
        assert!(!report.snapshot_in_progress);
        assert!(matches!(
            parse(br#"{"event":"something_new","x":1}"#).unwrap(),
            Message::Event(LauncherEvent::Unknown)
        ));
        let Message::Event(LauncherEvent::Error { kind, .. }) =
            parse(br#"{"event":"error","kind":"sandbox","message":"mount denied"}"#).unwrap()
        else {
            panic!("error event expected");
        };
        assert_eq!(kind, Some(FailureKind::Sandbox));
        assert!(matches!(
            parse(br#"{"id":3,"ok":true}"#).unwrap(),
            Message::Reply(_)
        ));
        assert!(parse(b"[1,2]").is_err());
    }

    #[tokio::test]
    async fn requests_match_replies_by_id_and_queue_interleaved_events() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(CONTROL_SOCKET);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut lines = BufReader::new(reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["command"], "navigate");
            assert_eq!(request["url"], "https://example.com");
            writer
                .write_all(
                    b"{\"event\":\"snapshot_done\",\"generation\":4}\n{\"id\":99,\"ok\":true}\n",
                )
                .await
                .unwrap();
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"ok\":false,\"error\":\"no\",\"kind\":\"host\"}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let mut control = Control::connect(temp.path(), Duration::from_secs(2))
            .await
            .unwrap();
        let error = control
            .navigate("https://example.com", Duration::from_secs(2))
            .await
            .unwrap_err();
        let refusal = error.downcast_ref::<LauncherRefusal>().unwrap();
        assert_eq!(refusal.kind, Some(FailureKind::Host));
        assert_eq!(
            control.take_events(),
            vec![LauncherEvent::SnapshotDone {
                generation: 4,
                bytes: 0,
                entries: 0
            }]
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_line_split_across_a_read_timeout_is_not_lost() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(CONTROL_SOCKET);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (_reader, mut writer) = stream.into_split();
            writer
                .write_all(b"{\"event\":\"snapshot_done\",\"gen")
                .await
                .unwrap();
            writer.flush().await.unwrap();
            released.await.unwrap();
            writer.write_all(b"eration\":7}\n").await.unwrap();
            writer.flush().await.unwrap();
            // Keep the socket open until the client has read the line.
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let mut control = Control::connect(temp.path(), Duration::from_secs(2))
            .await
            .unwrap();
        // The prefix arrives, then the read times out mid-line.
        assert_eq!(
            control.next_event(Duration::from_millis(50)).await.unwrap(),
            None
        );
        release.send(()).unwrap();
        assert_eq!(
            control.next_event(Duration::from_secs(2)).await.unwrap(),
            Some(LauncherEvent::SnapshotDone {
                generation: 7,
                bytes: 0,
                entries: 0
            })
        );
        server.await.unwrap();
    }
}

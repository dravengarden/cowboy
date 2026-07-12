//! Versioned local IPC contract between Cowboy, `cowboy-agentd`, and workers.
//!
//! This protocol is deliberately independent from ACP. ACP is the downstream
//! worker-to-agent protocol; this is Cowboy's stable process boundary. Every
//! rolling deployment must retain at least one overlapping protocol version
//! until all workers using the older version have drained.

use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Current runtime protocol. Version 1 is additive-only: new optional fields
/// must use serde defaults and existing meanings must not change in place.
pub const PROTOCOL_VERSION: u16 = 1;
/// Oldest runtime protocol this build accepts during a mixed-generation roll.
pub const MIN_PROTOCOL_VERSION: u16 = 1;
/// Upper bound for a single IPC frame. Large binary artifacts are referenced by
/// content hash elsewhere; accepting unbounded JSON here would turn a corrupt
/// peer into an allocation denial of service.
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    Core,
    Worker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Starting,
    Running,
    Busy,
    Draining,
    Exited,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerSnapshot {
    pub session_id: String,
    pub worker_epoch: String,
    pub generation: String,
    /// Concrete executable for this generation. Agentd retains it so a failed
    /// new generation can be rolled back even after agentd itself restarted.
    #[serde(default)]
    pub executable: Option<String>,
    /// Full relaunch specification, self-reported by the worker so a freshly
    /// restarted agentd can reconstruct its session registry before the next
    /// generation handoff.
    #[serde(default)]
    pub launch: Option<StartSession>,
    pub state: WorkerState,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub current_turn_id: Option<String>,
    #[serde(default)]
    pub last_runtime_seq: u64,
    #[serde(default)]
    pub pending_permissions: Vec<String>,
    #[serde(default)]
    pub config_options: Option<serde_json::Value>,
    #[serde(default)]
    pub context_used: Option<u64>,
    #[serde(default)]
    pub context_size: Option<u64>,
    /// Prompts accepted by agentd but intentionally held until a worker is
    /// ready (for example during generation drain).
    #[serde(default)]
    pub pending_prompt_count: u64,
    #[serde(default)]
    pub drain_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartSession {
    pub session_id: String,
    pub provider: String,
    pub cwd: String,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub system: bool,
    pub generation: String,
    /// Desired generation this session is temporarily falling back from.
    #[serde(default)]
    pub fallback_for: Option<String>,
    /// Rebuild agentd's launch registry without spawning a missing worker.
    /// Cowboy sends this on broker reconnect while surviving workers are still
    /// converging; a later real EnsureSession may start one if it never returns.
    #[serde(default)]
    pub adopt_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CoreCommand {
    EnsureSession {
        session: StartSession,
    },
    Prompt {
        session_id: String,
        command_id: String,
        turn_id: String,
        content: Vec<serde_json::Value>,
        #[serde(default)]
        cmid: Option<String>,
    },
    Cancel {
        session_id: String,
        command_id: String,
    },
    Permission {
        session_id: String,
        command_id: String,
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
    SetConfigOption {
        session_id: String,
        command_id: String,
        config_id: String,
        value: serde_json::Value,
    },
    DrainSession {
        session_id: String,
        generation: String,
    },
    StopSession {
        session_id: String,
        command_id: String,
    },
    SetDesiredGeneration {
        generation: String,
        /// Concrete worker executable for this generation. Optional keeps the
        /// v1 wire format compatible with older cores during a rolling update.
        #[serde(default)]
        worker_command: Option<String>,
    },
}

impl CoreCommand {
    #[must_use]
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::EnsureSession { session } => Some(&session.session_id),
            Self::Prompt { session_id, .. }
            | Self::Cancel { session_id, .. }
            | Self::Permission { session_id, .. }
            | Self::SetConfigOption { session_id, .. }
            | Self::DrainSession { session_id, .. }
            | Self::StopSession { session_id, .. } => Some(session_id),
            Self::SetDesiredGeneration { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WorkerCommand {
    Prompt {
        command_id: String,
        turn_id: String,
        content: Vec<serde_json::Value>,
        #[serde(default)]
        cmid: Option<String>,
    },
    Cancel {
        command_id: String,
    },
    Permission {
        command_id: String,
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
    SetConfigOption {
        command_id: String,
        config_id: String,
        value: serde_json::Value,
    },
    Drain,
    Stop {
        command_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeEvent {
    Ready {
        #[serde(default)]
        agent_session_id: Option<String>,
    },
    Status {
        state: WorkerState,
        #[serde(default)]
        detail: Option<String>,
    },
    Update {
        update: serde_json::Value,
        #[serde(default)]
        cmid: Option<String>,
    },
    ConfigOptions {
        options: serde_json::Value,
    },
    ContextUsage {
        used: u64,
        size: u64,
        #[serde(default)]
        raw: serde_json::Value,
        #[serde(default)]
        observed_at_ms: i64,
    },
    PermissionRequest {
        request_id: String,
        tool_call: serde_json::Value,
        options: serde_json::Value,
    },
    PermissionResolved {
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
    TurnStarted {
        turn_id: String,
        command_id: String,
    },
    TurnEnded {
        turn_id: String,
        stop_reason: String,
    },
    AgentSessionId {
        agent_session_id: String,
    },
    ScheduleWakeup {
        delay_seconds: i64,
        prompt: String,
    },
    UndeliveredPrompt {
        command_id: String,
        #[serde(default)]
        cmid: Option<String>,
        content: Vec<serde_json::Value>,
        error: String,
    },
    CommandRejected {
        command_id: String,
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Frame {
    Hello {
        role: PeerRole,
        min_protocol: u16,
        max_protocol: u16,
        build: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        worker_epoch: Option<String>,
        #[serde(default)]
        generation: Option<String>,
        #[serde(default)]
        executable: Option<String>,
        #[serde(default)]
        fallback_for: Option<String>,
    },
    Welcome {
        protocol: u16,
        controller_epoch: u64,
        #[serde(default)]
        workers: Vec<WorkerSnapshot>,
    },
    Reject {
        reason: String,
    },
    CoreCommand {
        command: CoreCommand,
    },
    WorkerCommand {
        session_id: String,
        command: WorkerCommand,
    },
    WorkerEvent {
        session_id: String,
        worker_epoch: String,
        runtime_seq: u64,
        event: RuntimeEvent,
    },
    Ack {
        session_id: String,
        worker_epoch: String,
        runtime_seq: u64,
    },
    /// Ask a worker to replay every still-unacknowledged event after this
    /// sequence. Used whenever a new Cowboy controller takes the lease while
    /// the worker-to-agentd connection itself stayed up.
    Replay {
        session_id: String,
        worker_epoch: String,
        after_runtime_seq: u64,
    },
    CommandAck {
        session_id: String,
        command_id: String,
        accepted: bool,
        #[serde(default)]
        reason: Option<String>,
    },
    Snapshot {
        // Keep the frequently queued wire envelope small. Serde still emits
        // the same object shape, so this is not a protocol change.
        worker: Box<WorkerSnapshot>,
    },
    Heartbeat,
}

#[must_use]
pub fn negotiate(local_min: u16, local_max: u16, peer_min: u16, peer_max: u16) -> Option<u16> {
    let low = local_min.max(peer_min);
    let high = local_max.min(peer_max);
    (low <= high).then_some(high)
}

/// Write one length-prefixed JSON frame.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Frame) -> io::Result<()> {
    let payload = serde_json::to_vec(frame)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("runtime frame too large: {} bytes", payload.len()),
        ));
    }
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runtime frame length overflow"))?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

/// Read one length-prefixed JSON frame. EOF before a new header is a clean
/// disconnect; EOF inside a frame is reported as corruption.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Option<Frame>> {
    let mut header = [0_u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("runtime frame too large: {len} bytes"),
        ));
    }
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_negotiation_requires_overlap_and_selects_newest() {
        assert_eq!(negotiate(1, 2, 2, 3), Some(2));
        assert_eq!(negotiate(1, 1, 2, 2), None);
        assert_eq!(negotiate(2, 4, 1, 3), Some(3));
    }

    #[tokio::test]
    async fn framed_json_round_trips_fragment_safe() {
        let (mut left, mut right) = tokio::io::duplex(128);
        let frame = Frame::WorkerEvent {
            session_id: "sess-1".to_owned(),
            worker_epoch: "epoch-1".to_owned(),
            runtime_seq: 9,
            event: RuntimeEvent::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "hello\nworld"},
                }),
                cmid: None,
            },
        };
        let expected = frame.clone();
        let send = tokio::spawn(async move { write_frame(&mut left, &frame).await });
        let received = read_frame(&mut right).await.expect("read frame");
        send.await.expect("writer task").expect("write frame");
        assert_eq!(received, Some(expected));
    }

    #[test]
    fn old_additive_frames_ignore_future_fields() {
        let raw = serde_json::json!({
            "type": "heartbeat",
            "futureOptionalField": true
        });
        let parsed: Frame = serde_json::from_value(raw).expect("additive field must be ignored");
        assert_eq!(parsed, Frame::Heartbeat);
    }

    #[test]
    fn old_worker_snapshot_defaults_new_recovery_fields() {
        let raw = serde_json::json!({
            "type": "snapshot",
            "worker": {
                "session_id": "sess-1",
                "worker_epoch": "epoch-1",
                "generation": "gen-1",
                "state": "busy"
            }
        });
        let Frame::Snapshot { worker } =
            serde_json::from_value(raw).expect("old snapshot remains compatible")
        else {
            panic!("snapshot frame")
        };
        assert!(worker.launch.is_none());
        assert!(worker.config_options.is_none());
        assert_eq!(worker.pending_prompt_count, 0);
    }

    #[test]
    fn old_ensure_session_defaults_to_real_spawn_semantics() {
        let raw = serde_json::json!({
            "command": "ensure_session",
            "session": {
                "session_id": "sess-1",
                "provider": "codex",
                "cwd": "/work",
                "generation": "gen-1"
            }
        });
        let CoreCommand::EnsureSession { session } =
            serde_json::from_value(raw).expect("old ensure remains compatible")
        else {
            panic!("ensure command")
        };
        assert!(!session.adopt_only);
    }
}

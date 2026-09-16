//! Private typed protobuf extension, separate from upstream read operations.
//! A transport error is not proof that an admitted native mutation did nothing.
use super::*;

#[allow(
    dead_code,
    reason = "generated complete private wire codec, shared with the native server"
)]
pub(super) mod wire {
    include!(concat!(env!("OUT_DIR"), "/zed.messages.rs"));
}

use wire::cowboy_buffer_sync::Action;
use wire::cowboy_buffer_sync_envelope::Payload;
use wire::cowboy_buffer_sync_response::{Phase, Refusal};

type ResponseSender = oneshot::Sender<Option<wire::CowboyBufferSyncResponse>>;
type Pending = Arc<std::sync::Mutex<HashMap<u32, ResponseSender>>>;

#[cfg(test)]
mod connected;
#[cfg(test)]
mod tests;

fn valid_version(version: &[wire::CowboyBufferSyncVersion]) -> bool {
    version.len() <= 256
        && version
            .iter()
            .all(|entry| u16::try_from(entry.replica_id).is_ok() && entry.timestamp > 0)
        && version
            .windows(2)
            .all(|pair| pair[0].replica_id < pair[1].replica_id)
}

fn no_content(request: &wire::CowboyBufferSync) -> bool {
    request.buffer_id == 0
        && request.version.is_empty()
        && request.content_sha256.is_empty()
        && request.content_bytes == 0
}

fn validate_request(request: &wire::CowboyBufferSync) -> Result<Action> {
    anyhow::ensure!(
        request.project_id == proto::REMOTE_SERVER_PROJECT_ID
            && request.protocol == 1
            && request.encoded_len() <= 8 * 1024,
        "invalid private native request"
    );
    let action = Action::from_i32(request.action).context("invalid native sync action")?;
    let valid = match action {
        Action::Probe => {
            request.instance.is_empty() && request.operation_id == 0 && no_content(request)
        }
        Action::Prepare => {
            request.instance.len() == 16
                && request.operation_id == 0
                && request.buffer_id > 0
                && valid_version(&request.version)
                && request.content_sha256.len() == 32
                && request.content_bytes <= 4 * 1024 * 1024
        }
        Action::Apply | Action::Query | Action::Retire => {
            request.instance.len() == 16 && request.operation_id > 0 && no_content(request)
        }
        Action::Unspecified => false,
    };
    anyhow::ensure!(valid, "invalid private native request shape");
    Ok(action)
}

fn validate_response(
    request: &wire::CowboyBufferSync,
    response: &wire::CowboyBufferSyncResponse,
) -> Result<()> {
    let action = validate_request(request)?;
    anyhow::ensure!(
        response.protocol == 1
            && response.instance.len() == 16
            && valid_version(&response.version)
            && response.content_bytes <= 4 * 1024 * 1024,
        "invalid private native response"
    );
    let phase = Phase::from_i32(response.phase).context("invalid native sync phase")?;
    let correlated = match action {
        Action::Probe => phase == Phase::Supported && response.operation_id == 0,
        Action::Prepare => {
            phase == Phase::Prepared
                && response.operation_id > 0
                && response.instance == request.instance
        }
        Action::Apply | Action::Query | Action::Retire => {
            response.instance == request.instance
                && response.operation_id == request.operation_id
                && match action {
                    Action::Apply => matches!(
                        phase,
                        Phase::Pending | Phase::Applied | Phase::Refused | Phase::Retired
                    ),
                    Action::Retire => matches!(phase, Phase::Pending | Phase::Retired),
                    _ => matches!(
                        phase,
                        Phase::Prepared
                            | Phase::Pending
                            | Phase::Applied
                            | Phase::Refused
                            | Phase::Retired
                    ),
                }
        }
        Action::Unspecified => false,
    };
    let content_valid = if phase == Phase::Applied {
        response.content_sha256.len() == 32
    } else {
        response.content_sha256.is_empty()
            && response.content_bytes == 0
            && response.version.is_empty()
    };
    let refusal_valid = if phase == Phase::Refused {
        matches!(
            Refusal::from_i32(response.refusal),
            Some(Refusal::Changed | Refusal::Source | Refusal::Shared)
        )
    } else {
        response.refusal == Refusal::None as i32
    };
    anyhow::ensure!(
        correlated && content_valid && refusal_valid,
        "native sync response does not match the original request"
    );
    Ok(())
}

pub(super) struct Transport {
    outbound: mpsc::Sender<wire::CowboyBufferSyncEnvelope>,
    pending: Pending,
}

struct PendingGuard<'a> {
    transport: &'a Transport,
    id: u32,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.transport
            .pending
            .lock()
            .expect("native sync pending poisoned")
            .remove(&self.id);
    }
}

impl Transport {
    pub fn new() -> (Arc<Self>, mpsc::Receiver<wire::CowboyBufferSyncEnvelope>) {
        let (outbound, receiver) = mpsc::channel(32);
        (
            Arc::new(Self {
                outbound,
                pending: Arc::default(),
            }),
            receiver,
        )
    }

    pub fn stopped(&self) {
        self.pending
            .lock()
            .expect("native sync pending poisoned")
            .clear();
    }

    pub fn response(&self, id: u32, encoded: &[u8]) -> bool {
        let Some(sender) = self
            .pending
            .lock()
            .expect("native sync pending poisoned")
            .remove(&id)
        else {
            return false;
        };
        let response = wire::CowboyBufferSyncEnvelope::decode(encoded)
            .ok()
            .and_then(|value| {
                if value.responding_to != Some(id) {
                    return None;
                }
                match value.payload? {
                    Payload::Response(response) => Some(response),
                    Payload::Request(_) => None,
                }
            });
        let _ = sender.send(response);
        true
    }

    pub async fn request(
        &self,
        zed: &ZedRuntime,
        request: wire::CowboyBufferSync,
    ) -> Result<wire::CowboyBufferSyncResponse> {
        validate_request(&request)?;
        let id = zed
            .next_message_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| {
                (id > 0 && id < 1_000_000_000).then_some(id + 1)
            })
            .map_err(|_| anyhow::anyhow!("native request IDs exhausted"))?;
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().expect("native sync pending poisoned");
            anyhow::ensure!(
                pending.len() < 32 && !pending.contains_key(&id),
                "private native transport capacity reached"
            );
            pending.insert(id, sender);
        }
        let _guard = PendingGuard {
            transport: self,
            id,
        };
        let response = tokio::time::timeout(Duration::from_secs(30), async {
            self.outbound
                .send(wire::CowboyBufferSyncEnvelope {
                    id,
                    responding_to: None,
                    payload: Some(Payload::Request(request.clone())),
                })
                .await
                .context("native sync writer ended")?;
            receiver
                .await
                .context("native sync reader ended")?
                .context("native sync response unavailable")
        })
        .await
        .context("native sync observation timed out")??;
        validate_response(&request, &response)?;
        Ok(response)
    }

    pub async fn probe(&self, zed: &ZedRuntime) -> Result<[u8; 16]> {
        let reply = self
            .request(
                zed,
                wire::CowboyBufferSync {
                    protocol: 1,
                    action: Action::Probe as i32,
                    ..Default::default()
                },
            )
            .await?;
        reply
            .instance
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid native process instance"))
    }
}

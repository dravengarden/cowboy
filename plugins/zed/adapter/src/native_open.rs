//! A native open is one acquisition, including initial sharing and registration.
//! An unobserved result never authorizes close/reopen or a replacement owner.
use super::*;
use crate::sync_native::wire::{self, cowboy_buffer_sync_envelope::Payload};
use std::sync::atomic::{AtomicBool, AtomicU32};
use wire::cowboy_open_attempt_response::Outcome;

mod buffer;
mod recovery;
pub(super) use buffer::open_buffer_at;
pub(super) use recovery::{Admission, Recovery};

#[derive(Default)]
pub(super) struct Fence {
    latched: AtomicBool,
    /// Message ID of the latched attempt's `OpenBufferByPath`; 0 before dispatch.
    request: AtomicU32,
    receipt: std::sync::Mutex<Option<Arc<AtomicBool>>>,
}

// Deliberately no Drop-based reset: cancellation is uncertainty, not rollback.
// Only committing the original native owner, or `recover` learning the
// outcome from the same native process, may end this one-use attempt.
#[must_use = "commit only after retaining the original owner; dropping leaves uncertainty"]
pub(super) struct Attempt<'a>(&'a Fence);

impl Fence {
    pub(super) fn check(&self) -> Result<()> {
        anyhow::ensure!(
            !self.latched.load(Ordering::Acquire),
            "unresolved native open prevents admission"
        );
        Ok(())
    }

    pub(super) fn begin(&self) -> Result<Attempt<'_>> {
        self.latched
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| anyhow::anyhow!("unresolved native open prevents admission"))?;
        self.request.store(0, Ordering::Release);
        Ok(Attempt(self))
    }

    fn clear(&self) {
        if let Some(receipt) = self.receipt.lock().expect("open receipt poisoned").take() {
            receipt.store(true, Ordering::Release);
        }
        self.request.store(0, Ordering::Release);
        self.latched.store(false, Ordering::Release);
    }
}

impl Attempt<'_> {
    pub(super) fn track(&self, receipt: &Recovery) {
        let mut retained = self.0.receipt.lock().expect("open receipt poisoned");
        assert!(retained.is_none(), "one receipt per original attempt");
        receipt.0.store(false, Ordering::Release);
        *retained = Some(Arc::clone(&receipt.0));
    }
    /// Called with the request's message ID before it is queued.
    pub(super) fn dispatching(&self, message_id: u32) {
        self.0.request.store(message_id, Ordering::Release);
    }

    pub(super) fn complete(self) {
        self.0.clear();
    }
}

/// Resolve a latched open from the same native process, without replaying it.
/// An upstream round trip first proves the original request reached the
/// server, so `UNKNOWN` means it was never sent. A buffer it opened that no
/// owner holds is released through the confirmed native close.
/// Admission of one open: the process-wide fences, then reservations confined
/// to what this open could return. Without a native runtime every
/// reservation still applies.
pub(super) async fn admit(
    buffers: &BufferState,
    active: &HashMap<(PathBuf, PathBuf), BufferLease>,
    zed: Option<&Zed>,
    key: &(PathBuf, PathBuf),
    worktree_id: u64,
) -> Result<()> {
    let Some(zed) = zed else {
        return sync_owners::ensure_admission(buffers, active);
    };
    sync_owners::ensure_unfenced(buffers, active)?;
    if let Some(buffer) = active.get(key) {
        return sync_owners::ensure_readable(buffer);
    }
    ensure_target_unreserved(&sync_owners::reserved(active), zed, worktree_id, &key.1).await
}

/// Refuse an open only when its target could be a reserved native buffer:
/// the same server reports which existing buffer the open would return.
pub(super) async fn ensure_target_unreserved(
    reserved: &HashSet<u64>,
    zed: &ZedRuntime,
    worktree_id: u64,
    path: &Path,
) -> Result<()> {
    use wire::cowboy_open_target_response::State;
    if reserved.is_empty() {
        return Ok(());
    }
    let refused = || anyhow::anyhow!("the open target may alias a reserved native buffer");
    let query = Payload::OpenTargetRequest(wire::CowboyOpenTarget {
        project_id: proto::REMOTE_SERVER_PROJECT_ID,
        protocol: 1,
        worktree_id,
        path: path.to_string_lossy().into_owned(),
    });
    let Ok(Ok(Payload::OpenTargetResponse(reply))) =
        tokio::time::timeout(Duration::from_secs(5), zed.sync.exchange(zed, query)).await
    else {
        return Err(refused());
    };
    anyhow::ensure!(reply.protocol == 1, refused());
    if reply.state == State::NotOpen as i32
        || (reply.state == State::Open as i32
            && reply.buffer_id != 0
            && !reserved.contains(&reply.buffer_id))
    {
        Ok(())
    } else {
        Err(refused())
    }
}

pub(super) async fn recover(
    fence: &Fence,
    active: &HashMap<(PathBuf, PathBuf), BufferLease>,
    zed: &ZedRuntime,
) -> Result<()> {
    recover_observed(fence, active, zed, true).await
}

async fn recover_observed(
    fence: &Fence,
    active: &HashMap<(PathBuf, PathBuf), BufferLease>,
    zed: &ZedRuntime,
    allow_cleanup: bool,
) -> Result<()> {
    if !fence.latched.load(Ordering::Acquire) {
        return Ok(());
    }
    let message_id = fence.request.load(Ordering::Acquire);
    if message_id == 0 {
        // Latched before the request was queued: nothing reached the server.
        fence.clear();
        return Ok(());
    }
    let unresolved =
        |reason: &str| anyhow::anyhow!("unresolved native open prevents admission: {reason}");
    let barrier = zed
        .request(proto::envelope::Payload::Ping(proto::Ping {}))
        .await
        .map_err(|_| unresolved("server unreachable"))?;
    anyhow::ensure!(
        matches!(barrier.payload, Some(proto::envelope::Payload::Ack(_))),
        unresolved("server unreachable")
    );
    let query = Payload::OpenAttemptRequest(wire::CowboyOpenAttempt {
        project_id: proto::REMOTE_SERVER_PROJECT_ID,
        protocol: 1,
        message_id,
    });
    let Ok(Ok(Payload::OpenAttemptResponse(reply))) =
        tokio::time::timeout(Duration::from_secs(5), zed.sync.exchange(zed, query)).await
    else {
        return Err(unresolved("outcome unavailable"));
    };
    anyhow::ensure!(reply.protocol == 1, unresolved("outcome unavailable"));
    let outcome = reply.outcome;
    if outcome == Outcome::Opened as i32 {
        anyhow::ensure!(reply.buffer_id != 0, unresolved("invalid native buffer ID"));
        // Already held under another owner key: an alias, not an orphan.
        if !active
            .values()
            .any(|buffer| buffer.remote_id == reply.buffer_id)
        {
            if allow_cleanup {
                crate::native_close::release_unowned(zed, reply.buffer_id).await?;
            } else {
                anyhow::ensure!(
                    crate::native_close::observe_unowned(zed, reply.buffer_id).await?,
                    unresolved("orphan cleanup has not been dispatched")
                );
            }
        }
    } else if outcome == Outcome::Pending as i32 {
        return Err(unresolved("still pending"));
    } else if outcome != Outcome::Unknown as i32 && outcome != Outcome::Failed as i32 {
        return Err(unresolved("outcome unavailable"));
    }
    fence.clear();
    Ok(())
}

impl ZedRuntime {
    #[cfg(test)]
    pub(super) async fn open_buffer(
        &self,
        worktree_id: u64,
        path: &Path,
    ) -> Result<(u64, Vec<BufferVersionEntry>)> {
        self.open_buffer_tracked(worktree_id, path, |_| {}).await
    }

    pub(super) async fn open_buffer_tracked(
        &self,
        worktree_id: u64,
        path: &Path,
        dispatching: impl FnOnce(u32),
    ) -> Result<(u64, Vec<BufferVersionEntry>)> {
        // Subscribe before dispatch: the native share may precede the reply.
        let mut events = self.events.subscribe();
        let response = self
            .request_with(
                proto::envelope::Payload::OpenBufferByPath(proto::OpenBufferByPath {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    worktree_id,
                    path: path.to_string_lossy().into_owned(),
                }),
                dispatching,
            )
            .await?;
        let Some(proto::envelope::Payload::OpenBufferResponse(response)) = response.payload else {
            bail!("Zed returned the wrong OpenBufferByPath response");
        };
        let buffer_id = response.buffer_id;
        anyhow::ensure!(buffer_id != 0, "Zed returned an invalid native buffer ID");
        let version = tokio::time::timeout(Duration::from_secs(5), async {
            let mut version = HashMap::<u32, u32>::new();
            let mut received_state = false;
            loop {
                let envelope = events.recv().await?;
                let Some(proto::envelope::Payload::CreateBufferForPeer(message)) = envelope.payload
                else {
                    continue;
                };
                match message.variant {
                    Some(proto::create_buffer_for_peer::Variant::State(state))
                        if state.id == buffer_id =>
                    {
                        received_state = true;
                        merge_version(&mut version, state.saved_version);
                    }
                    Some(proto::create_buffer_for_peer::Variant::Chunk(chunk))
                        if chunk.buffer_id == buffer_id && received_state =>
                    {
                        for operation in chunk.operations {
                            merge_operation_version(&mut version, operation);
                        }
                        if chunk.is_last {
                            let mut version = version
                                .into_iter()
                                .map(|(replica_id, timestamp)| BufferVersionEntry {
                                    replica_id,
                                    timestamp,
                                })
                                .collect::<Vec<_>>();
                            version.sort_by_key(|entry| entry.replica_id);
                            break anyhow::Ok(version);
                        }
                    }
                    _ => {}
                }
            }
        })
        .await
        .context("Zed did not publish the initial buffer state")?
        .context("Zed initial buffer stream is unavailable")?;
        // Sharing is not registration. Failure here still belongs to this same
        // acquisition; neither stage retries or enqueues an implicit close.
        self.register_buffer(buffer_id).await?;
        Ok((buffer_id, version))
    }
}

#[cfg(test)]
pub(crate) mod tests;

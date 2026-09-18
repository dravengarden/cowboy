//! A native open is one acquisition, including initial sharing and registration.
//! An unobserved result never authorizes close/reopen or a replacement owner.
use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub(super) struct Fence(AtomicBool);

// Deliberately no Drop-based reset: cancellation is uncertainty, not rollback.
// Only committing the original native owner may consume this one-use attempt.
#[must_use = "commit only after retaining the original owner; dropping leaves uncertainty"]
pub(super) struct Attempt<'a>(&'a Fence);

impl Fence {
    pub(super) fn check(&self) -> Result<()> {
        anyhow::ensure!(
            !self.0.load(Ordering::Acquire),
            "unresolved native open prevents admission"
        );
        Ok(())
    }

    pub(super) fn begin(&self) -> Result<Attempt<'_>> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| anyhow::anyhow!("unresolved native open prevents admission"))?;
        Ok(Attempt(self))
    }
}

impl Attempt<'_> {
    pub(super) fn complete(self) {
        self.0.0.store(false, Ordering::Release);
    }
}

impl ZedRuntime {
    pub(super) async fn open_buffer(
        &self,
        worktree_id: u64,
        path: &Path,
    ) -> Result<(u64, Vec<BufferVersionEntry>)> {
        // Subscribe before dispatch: the native share may precede the reply.
        let mut events = self.events.subscribe();
        let response = self
            .request(proto::envelope::Payload::OpenBufferByPath(
                proto::OpenBufferByPath {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    worktree_id,
                    path: path.to_string_lossy().into_owned(),
                },
            ))
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

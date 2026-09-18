//! One-use local release, confirmed by the original native peer. Losing that
//! reply retains ownership/exclusion; neither Query nor Release resends Close.
use super::*;
use std::collections::BTreeSet;
use sync_native::wire::{self, cowboy_buffer_sync_envelope::Payload};
use wire::cowboy_close_buffers_response::Outcome;

type Key = (PathBuf, PathBuf);
type Active = HashMap<Key, BufferLease>;

// A probe observation, not authority, a durable identity or a replay ticket.
struct Instance([u8; 16]);

fn decode(
    request: &wire::CowboyCloseBuffers,
    response: wire::CowboyCloseBuffersResponse,
) -> Result<Instance> {
    anyhow::ensure!(
        response.protocol == 1 && response.instance.len() == 16,
        "invalid native close response"
    );
    let probe = request.instance.is_empty() && request.buffer_ids.is_empty();
    anyhow::ensure!(
        response.buffer_ids == request.buffer_ids
            && if probe {
                response.outcome == Outcome::Supported as i32
            } else {
                response.instance == request.instance && response.outcome == Outcome::Closed as i32
            },
        "original native peer close was not confirmed"
    );
    Ok(Instance(
        response.instance.try_into().expect("checked length"),
    ))
}

async fn exchange(zed: &ZedRuntime, request: wire::CowboyCloseBuffers) -> Result<Instance> {
    let response = zed
        .sync
        .exchange(zed, Payload::CloseRequest(request.clone()))
        .await?;
    let Payload::CloseResponse(response) = response else {
        bail!("unexpected native close response kind");
    };
    decode(&request, response)
}

fn request(instance: Vec<u8>, buffer_ids: Vec<u64>) -> wire::CowboyCloseBuffers {
    wire::CowboyCloseBuffers {
        project_id: proto::REMOTE_SERVER_PROJECT_ID,
        protocol: 1,
        instance,
        buffer_ids,
    }
}

// Non-cloneable, process-local plan derived under the caller's active write
// lock. That lock is retained across probe, the single effect and local commit.
struct Plan {
    keys: HashSet<Key>,
    native_ids: Vec<u64>,
}

impl Plan {
    fn checked(keys: HashSet<Key>, owner: &BufferOwner, active: &Active) -> Result<Self> {
        anyhow::ensure!(!keys.is_empty() && keys.len() <= 33, "invalid release set");
        let mut ids = BTreeSet::new();
        for key in &keys {
            let buffer = active.get(key).context("original buffer is not retained")?;
            anyhow::ensure!(
                matches!(owner, BufferOwner::Legacy(_)) || buffer.lease_ids.contains(owner),
                "original native buffer owner changed"
            );
            sync_owners::ensure_readable(buffer)?;
            ids.insert(buffer.remote_id);
        }
        let native_ids = ids
            .into_iter()
            .filter(|id| {
                active
                    .iter()
                    .filter(|(_, buffer)| buffer.remote_id == *id)
                    .all(|(key, buffer)| {
                        keys.contains(key) && buffer.lease_ids.iter().all(|other| other == owner)
                    })
            })
            .collect();
        Ok(Self { keys, native_ids })
    }

    async fn release(
        self,
        owner: &BufferOwner,
        active: &mut Active,
        zed: Option<&Zed>,
        on_admit: impl FnOnce(),
    ) -> Result<()> {
        if !self.native_ids.is_empty()
            && let Some(zed) = zed
        {
            anyhow::ensure!(self.native_ids[0] != 0, "invalid original native buffer ID");
            // Capability observation has no release effect. An unsupported pair
            // cannot fall back to the upstream unacknowledged CloseBuffer.
            let instance = exchange(zed, request(Vec::new(), Vec::new())).await?;
            for buffer in active.values_mut() {
                if self.native_ids.binary_search(&buffer.remote_id).is_ok() {
                    buffer.closing = true;
                }
            }
            on_admit();
            // Dropping this future never clears closing flags or original pins.
            exchange(zed, request(instance.0.to_vec(), self.native_ids.clone())).await?;
            // Receipt validation, mirror retirement, pin removal and ending
            // exclusion have no intervening await. Another close cannot reuse
            // this plan, and freeing a different buffer cannot clear its flags.
            let mut cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
            for id in &self.native_ids {
                cache.remove(*id);
            }
        }
        for key in &self.keys {
            let buffer = active
                .get_mut(key)
                .expect("original owner held through release");
            buffer.lease_ids.remove(owner);
            if buffer.lease_ids.is_empty() {
                active.remove(key);
            }
        }
        Ok(())
    }
}

pub(super) async fn release(
    keys: HashSet<Key>,
    owner: &BufferOwner,
    active: &mut Active,
    zed: Option<&Zed>,
    on_admit: impl FnOnce(),
) -> Result<()> {
    Plan::checked(keys, owner, active)?
        .release(owner, active, zed, on_admit)
        .await
}

#[cfg(test)]
pub(crate) mod connected;
#[cfg(test)]
pub(crate) mod tests;

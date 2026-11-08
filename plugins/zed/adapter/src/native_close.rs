//! One-use local release, confirmed by the original native peer. Losing that
//! reply retains ownership/exclusion; neither Query nor Release resends Close.
use super::*;
use anyhow::ensure;
use std::collections::{BTreeMap, BTreeSet};
use sync_native::wire::{self, cowboy_buffer_sync_envelope::Payload};
use wire::cowboy_close_buffers::Action;
use wire::cowboy_close_buffers_response::Outcome;

mod receipts;
use receipts::Owner;
pub(super) use receipts::Registry;

type Key = (PathBuf, PathBuf);
type Active = HashMap<Key, BufferLease>;

// A probe observation, not authority, a durable identity or a replay ticket.
struct Instance([u8; 16]);

fn decode(
    request: &wire::CowboyCloseBuffers,
    response: wire::CowboyCloseBuffersResponse,
) -> Result<Instance> {
    anyhow::ensure!(
        response.protocol == 2
            && response.instance.len() == 16
            && response.operation_id == request.operation_id,
        "invalid native close response"
    );
    let probe = request.instance.is_empty() && request.buffer_ids.is_empty();
    anyhow::ensure!(
        response.buffer_ids == request.buffer_ids
            && if probe {
                response.outcome == Outcome::Supported as i32
            } else {
                request.operation_id != 0
                    && response.instance == request.instance
                    && response.outcome == Outcome::Closed as i32
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
        protocol: 2,
        instance,
        buffer_ids,
        operation_id: 0,
        action: Action::Close as i32,
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
        let native_ids: Vec<_> = ids
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
        ensure!(
            native_ids.is_empty()
                || !active.values().any(|buffer| buffer
                    .lease_ids
                    .iter()
                    .any(|owner| matches!(owner, BufferOwner::NavigationPending(_)))),
            "navigation outcome must be recovered before removing shared native buffers"
        );
        Ok(Self { keys, native_ids })
    }

    async fn release(
        self,
        owner: &BufferOwner,
        active: &mut Active,
        zed: Option<&Zed>,
        open_fence: &native_open::Fence,
        on_admit: impl FnOnce(),
    ) -> Result<()> {
        if !self.native_ids.is_empty() {
            // An unresolved Open can return any existing native ID. Keep its
            // possible target alive until that original outcome is resolved.
            // Dropping a shared local owner without native Close remains safe.
            open_fence.check()?;
        }
        if !self.native_ids.is_empty()
            && let Some(zed) = zed
        {
            anyhow::ensure!(self.native_ids[0] != 0, "invalid original native buffer ID");
            // Capability observation has no release effect. An unsupported pair
            // cannot fall back to the upstream unacknowledged CloseBuffer.
            let instance = exchange(zed, request(Vec::new(), Vec::new())).await?;
            zed.closes.flush(zed);
            let key = Owner::buffer(owner, &self.keys);
            let receipt = zed.closes.begin(
                key.clone(),
                request(instance.0.to_vec(), self.native_ids.clone()),
                self.keys
                    .iter()
                    .map(|key| (key.clone(), active[key].remote_id))
                    .collect(),
            )?;
            for buffer in active.values_mut() {
                if self.native_ids.binary_search(&buffer.remote_id).is_ok() {
                    buffer.closing = true;
                }
            }
            on_admit();
            // Dropping this future never clears closing flags or original pins.
            exchange(zed, receipt.request).await?;
            // Receipt validation, mirror retirement, pin removal and ending
            // exclusion have no intervening await. Another close cannot reuse
            // this plan, and freeing a different buffer cannot clear its flags.
            let mut cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
            for id in &self.native_ids {
                cache.remove(*id);
            }
            zed.closes.confirm(&key, zed);
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

/// Confirmed release of a native buffer the peer holds without any owner,
/// such as one a recovered open acquired after its caller went away.
#[cfg(test)]
pub(super) async fn release_unowned(zed: &ZedRuntime, buffer_id: u64) -> Result<()> {
    release_unowned_with(zed, buffer_id, || {}).await
}

pub(super) async fn release_unowned_with(
    zed: &ZedRuntime,
    buffer_id: u64,
    dispatching: impl FnOnce(),
) -> Result<()> {
    let owner = Owner::Unowned(buffer_id);
    let request = if let Some(mut receipt) = zed.closes.get(&owner) {
        receipt.request.action = Action::Query as i32;
        receipt.request
    } else {
        let instance = exchange(zed, request(Vec::new(), Vec::new())).await?;
        zed.closes.flush(zed);
        zed.closes
            .begin(
                owner.clone(),
                request(instance.0.to_vec(), vec![buffer_id]),
                BTreeMap::default(),
            )?
            .request
    };
    dispatching();
    exchange(zed, request).await?;
    zed.diagnostics
        .lock()
        .expect("diagnostic cache poisoned")
        .remove(buffer_id);
    zed.closes.confirm(&owner, zed);
    Ok(())
}

/// Query the same process and operation only. An absent/failed receipt does
/// not permit starting an orphan's cleanup during a read-only observation.
pub(super) async fn observe_unowned(zed: &ZedRuntime, buffer_id: u64) -> Result<bool> {
    let owner = Owner::Unowned(buffer_id);
    let Some(mut receipt) = zed.closes.get(&owner) else {
        return Ok(false);
    };
    receipt.request.action = Action::Query as i32;
    exchange(zed, receipt.request).await?;
    zed.diagnostics
        .lock()
        .expect("diagnostic cache poisoned")
        .remove(buffer_id);
    zed.closes.confirm(&owner, zed);
    Ok(true)
}

/// Query the same process and operation only. An absent/failed receipt does
/// not authorize cleanup, a replacement owner, or another Close request.
pub(super) async fn recover(
    keys: &HashSet<Key>,
    owner: &BufferOwner,
    active: &mut Active,
    zed: &ZedRuntime,
) -> Result<bool> {
    let key = Owner::buffer(owner, keys);
    let Some(mut receipt) = zed.closes.get(&key) else {
        return Ok(false);
    };
    for (key, id) in &receipt.bindings {
        let buffer = active
            .get(key)
            .context("original close owner disappeared")?;
        ensure!(
            buffer.remote_id == *id && buffer.lease_ids.contains(owner),
            "original close owner changed"
        );
    }
    receipt.request.action = Action::Query as i32;
    exchange(zed, receipt.request.clone()).await?;
    let mut cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
    for id in &receipt.request.buffer_ids {
        cache.remove(*id);
    }
    for key in keys {
        let buffer = active
            .get_mut(key)
            .expect("original owner held through recovery");
        buffer.lease_ids.remove(owner);
        if buffer.lease_ids.is_empty() {
            active.remove(key);
        }
    }
    zed.closes.confirm(&key, zed);
    Ok(true)
}

pub(super) async fn release(
    keys: HashSet<Key>,
    owner: &BufferOwner,
    active: &mut Active,
    zed: Option<&Zed>,
    open_fence: &native_open::Fence,
    on_admit: impl FnOnce(),
) -> Result<()> {
    if let Some(zed) = zed {
        if recover(&keys, owner, active, zed).await? {
            return Ok(());
        }
        zed.closes.ensure_unreserved(
            &keys
                .iter()
                .filter_map(|key| active.get(key).map(|buffer| buffer.remote_id))
                .collect(),
        )?;
    }
    Plan::checked(keys, owner, active)?
        .release(owner, active, zed, open_fence, on_admit)
        .await
}

#[cfg(test)]
pub(crate) mod connected;
#[cfg(test)]
pub(crate) mod tests;

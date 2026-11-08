//! Internal automatic-acquisition owner: original handles, never path replay.
use super::*;

impl Registry {
    pub(crate) async fn open_acquisition(
        &mut self,
        lease: LeaseRef,
        capture: &crate::trust::Capture,
        worktrees: &Worktrees,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        let id = self.resolve(&lease)?;
        let slot = self
            .slots
            .get(&id)
            .context("original acquisition lease is absent")?;
        ensure!(
            matches!(&slot.origin, Origin::Path(incarnation) if capture.matches(incarnation)),
            "worktree changed between acquisition and buffer preparation"
        );
        // Open independently rechecks this same incarnation while holding the
        // worktree read lock across native I/O, so replacement cannot slip in.
        self.open(lease, worktrees, buffers, zed).await
    }

    pub(crate) async fn acquisition_result(
        &mut self,
        lease: &LeaseRef,
        buffers: &Buffers,
    ) -> Result<Response> {
        let id = self.resolve(lease)?;
        let slot = self
            .slots
            .get(&id)
            .context("original acquisition lease is absent")?;
        ensure!(
            slot.state == Phase::Open,
            "original acquisition is not open"
        );
        let active = buffers.active.read().await;
        let buffer = active
            .get(&(slot.worktree.clone(), slot.path.clone()))
            .filter(|buffer| buffer.lease_ids.contains(&BufferOwner::Owned(id)))
            .context("original acquisition owner changed")?;
        super::super::sync_owners::ensure_readable(buffer)?;
        Ok(Response::Buffer {
            api_version: 1,
            worktree: slot.worktree.clone(),
            path: slot.path.clone(),
            leases: buffer.lease_ids.len(),
            buffer_id: buffer.remote_id,
            version: buffer.version.clone(),
        })
    }

    /// Explicit cleanup may release an original orphan. Generic Unknown lease
    /// reads and releases keep their established observation-only contract.
    pub(crate) async fn release_acquisition(
        &mut self,
        lease: LeaseRef,
        buffers: &Buffers,
        zed: Option<&Zed>,
        dispatching: impl FnOnce(),
    ) -> Result<Response> {
        let id = self.resolve(&lease)?;
        let Some(slot) = self.slots.get_mut(&id) else {
            return Ok(reply(lease, LeaseState::Released));
        };
        match slot.state {
            Phase::Unknown => {
                let Some(recovery) = &slot.open_recovery else {
                    return self.query_recovered(lease, buffers, zed).await;
                };
                let active = buffers.active.write().await;
                ensure!(
                    recovery
                        .release(&buffers.native_open, &active, zed, dispatching)
                        .await?,
                    "original acquisition Open remains unconfirmed"
                );
            }
            Phase::Open => {
                super::super::close_buffer_at(
                    slot.worktree.clone(),
                    slot.path.clone(),
                    BufferOwner::Owned(id),
                    buffers,
                    zed,
                    || {
                        slot.state = Phase::Unknown;
                        dispatching();
                    },
                )
                .await?;
            }
            Phase::Prepared => {}
        }
        self.slots.remove(&id);
        Ok(reply(lease, LeaseState::Released))
    }
}

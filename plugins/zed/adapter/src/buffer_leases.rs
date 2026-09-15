//! Native buffer references, allocated before effects and never recycled. The
//! private socket's caller owns admission; a serialized reference is not a grant.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use super::{BufferOwner, Buffers, Response, WorktreeState, Worktrees, Zed};

const MAX_LEASES: usize = 1_024;
const MAX_PATH_BYTES: usize = 4_096;
const PREPARE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LeaseRef {
    instance: String,
    // Hex, not a JSON integer: browser decoders cannot preserve every u64.
    id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LeaseState {
    Prepared,
    Open,
    Released,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prepared,
    Open,
    Unknown,
}

impl From<Phase> for LeaseState {
    fn from(value: Phase) -> Self {
        match value {
            Phase::Prepared => Self::Prepared,
            Phase::Open => Self::Open,
            Phase::Unknown => Self::Unknown,
        }
    }
}

struct Slot {
    worktree: PathBuf,
    path: PathBuf,
    worktree_incarnation: Arc<()>,
    prepared_at: Instant,
    state: Phase,
}

#[derive(Default)]
pub(super) struct Registry {
    instance: Option<String>,
    last_id: u64,
    slots: HashMap<u64, Slot>,
}

impl Registry {
    fn expire_prepared(&mut self) {
        // Only effect-free reservations expire. Open/ambiguous native effects
        // stay retained; no LRU or timeout may pretend they have been released.
        self.slots.retain(|_, slot| {
            slot.state != Phase::Prepared || slot.prepared_at.elapsed() < PREPARE_TTL
        });
    }

    fn resolve(&mut self, lease: &LeaseRef) -> Result<u64> {
        self.expire_prepared();
        ensure!(
            self.instance.as_ref() == Some(&lease.instance),
            "buffer lease belongs to another adapter instance"
        );
        ensure!(
            lease.id.len() == 16
                && lease
                    .id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "invalid buffer lease reference"
        );
        let id = u64::from_str_radix(&lease.id, 16)?;
        ensure!(
            id != 0 && id <= self.last_id,
            "buffer lease was never prepared"
        );
        Ok(id)
    }

    pub(super) async fn prepare(
        &mut self,
        worktree: PathBuf,
        path: PathBuf,
        worktrees: &Worktrees,
    ) -> Result<Response> {
        self.expire_prepared();
        ensure!(
            self.slots.len() < MAX_LEASES,
            "native buffer lease capacity reached"
        );
        ensure!(worktree.is_absolute(), "buffer worktree must be absolute");
        ensure!(
            worktree.as_os_str().len() <= MAX_PATH_BYTES
                && path.as_os_str().len() <= MAX_PATH_BYTES,
            "buffer lease path exceeds byte limit"
        );
        let (worktree, path) = super::buffer_key(worktree, path).await?;
        ensure!(
            worktree.as_os_str().len() <= MAX_PATH_BYTES
                && path.as_os_str().len() <= MAX_PATH_BYTES,
            "resolved buffer lease path exceeds byte limit"
        );
        let all = worktrees.read().await;
        let current = all.get(&worktree).context("worktree is not open")?;
        ensure!(
            matches!(current.state, WorktreeState::Ready),
            "worktree is not ready"
        );
        if self.instance.is_none() {
            let mut bytes = [0_u8; 16];
            getrandom::fill(&mut bytes)
                .map_err(|_| anyhow::anyhow!("adapter identity unavailable"))?;
            self.instance = Some(format!("{:032x}", u128::from_be_bytes(bytes)));
        }
        let id = self
            .last_id
            .checked_add(1)
            .context("buffer lease sequence exhausted")?;
        self.slots.insert(
            id,
            Slot {
                worktree,
                path,
                worktree_incarnation: Arc::clone(&current.incarnation),
                prepared_at: Instant::now(),
                state: Phase::Prepared,
            },
        );
        self.last_id = id;
        Ok(reply(
            LeaseRef {
                instance: self.instance.clone().expect("identity was initialized"),
                id: format!("{id:016x}"),
            },
            LeaseState::Prepared,
        ))
    }

    pub(super) fn query(&mut self, lease: LeaseRef) -> Result<Response> {
        let id = self.resolve(&lease)?;
        // A missing *issued* ID was retired, not an unknown native effect. IDs
        // are never reused, so this needs neither unbounded tombstones nor ABA.
        let state = self
            .slots
            .get(&id)
            .map_or(LeaseState::Released, |slot| slot.state.into());
        Ok(reply(lease, state))
    }

    pub(super) async fn open(
        &mut self,
        lease: LeaseRef,
        worktrees: &Worktrees,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        let id = self.resolve(&lease)?;
        let slot = self
            .slots
            .get_mut(&id)
            .context("buffer lease has been released")?;
        if slot.state != Phase::Prepared {
            return Ok(reply(lease, slot.state.into()));
        }
        // Revalidate the captured target, never adopt a replacement worktree
        // or follow a changed symlink. Keep its incarnation alive through I/O.
        let key = super::buffer_key(slot.worktree.clone(), slot.path.clone()).await?;
        ensure!(
            key == (slot.worktree.clone(), slot.path.clone()),
            "buffer target changed"
        );
        let all = worktrees.read().await;
        let current = all
            .get(&slot.worktree)
            .context("worktree is no longer open")?;
        ensure!(
            Arc::ptr_eq(&current.incarnation, &slot.worktree_incarnation)
                && matches!(current.state, WorktreeState::Ready),
            "buffer worktree incarnation changed"
        );
        ensure!(
            slot.prepared_at.elapsed() < PREPARE_TTL,
            "buffer preparation expired"
        );
        open_prevalidated(slot, id, current.remote_id, buffers, zed).await?;
        Ok(reply(lease, LeaseState::Open))
    }

    pub(super) async fn release(
        &mut self,
        lease: LeaseRef,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        let id = self.resolve(&lease)?;
        let Some(slot) = self.slots.get(&id) else {
            return Ok(reply(lease, LeaseState::Released));
        };
        match slot.state {
            Phase::Unknown => return Ok(reply(lease, LeaseState::Unknown)),
            Phase::Open => {
                super::close_buffer_at(
                    slot.worktree.clone(),
                    slot.path.clone(),
                    BufferOwner::Owned(id),
                    buffers,
                    zed,
                )
                .await?;
            }
            Phase::Prepared => {}
        }
        self.slots.remove(&id);
        Ok(reply(lease, LeaseState::Released))
    }
}

async fn open_prevalidated(
    slot: &mut Slot,
    id: u64,
    worktree_id: u64,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<()> {
    // Cancellation, timeout or a native error after this point must not replay
    // open or claim successful release without a native buffer ID. The caller
    // holds the checked worktree incarnation's read lock through this step.
    slot.state = Phase::Unknown;
    super::open_buffer_at(
        slot.worktree.clone(),
        slot.path.clone(),
        worktree_id,
        BufferOwner::Owned(id),
        buffers,
        zed,
    )
    .await?;
    slot.state = Phase::Open;
    Ok(())
}

fn reply(lease: LeaseRef, state: LeaseState) -> Response {
    Response::BufferLease {
        api_version: 1,
        lease,
        state,
    }
}

#[cfg(test)]
mod tests;

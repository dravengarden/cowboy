//! Cleanup of one captured worktree, without canonicalizing a replacement path.
use super::lifecycle::{self, Change};
use crate::{Buffers, WorktreeLease, Worktrees, Zed};
use anyhow::{Context as _, Result, ensure};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone)]
pub(crate) struct Capture {
    path: PathBuf,
    incarnation: Arc<()>,
    retired: Arc<AtomicBool>,
}

impl Capture {
    pub(super) fn new(path: PathBuf, lease: &WorktreeLease) -> Self {
        Self {
            path,
            incarnation: lease.incarnation.clone(),
            retired: lease.retired.clone(),
        }
    }

    fn check(&self, lease: &WorktreeLease) -> Result<()> {
        ensure!(
            Arc::ptr_eq(&self.incarnation, &lease.incarnation),
            "original worktree incarnation changed"
        );
        Ok(())
    }

    pub(crate) fn matches(&self, incarnation: &Arc<()>) -> bool {
        !self.retired.load(Ordering::Acquire) && Arc::ptr_eq(&self.incarnation, incarnation)
    }

    /// True means this original worktree was positively retired, not just absent.
    pub(crate) async fn observe(&self, worktrees: &Worktrees, zed: Option<&Zed>) -> Result<bool> {
        if self.retired.load(Ordering::Acquire) {
            return Ok(true);
        }
        let mut all = worktrees.write().await;
        if self.retired.load(Ordering::Acquire) {
            return Ok(true);
        }
        let lease = all
            .get_mut(&self.path)
            .context("original worktree is unavailable")?;
        self.check(lease)?;
        if let Some(zed) = zed {
            lifecycle::observe(lease, zed).await?;
        }
        if self.retired.load(Ordering::Acquire) {
            all.remove(&self.path);
            return Ok(true);
        }
        Ok(false)
    }

    /// Release only when no interactive or buffer owner still uses this tree.
    /// `dispatching` runs before any possible Remove request and must be retained.
    pub(crate) async fn release(
        &self,
        worktrees: &Worktrees,
        buffers: &Buffers,
        zed: Option<&Zed>,
        dispatching: impl FnOnce(),
    ) -> Result<()> {
        if self.observe(worktrees, zed).await? {
            return Ok(());
        }
        let mut all = worktrees.write().await;
        if self.retired.load(Ordering::Acquire) {
            return Ok(());
        }
        let lease = all
            .get_mut(&self.path)
            .context("original worktree is unavailable")?;
        self.check(lease)?;
        ensure!(
            lease.pending.is_none(),
            "original worktree transition remains unconfirmed"
        );
        let active = buffers.active.read().await;
        if lease.leases != 0 || active.keys().any(|(root, _)| root == &self.path) {
            // This acquisition owns no worktree lease. Other owners retain it.
            return Ok(());
        }
        crate::sync_owners::ensure_unfenced(buffers, &active)?;
        if let Some(zed) = zed {
            dispatching();
            lifecycle::apply(lease, Change::Remove, zed).await?;
        }
        lease.retired.store(true, Ordering::Release);
        all.remove(&self.path);
        Ok(())
    }
}

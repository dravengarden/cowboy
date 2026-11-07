//! Worktree trust: granted and revoked by the owner through Cowboy.
//!
//! At the pinned Zed revision `RestrictWorktrees` only records a restriction:
//! the LSP store reacts to `Trusted` alone, so servers started while trusted
//! keep running. Revocation therefore also removes the worktree from the native
//! project, which drops the language servers rooted only there, and adds it
//! back restricted. That is only safe without live leases or buffers; until
//! then the worktree is restricted and every language query on it is refused,
//! and the next ensure after the last owner leaves completes the removal.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};

use super::{ADAPTER_VERSION, Buffers, Response, WorktreeLease, WorktreeState, Worktrees, Zed};

mod capture;
pub(crate) mod lifecycle;
pub(crate) use capture::Capture;
use lifecycle::Change;
use std::sync::atomic::Ordering;

pub(super) async fn ensure_worktree(
    path: PathBuf,
    trusted: bool,
    acquire_lease: bool,
    worktrees: &Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    ensure_captured(
        path,
        trusted,
        acquire_lease,
        worktrees,
        buffers,
        zed,
        &mut None,
    )
    .await
}

pub(crate) async fn ensure_captured(
    path: PathBuf,
    trusted: bool,
    acquire_lease: bool,
    worktrees: &Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
    captured: &mut Option<Capture>,
) -> Result<Response> {
    let path = tokio::fs::canonicalize(path).await?;
    if !path.is_dir() {
        bail!("worktree is not a directory");
    }
    let mut all = worktrees.write().await;
    if let Some(lease) = all.get_mut(&path) {
        *captured = Some(Capture::new(path.clone(), lease));
        if lease.pending.is_some()
            && let Some(zed) = zed
        {
            lifecycle::observe(lease, zed).await?;
        }
        anyhow::ensure!(
            lease.pending.is_none(),
            "original worktree transition remains unconfirmed"
        );
        if lease.retired.load(Ordering::Acquire) {
            all.remove(&path);
        }
    }
    if !trusted
        && all
            .get(&path)
            .is_some_and(|lease| lease.trusted_once && lease.leases == 0)
        && !has_buffers(buffers, &path).await
    {
        // Drop the native worktree (and its servers) before adding it back.
        let removed = all.get_mut(&path).expect("checked above");
        *captured = Some(Capture::new(path.clone(), removed));
        if let Some(zed) = zed {
            let active = buffers.active.read().await;
            anyhow::ensure!(
                !active.keys().any(|(root, _)| root == &path),
                "worktree acquired another owner before revocation"
            );
            crate::sync_owners::ensure_unfenced(buffers, &active)?;
            lifecycle::apply(removed, Change::Remove, zed).await?;
        }
        removed.retired.store(true, Ordering::Release);
        all.remove(&path);
    }
    if !all.contains_key(&path) {
        if let Some(zed) = zed {
            crate::worktree_recovery::support(zed).await?;
        }
        let remote_id = if zed.is_none() {
            u64::try_from(all.len() + 1)?
        } else {
            0
        };
        all.insert(path.clone(), placeholder(remote_id));
        *captured = Some(Capture::new(path.clone(), &all[&path]));
        if let Some(zed) = zed {
            lifecycle::apply(all.get_mut(&path).unwrap(), Change::Add(path.clone()), zed).await?;
            // A new native worktree needs an explicit trust decision, including
            // restriction; a local placeholder is not native trust evidence.
            lifecycle::apply(all.get_mut(&path).unwrap(), Change::Trust(trusted), zed).await?;
        } else {
            let lease = all.get_mut(&path).unwrap();
            lease.state = if trusted {
                WorktreeState::Warming
            } else {
                WorktreeState::Restricted
            };
            lease.trusted_once = trusted;
        }
    }
    let lease = all.get_mut(&path).expect("worktree was just inserted");
    *captured = Some(Capture::new(path.clone(), lease));
    match (trusted, lease.state) {
        (true, WorktreeState::Restricted) => {
            if let Some(zed) = zed {
                lifecycle::apply(lease, Change::Trust(true), zed).await?;
            } else {
                lease.state = WorktreeState::Warming;
            }
            lease.trusted_once = true;
        }
        (false, WorktreeState::Warming | WorktreeState::Ready) => {
            // Owners remain: restrict now, remove once they have left.
            if let Some(zed) = zed {
                lifecycle::apply(lease, Change::Trust(false), zed).await?;
            } else {
                lease.state = WorktreeState::Restricted;
            }
        }
        _ => {}
    }
    if acquire_lease {
        lease.leases += 1;
    }
    Ok(Response::Worktree {
        api_version: ADAPTER_VERSION,
        path,
        state: lease.state,
        leases: lease.leases,
    })
}

fn placeholder(remote_id: u64) -> WorktreeLease {
    WorktreeLease {
        state: WorktreeState::Restricted,
        leases: 0,
        remote_id,
        incarnation: Arc::default(),
        trusted_once: false,
        pending: None,
        retired: Arc::default(),
    }
}

async fn has_buffers(buffers: &Buffers, root: &Path) -> bool {
    buffers
        .active
        .read()
        .await
        .keys()
        .any(|(worktree, _)| worktree == root)
}

/// Language queries need a trusted worktree. Unknown worktrees fall through to
/// the query's own "not open" refusal.
pub(super) async fn ensure_language(worktree: &Path, worktrees: &Worktrees) -> Result<()> {
    let Ok(worktree) = tokio::fs::canonicalize(worktree).await else {
        return Ok(());
    };
    if worktrees.read().await.get(&worktree).is_some_and(|lease| {
        lease.pending.is_some()
            || lease.retired.load(Ordering::Acquire)
            || matches!(lease.state, WorktreeState::Restricted)
    }) {
        bail!("worktree is restricted");
    }
    Ok(())
}

#[cfg(test)]
mod tests;

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

pub(super) async fn ensure_worktree(
    path: PathBuf,
    trusted: bool,
    acquire_lease: bool,
    worktrees: &Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    let path = tokio::fs::canonicalize(path).await?;
    if !path.is_dir() {
        bail!("worktree is not a directory");
    }
    let mut all = worktrees.write().await;
    if !trusted
        && all
            .get(&path)
            .is_some_and(|lease| lease.trusted_once && lease.leases == 0)
        && !has_buffers(buffers, &path).await
    {
        // Drop the native worktree (and its servers) before adding it back.
        let removed = all.remove(&path).expect("checked above");
        if let Some(zed) = zed {
            zed.remove_worktree(removed.remote_id)?;
        }
    }
    if !all.contains_key(&path) {
        let (remote_id, state) = if let Some(zed) = zed {
            zed.open_worktree(&path, trusted).await?
        } else {
            (
                u64::try_from(all.len() + 1)?,
                if trusted {
                    WorktreeState::Warming
                } else {
                    WorktreeState::Restricted
                },
            )
        };
        all.insert(
            path.clone(),
            WorktreeLease {
                state,
                leases: 0,
                remote_id,
                incarnation: Arc::default(),
                trusted_once: trusted,
            },
        );
    }
    let lease = all.get_mut(&path).expect("worktree was just inserted");
    if acquire_lease {
        lease.leases += 1;
    }
    match (trusted, lease.state) {
        (true, WorktreeState::Restricted) => {
            lease.state = if let Some(zed) = zed {
                zed.set_trust(lease.remote_id, true).await?
            } else {
                WorktreeState::Warming
            };
            lease.trusted_once = true;
        }
        (false, WorktreeState::Warming | WorktreeState::Ready) => {
            // Owners remain: restrict now, remove once they have left.
            lease.state = if let Some(zed) = zed {
                zed.set_trust(lease.remote_id, false).await?
            } else {
                WorktreeState::Restricted
            };
        }
        _ => {}
    }
    Ok(Response::Worktree {
        api_version: ADAPTER_VERSION,
        path,
        state: lease.state,
        leases: lease.leases,
    })
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
    if worktrees
        .read()
        .await
        .get(&worktree)
        .is_some_and(|lease| matches!(lease.state, WorktreeState::Restricted))
    {
        bail!("worktree is restricted");
    }
    Ok(())
}

#[cfg(test)]
mod tests;

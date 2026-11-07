//! Retain the original native transition until its own outcome is confirmed.
use crate::worktree_recovery::{Kind, Observation, Operation};
use crate::{WorktreeLease, WorktreeState, Zed};
use anyhow::{Result, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};
use tokio::sync::Mutex;

pub(crate) enum Change {
    Add(PathBuf),
    Trust(bool),
    Remove,
}

pub(crate) struct Pending {
    change: Change,
    operation: Operation,
}

pub(crate) async fn apply(lease: &mut WorktreeLease, change: Change, zed: &Zed) -> Result<()> {
    ensure!(
        lease.pending.is_none() && !lease.retired.load(Ordering::Acquire),
        "original worktree transition remains unresolved"
    );
    let (kind, target, payload) = match &change {
        Change::Add(path) => (
            Kind::Add,
            None,
            proto::envelope::Payload::AddWorktree(proto::AddWorktree {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                path: path.to_string_lossy().into_owned(),
                visible: true,
            }),
        ),
        Change::Trust(true) => (
            Kind::Trust,
            Some(lease.remote_id),
            proto::envelope::Payload::TrustWorktrees(proto::TrustWorktrees {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                trusted_paths: vec![proto::PathTrust {
                    content: Some(proto::path_trust::Content::WorktreeId(lease.remote_id)),
                }],
            }),
        ),
        Change::Trust(false) => (
            Kind::Restrict,
            Some(lease.remote_id),
            proto::envelope::Payload::RestrictWorktrees(proto::RestrictWorktrees {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                worktree_ids: vec![lease.remote_id],
            }),
        ),
        Change::Remove => (
            Kind::Remove,
            Some(lease.remote_id),
            proto::envelope::Payload::RemoveWorktree(proto::RemoveWorktree {
                worktree_id: lease.remote_id,
            }),
        ),
    };
    let pending = Arc::new(Mutex::new(Pending {
        change,
        operation: Operation::new(kind, target)?,
    }));
    lease.pending = Some(Arc::clone(&pending));
    let mut transition = pending.lock().await;
    let observed = transition.operation.execute(zed, payload).await?;
    settle(lease, &transition.change, observed, zed).await?;
    ensure!(
        matches!(observed, Observation::Completed(_)),
        "original worktree effect was not admitted"
    );
    Ok(())
}

/// Read-only native observation. Local state commits only the original result.
pub(crate) async fn observe(lease: &mut WorktreeLease, zed: &Zed) -> Result<()> {
    let Some(pending) = lease.pending.clone() else {
        return Ok(());
    };
    let transition = pending.lock().await;
    let observed = transition.operation.observe(zed).await?;
    settle(lease, &transition.change, observed, zed).await
}

async fn settle(
    lease: &mut WorktreeLease,
    change: &Change,
    observed: Observation,
    zed: &Zed,
) -> Result<()> {
    match observed {
        Observation::Completed(id) => match change {
            Change::Add(path) => {
                zed.worktree_paths.write().await.insert(id, path.clone());
                lease.remote_id = id;
            }
            Change::Trust(trusted) => {
                lease.state = if *trusted {
                    WorktreeState::Ready
                } else {
                    WorktreeState::Restricted
                };
                lease.trusted_once |= *trusted;
            }
            Change::Remove => {
                zed.worktree_paths.write().await.remove(&id);
                lease.retired.store(true, Ordering::Release);
            }
        },
        Observation::NoEffect => {
            if matches!(change, Change::Add(_)) {
                lease.retired.store(true, Ordering::Release);
            }
        }
        Observation::Pending | Observation::Unknown => {
            anyhow::bail!("original worktree result remains unconfirmed")
        }
    }
    lease.pending = None;
    Ok(())
}

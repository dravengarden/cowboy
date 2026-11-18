//! Account fence dispositions. The Controller orders its stop and admit
//! requests by the account control's revision; the daemon persists the last
//! applied disposition with the account and refuses an older one under the
//! account lock, so a delayed admit can never undo a newer confirmed stop.
//!
//! A fence of an account the daemon has not created (or has removed) is kept
//! as a root-held tombstone `<state_root>/fences/<key>.json` that
//! `ensure_account` adopts, so a stop before the first login still fences.

use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::protocol::{Code, Outcome, fail};
use crate::safefs;

pub const FENCES_DIR: &str = "fences";
const MAX_TOMBSTONE_BYTES: u64 = 4096;
const TOMBSTONE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceAction {
    Stop,
    Admit,
}

/// The last revisioned fence operation applied to an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disposition {
    pub revision: u64,
    pub operation_id: String,
    pub action: FenceAction,
}

/// One requested fence operation. `revision` and `operation_id` are either
/// both present (a Controller account control) or both absent.
#[derive(Debug, Clone, Copy)]
pub struct FenceOp<'a> {
    pub action: FenceAction,
    pub revision: Option<u64>,
    pub operation_id: Option<&'a str>,
}

/// Whether an admitted operation is new or repeats the applied one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Apply,
    Replay,
}

impl FenceOp<'_> {
    /// Order this operation against the applied `current` disposition.
    pub fn admit(&self, current: Option<&Disposition>) -> Outcome<Admission> {
        let (Some(revision), Some(operation_id)) = (self.revision, self.operation_id) else {
            // An unrevisioned stop only tightens; an unrevisioned admit must
            // not undo a fence a revisioned control owns.
            return match (self.action, current) {
                (FenceAction::Admit, Some(current)) => Err(fail(
                    Code::Conflict,
                    format!(
                        "the account fence is owned by control revision {}; admit needs a revision",
                        current.revision
                    ),
                )),
                _ => Ok(Admission::Apply),
            };
        };
        let Some(current) = current else {
            return Ok(Admission::Apply);
        };
        if revision < current.revision {
            return Err(fail(
                Code::Conflict,
                format!(
                    "stale fence operation: revision {revision} is older than the applied revision {}",
                    current.revision
                ),
            ));
        }
        if revision > current.revision {
            return Ok(Admission::Apply);
        }
        if current.operation_id == operation_id && current.action == self.action {
            Ok(Admission::Replay)
        } else {
            Err(fail(
                Code::Conflict,
                format!("revision {revision} already applied another fence operation"),
            ))
        }
    }

    /// The disposition this operation records, if it is revisioned.
    #[must_use]
    pub fn disposition(&self) -> Option<Disposition> {
        Some(Disposition {
            revision: self.revision?,
            operation_id: self.operation_id?.to_owned(),
            action: self.action,
        })
    }
}

/// The fence state of an account without metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tombstone {
    pub schema: u32,
    pub fence_epoch: u64,
    pub fenced: Option<u64>,
    pub disposition: Option<Disposition>,
    /// The `remove_account` that removed the account, kept so a retry of the
    /// same operation is answered as its replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removal: Option<Removal>,
}

/// A completed or started removal of an account's home and system user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Removal {
    pub operation_id: Option<String>,
    pub user: String,
}

fn file_name(key: &str) -> String {
    format!("{key}.json")
}

fn open_dir(root: BorrowedFd<'_>) -> Result<OwnedFd> {
    safefs::ensure_dir(root, FENCES_DIR, 0o700, crate::daemon::root_owner())
}

/// The account's tombstone; `None` when it has none.
pub fn load(root: BorrowedFd<'_>, key: &str) -> Result<Option<Tombstone>> {
    let dir = open_dir(root)?;
    let Some(bytes) = safefs::read_file(dir.as_fd(), &file_name(key), MAX_TOMBSTONE_BYTES)? else {
        return Ok(None);
    };
    let tombstone: Tombstone = serde_json::from_slice(&bytes).context("parse fence tombstone")?;
    anyhow::ensure!(
        tombstone.schema == TOMBSTONE_SCHEMA,
        "unsupported fence tombstone schema {}",
        tombstone.schema
    );
    Ok(Some(tombstone))
}

/// Durably persist the account's tombstone.
pub fn save(root: BorrowedFd<'_>, key: &str, tombstone: &Tombstone) -> Result<()> {
    let dir = open_dir(root)?;
    let mut tombstone = tombstone.clone();
    tombstone.schema = TOMBSTONE_SCHEMA;
    let mut bytes = serde_json::to_vec(&tombstone)?;
    bytes.push(b'\n');
    safefs::write_atomic(dir.as_fd(), &file_name(key), &bytes, 0o600)
}

/// Remove the account's tombstone after its metadata adopted it.
pub fn remove(root: BorrowedFd<'_>, key: &str) -> Result<()> {
    let dir = open_dir(root)?;
    safefs::remove_file(dir.as_fd(), &file_name(key))?;
    rustix::fs::fsync(&dir).context("fsync fences")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(action: FenceAction, revision: u64, id: &str) -> FenceOp<'_> {
        FenceOp {
            action,
            revision: Some(revision),
            operation_id: Some(id),
        }
    }

    #[test]
    fn revisions_order_fence_operations() {
        let stop = op(FenceAction::Stop, 5, "s5").disposition().unwrap();
        // A delayed admit of an older revision cannot undo the newer stop.
        let stale = op(FenceAction::Admit, 4, "a4").admit(Some(&stop));
        assert_eq!(stale.unwrap_err().code, Code::Conflict);
        assert_eq!(
            op(FenceAction::Stop, 5, "s5").admit(Some(&stop)).unwrap(),
            Admission::Replay
        );
        assert_eq!(
            op(FenceAction::Admit, 5, "a5")
                .admit(Some(&stop))
                .unwrap_err()
                .code,
            Code::Conflict
        );
        assert_eq!(
            op(FenceAction::Admit, 6, "a6").admit(Some(&stop)).unwrap(),
            Admission::Apply
        );
        let unrevisioned = |action| FenceOp {
            action,
            revision: None,
            operation_id: None,
        };
        assert_eq!(
            unrevisioned(FenceAction::Stop).admit(Some(&stop)).unwrap(),
            Admission::Apply
        );
        assert_eq!(
            unrevisioned(FenceAction::Admit)
                .admit(Some(&stop))
                .unwrap_err()
                .code,
            Code::Conflict
        );
        assert_eq!(
            unrevisioned(FenceAction::Admit).admit(None).unwrap(),
            Admission::Apply
        );
    }

    #[test]
    fn tombstones_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let root = safefs::open_root(temp.path()).unwrap();
        assert!(load(root.as_fd(), "0123456789abcdef").unwrap().is_none());
        let tombstone = Tombstone {
            schema: 0,
            fence_epoch: 2,
            fenced: Some(2),
            disposition: op(FenceAction::Stop, 1, "s1").disposition(),
            removal: None,
        };
        save(root.as_fd(), "0123456789abcdef", &tombstone).unwrap();
        let loaded = load(root.as_fd(), "0123456789abcdef").unwrap().unwrap();
        assert_eq!((loaded.fence_epoch, loaded.fenced), (2, Some(2)));
        remove(root.as_fd(), "0123456789abcdef").unwrap();
        assert!(load(root.as_fd(), "0123456789abcdef").unwrap().is_none());
    }
}

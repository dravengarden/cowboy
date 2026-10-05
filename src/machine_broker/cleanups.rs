//! Advisory continuation records for terminal-Session artifact cleanup.
//!
//! A record nominates one original worktree root object for a Session whose
//! terminal deletion is already committed in the deletion journal. It carries no
//! target list, path or deletion authority: resuming re-observes the root, then
//! scans for Cargo targets from scratch. Losing, ignoring or never writing a
//! record leaves exactly the pre-existing behaviour (artifacts are preserved),
//! so older Machines and reader-only builds need no compatibility floor for it.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use super::deletions::Owner;
use crate::session_workspace::RootIdentity;

const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const FILE: &str = "cleanups.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    session_id: String,
    root: RootIdentity,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u16,
    owner: Owner,
    pending: Vec<Entry>,
}

pub(super) struct Store {
    root: PathBuf,
    root_handle: File,
    lock: File,
    owner: Owner,
    pending: BTreeMap<String, RootIdentity>,
    poisoned: bool,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.contains('\0')
}

impl Store {
    /// Only a deletion-writer-admitted Machine opens this namespace; callers
    /// must treat any error as "no durable continuation", never as fatal.
    pub(super) fn open(path: &Path, owner: Owner) -> Result<Self> {
        ensure!(
            valid_id(&owner.machine_id),
            "invalid cleanup Machine identity"
        );
        ensure!(
            owner.service_id.as_deref().is_none_or(valid_id),
            "invalid cleanup Service identity"
        );
        match std::fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let root = std::path::absolute(path)?;
        let root_handle = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&root)
            .context("opening cleanup continuation directory without following links")?;
        File::open(root.parent().context("cleanup namespace has no parent")?)?.sync_all()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join(".lock"))?;
        ensure!(
            lock.metadata()?.is_file(),
            "cleanup continuation lock is not a regular file"
        );
        fs2::FileExt::try_lock_exclusive(&lock).context("cleanup continuations already owned")?;
        let mut pending = BTreeMap::new();
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join(FILE))
        {
            Ok(file) => {
                ensure!(
                    file.metadata()?.is_file(),
                    "cleanup continuation record is not a regular file"
                );
                let mut bytes = Vec::new();
                file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= MAX_BYTES,
                    "cleanup continuation record exceeds byte limit"
                );
                let record: Record =
                    serde_json::from_slice(&bytes).context("invalid cleanup continuation")?;
                ensure!(
                    record.schema == 1,
                    "unsupported cleanup continuation schema"
                );
                ensure!(
                    record.owner == owner,
                    "cleanup continuation belongs to another Machine or Service"
                );
                ensure!(
                    record.pending.len() <= MAX_RECORDS,
                    "cleanup continuation exceeds record limit"
                );
                for entry in record.pending {
                    ensure!(
                        valid_id(&entry.session_id)
                            && pending.insert(entry.session_id, entry.root).is_none(),
                        "invalid or duplicate cleanup continuation identity"
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let store = Self {
            root,
            root_handle,
            lock,
            owner,
            pending,
            poisoned: false,
        };
        store.check()?;
        Ok(store)
    }

    pub(super) fn pending(&self) -> Vec<(String, RootIdentity)> {
        self.pending
            .iter()
            .map(|(id, root)| (id.clone(), *root))
            .collect()
    }

    fn check(&self) -> Result<()> {
        ensure!(
            !self.poisoned,
            "cleanup continuation writer is fenced after a storage failure"
        );
        let root = std::fs::symlink_metadata(&self.root)?;
        let held = self.root_handle.metadata()?;
        ensure!(
            root.is_dir() && root.dev() == held.dev() && root.ino() == held.ino(),
            "cleanup continuation directory was replaced"
        );
        let lock = std::fs::symlink_metadata(self.root.join(".lock"))?;
        let held = self.lock.metadata()?;
        ensure!(
            lock.is_file() && lock.dev() == held.dev() && lock.ino() == held.ino(),
            "cleanup continuation lock was replaced"
        );
        Ok(())
    }

    /// Nominate `session_id`'s root. An existing nomination is kept: the first
    /// observed object stays the only one a resume may accept.
    pub(super) fn record(&mut self, session_id: &str, root: RootIdentity) -> Result<()> {
        ensure!(valid_id(session_id), "invalid terminal Session identity");
        if self.pending.contains_key(session_id) {
            return Ok(());
        }
        ensure!(
            self.pending.len() < MAX_RECORDS,
            "cleanup continuation record budget exhausted"
        );
        let mut next = self.pending.clone();
        next.insert(session_id.to_owned(), root);
        self.commit(next)
    }

    /// Withdraw a nomination after cleanup completed or its root was retired.
    pub(super) fn retire(&mut self, session_id: &str) -> Result<()> {
        if !self.pending.contains_key(session_id) {
            return Ok(());
        }
        let mut next = self.pending.clone();
        next.remove(session_id);
        self.commit(next)
    }

    fn commit(&mut self, next: BTreeMap<String, RootIdentity>) -> Result<()> {
        self.check()?;
        let bytes = serde_json::to_vec(&Record {
            schema: 1,
            owner: self.owner.clone(),
            pending: next
                .iter()
                .map(|(session_id, root)| Entry {
                    session_id: session_id.clone(),
                    root: *root,
                })
                .collect(),
        })?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "cleanup continuation byte budget exhausted"
        );
        let pending = self
            .root
            .join(format!(".pending-{:032x}", rand::random::<u128>()));
        let result: Result<()> = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&pending)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            self.check()?;
            std::fs::rename(&pending, self.root.join(FILE))?;
            self.root_handle.sync_all()?;
            self.check()?;
            Ok(())
        })();
        if let Err(error) = result {
            self.poisoned = true;
            return Err(error.context("cleanup continuation update was not confirmed"));
        }
        self.pending = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> Owner {
        Owner {
            machine_id: "fixture-machine".into(),
            service_id: Some("fixture-service".into()),
        }
    }

    fn identity(ino: u64) -> RootIdentity {
        RootIdentity {
            dev: 7,
            ino,
            birth_secs: 1_700_000_000,
            birth_nanos: 42,
        }
    }

    #[test]
    fn nominations_survive_reopen_and_retire_individually() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path(), owner()).unwrap();
        store.record("sess-1", identity(1)).unwrap();
        store.record("sess-2", identity(2)).unwrap();
        // The first observed object stays the only admissible one.
        store.record("sess-1", identity(99)).unwrap();
        drop(store);
        let mut store = Store::open(root.path(), owner()).unwrap();
        assert_eq!(
            store.pending(),
            vec![
                ("sess-1".into(), identity(1)),
                ("sess-2".into(), identity(2))
            ]
        );
        store.retire("sess-1").unwrap();
        store.retire("sess-unknown").unwrap();
        drop(store);
        let store = Store::open(root.path(), owner()).unwrap();
        assert_eq!(store.pending(), vec![("sess-2".into(), identity(2))]);
    }

    #[test]
    fn foreign_owner_corrupt_and_unknown_records_are_never_adopted() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path(), owner()).unwrap();
        store.record("sess-1", identity(1)).unwrap();
        drop(store);
        let mut other = owner();
        other.machine_id = "other-machine".into();
        assert!(Store::open(root.path(), other).is_err());
        let mut other = owner();
        other.service_id = None;
        assert!(Store::open(root.path(), other).is_err());
        let good = std::fs::read(root.path().join(FILE)).unwrap();
        let mut json: serde_json::Value = serde_json::from_slice(&good).unwrap();
        json["future_authority"] = serde_json::json!(true);
        let mut schema = serde_json::from_slice::<serde_json::Value>(&good).unwrap();
        schema["schema"] = serde_json::json!(2);
        let mut duplicate = serde_json::from_slice::<serde_json::Value>(&good).unwrap();
        let entry = duplicate["pending"][0].clone();
        duplicate["pending"].as_array_mut().unwrap().push(entry);
        let mut unknown_root_field = serde_json::from_slice::<serde_json::Value>(&good).unwrap();
        unknown_root_field["pending"][0]["root"]["mnt"] = serde_json::json!(1);
        for bytes in [
            b"{".to_vec(),
            serde_json::to_vec(&json).unwrap(),
            serde_json::to_vec(&schema).unwrap(),
            serde_json::to_vec(&duplicate).unwrap(),
            serde_json::to_vec(&unknown_root_field).unwrap(),
            vec![b' '; MAX_BYTES + 1],
        ] {
            std::fs::write(root.path().join(FILE), &bytes).unwrap();
            assert!(Store::open(root.path(), owner()).is_err());
            // Refusal never rewrites the evidence it refused.
            assert_eq!(std::fs::read(root.path().join(FILE)).unwrap(), bytes);
        }
    }

    #[test]
    fn linked_or_special_inputs_refuse_without_effects() {
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("retained"), b"unrelated").unwrap();
        let link = parent.path().join("cleanups");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(Store::open(&link, owner()).is_err());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);

        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside.json");
        std::fs::write(&outside, "{}").unwrap();
        std::os::unix::fs::symlink(&outside, root.path().join(FILE)).unwrap();
        assert!(Store::open(root.path(), owner()).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"{}");
    }

    #[test]
    fn namespace_has_one_owner_and_replacement_ends_writes() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("cleanups");
        let mut store = Store::open(&root, owner()).unwrap();
        assert!(Store::open(&root, owner()).is_err());
        std::fs::rename(&root, parent.path().join("retained")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(store.record("sess-1", identity(1)).is_err());
        assert!(!root.join(FILE).exists());
        assert!(store.pending().is_empty());
    }

    #[test]
    fn storage_failure_fences_writes_and_keeps_the_committed_set() {
        let root = tempfile::tempdir().unwrap();
        let mut store = Store::open(root.path(), owner()).unwrap();
        store.record("sess-1", identity(1)).unwrap();
        std::fs::remove_file(root.path().join(FILE)).unwrap();
        std::fs::create_dir(root.path().join(FILE)).unwrap();
        assert!(store.record("sess-2", identity(2)).is_err());
        assert_eq!(store.pending(), vec![("sess-1".into(), identity(1))]);
        std::fs::remove_dir(root.path().join(FILE)).unwrap();
        assert!(store.retire("sess-1").is_err());
        assert!(store.record("sess-3", identity(3)).is_err());
    }

    #[test]
    fn staging_is_never_replayed_and_the_record_budget_never_evicts() {
        let root = tempfile::tempdir().unwrap();
        let staged = Record {
            schema: 1,
            owner: owner(),
            pending: vec![Entry {
                session_id: "sess-staged".into(),
                root: identity(1),
            }],
        };
        std::fs::write(
            root.path().join(".pending-crashed"),
            serde_json::to_vec(&staged).unwrap(),
        )
        .unwrap();
        let mut store = Store::open(root.path(), owner()).unwrap();
        assert!(store.pending().is_empty());
        store.pending = (0..MAX_RECORDS as u64)
            .map(|n| (format!("sess-{n}"), identity(n)))
            .collect();
        assert!(store.record("sess-over-budget", identity(9)).is_err());
        assert_eq!(store.pending().len(), MAX_RECORDS);
        assert!(!root.path().join(FILE).exists());
    }
}

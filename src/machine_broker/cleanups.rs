//! Advisory continuation records for terminal-Session artifact cleanup.
//!
//! A record nominates one original worktree root object for a Session whose
//! terminal deletion is already committed in the deletion journal. It carries no
//! target list, path or deletion authority: resuming re-observes the root, then
//! scans for Cargo targets from scratch. Losing, ignoring or never writing a
//! record leaves exactly the pre-existing behaviour (artifacts are preserved),
//! so older Machines and reader-only builds need no compatibility floor for it.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use super::deletions::Owner;
use super::namespace::{Namespace, valid_id};
use crate::session_workspace::RootIdentity;

const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const FILE: &str = "cleanups.json";
const NOUN: &str = "cleanup continuation";

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
    namespace: Namespace,
    owner: Owner,
    pending: BTreeMap<String, RootIdentity>,
    poisoned: bool,
}

impl Store {
    /// Only a deletion-writer-admitted Machine opens this namespace; callers
    /// must treat any error as "no durable continuation", never as fatal.
    pub(super) fn open(path: &Path, owner: Owner) -> Result<Self> {
        let namespace = Namespace::open(path, &owner, NOUN)?;
        let mut pending = BTreeMap::new();
        if let Some(bytes) = namespace.read(FILE, MAX_BYTES)? {
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
        Ok(Self {
            namespace,
            owner,
            pending,
            poisoned: false,
        })
    }

    pub(super) fn contains(&self, session_id: &str) -> bool {
        self.pending.contains_key(session_id)
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
        self.namespace.check()
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
        if let Err(error) = self.namespace.commit(FILE, &bytes, &mut |_| {}) {
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

    /// Sibling tests may fork while a lock descriptor is open; the inherited
    /// copy lives only until that child execs, so retry just that window.
    fn reopen(path: &Path) -> Store {
        for _ in 0..300 {
            match Store::open(path, owner()) {
                Ok(store) => return store,
                Err(error) if format!("{error:#}").contains("already owned") => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("reopening cleanup namespace: {error:#}"),
            }
        }
        panic!("cleanup namespace stayed owned");
    }

    /// A content or link refusal, never the lock-inheritance window above.
    fn refused(path: &Path, owner: Owner, expected: &str) {
        for _ in 0..300 {
            match Store::open(path, owner.clone()) {
                Ok(_) => panic!("refused input was admitted"),
                Err(error) => {
                    let message = format!("{error:#}");
                    if message.contains("already owned") {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    assert!(message.contains(expected), "unexpected refusal: {message}");
                    return;
                }
            }
        }
        panic!("cleanup namespace stayed owned");
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
        let mut store = reopen(root.path());
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
        let store = reopen(root.path());
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
        refused(root.path(), other, "another Machine or Service");
        let mut other = owner();
        other.service_id = None;
        refused(root.path(), other, "another Machine or Service");
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
            refused(root.path(), owner(), "");
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
        refused(&link, owner(), "without following namespace links");
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);

        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside.json");
        std::fs::write(&outside, "{}").unwrap();
        std::os::unix::fs::symlink(&outside, root.path().join(FILE)).unwrap();
        refused(root.path(), owner(), "");
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

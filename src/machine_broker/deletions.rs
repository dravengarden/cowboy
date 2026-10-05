//! A reader-first, Machine-owned terminal-deletion namespace.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::namespace::Stage;
use super::namespace::{Namespace, valid_id};

const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const NOUN: &str = "deletion journal";
const FILE: &str = "deletions.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Owner {
    pub machine_id: String,
    pub service_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u16,
    owner: Owner,
    deleted: Vec<String>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) enum WriteCheckpoint {
    Staged,
    FileSynced,
    Renamed,
    DirectorySynced,
}

pub(super) struct Journal {
    namespace: Namespace,
    owner: Owner,
    deleted: HashSet<String>,
    writer_enabled: bool,
    poisoned: bool,
    #[cfg(test)]
    checkpoint: Option<Box<dyn Fn(WriteCheckpoint) + Send>>,
}

impl Journal {
    pub(super) fn open(path: &Path, owner: Owner, writer_enabled: bool) -> Result<Self> {
        let namespace = Namespace::open(path, &owner, NOUN)?;
        let mut deleted = HashSet::new();
        if let Some(bytes) = namespace.read(FILE, MAX_BYTES)? {
            let record: Record =
                serde_json::from_slice(&bytes).context("invalid deletion journal")?;
            ensure!(record.schema == 1, "unsupported deletion journal schema");
            ensure!(
                record.owner == owner,
                "deletion journal belongs to another Machine or Service"
            );
            ensure!(
                record.deleted.len() <= MAX_RECORDS,
                "deletion journal exceeds record limit"
            );
            for id in record.deleted {
                ensure!(
                    valid_id(&id) && deleted.insert(id),
                    "invalid or duplicate deletion identity"
                );
            }
        }
        let journal = Self {
            namespace,
            owner,
            deleted,
            writer_enabled,
            poisoned: false,
            #[cfg(test)]
            checkpoint: None,
        };
        journal.check()?;
        Ok(journal)
    }

    pub(super) fn deleted(&self) -> &HashSet<String> {
        &self.deleted
    }

    #[cfg(test)]
    pub(super) fn set_checkpoint(&mut self, checkpoint: impl Fn(WriteCheckpoint) + Send + 'static) {
        self.checkpoint = Some(Box::new(checkpoint));
    }

    #[cfg(test)]
    fn checkpoint(&self, stage: WriteCheckpoint) {
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint(stage);
        }
    }

    pub(super) fn writer_enabled(&self) -> bool {
        self.writer_enabled
    }

    pub(super) fn check(&self) -> Result<()> {
        ensure!(
            !self.poisoned,
            "deletion journal writer is fenced after a storage failure"
        );
        self.namespace.check()
    }

    pub(super) fn mark_deleted(&mut self, session_id: &str) -> Result<()> {
        self.check()?;
        ensure!(
            self.writer_enabled,
            "durable Session deletion writer is not admitted"
        );
        ensure!(valid_id(session_id), "invalid terminal Session identity");
        if self.deleted.contains(session_id) {
            return Ok(());
        }
        ensure!(
            self.deleted.len() < MAX_RECORDS,
            "deletion journal record budget exhausted"
        );
        let mut next: Vec<_> = self.deleted.iter().cloned().collect();
        next.push(session_id.to_owned());
        next.sort();
        let bytes = serde_json::to_vec(&Record {
            schema: 1,
            owner: self.owner.clone(),
            deleted: next,
        })?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "deletion journal byte budget exhausted"
        );
        let result = self.namespace.commit(FILE, &bytes, &mut |stage| {
            #[cfg(test)]
            self.checkpoint(match stage {
                Stage::Staged => WriteCheckpoint::Staged,
                Stage::FileSynced => WriteCheckpoint::FileSynced,
                Stage::Renamed => WriteCheckpoint::Renamed,
                Stage::DirectorySynced => WriteCheckpoint::DirectorySynced,
            });
            #[cfg(not(test))]
            let _ = stage;
        });
        if let Err(error) = result {
            self.poisoned = true;
            return Err(error.context("durable Session deletion was not confirmed"));
        }
        self.deleted.insert(session_id.to_owned());
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

    #[test]
    fn committed_terminal_ids_survive_close_and_read_only_reopen() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path(), owner(), true).unwrap();
        journal.mark_deleted("sess-1").unwrap();
        journal.mark_deleted("sess-1").unwrap();
        drop(journal);
        // A sibling test can be between fork and exec with an inherited copy of
        // the just-closed lock descriptor; retry only that window.
        let mut reader = (0..300)
            .find_map(|_| match Journal::open(root.path(), owner(), false) {
                Ok(journal) => Some(journal),
                Err(error) if format!("{error:#}").contains("already owned") => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    None
                }
                Err(error) => panic!("reopening deletion journal: {error:#}"),
            })
            .expect("journal namespace stayed owned");
        assert!(reader.deleted().contains("sess-1"));
        assert!(reader.mark_deleted("sess-2").is_err());
        assert!(!reader.deleted().contains("sess-2"));
    }

    #[test]
    fn another_owner_or_corrupt_record_cannot_be_adopted() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path(), owner(), true).unwrap();
        journal.mark_deleted("sess-1").unwrap();
        drop(journal);
        let mut other = owner();
        other.machine_id = "other-machine".into();
        assert!(Journal::open(root.path(), other, false).is_err());
        let mut other = owner();
        other.service_id = Some("other-service".into());
        assert!(Journal::open(root.path(), other, false).is_err());
        for bytes in [
            "{",
            "{\"schema\":2,\"owner\":{\"machine_id\":\"fixture-machine\",\"service_id\":\"fixture-service\"},\"deleted\":[]}",
        ] {
            std::fs::write(root.path().join("deletions.json"), bytes).unwrap();
            assert!(Journal::open(root.path(), owner(), false).is_err());
        }
    }

    #[test]
    fn duplicate_ids_and_oversized_records_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let record = Record {
            schema: 1,
            owner: owner(),
            deleted: vec!["sess-1".into(), "sess-1".into()],
        };
        std::fs::write(
            root.path().join("deletions.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        assert!(Journal::open(root.path(), owner(), false).is_err());
        std::fs::write(
            root.path().join("deletions.json"),
            vec![b' '; MAX_BYTES + 1],
        )
        .unwrap();
        assert!(Journal::open(root.path(), owner(), false).is_err());
    }

    #[test]
    fn linked_namespace_refuses_before_lock_or_record_changes() {
        for writer in [false, true] {
            let parent = tempfile::tempdir().unwrap();
            let target = parent.path().join("target");
            std::fs::create_dir(&target).unwrap();
            let retained = b"unrelated evidence";
            std::fs::write(target.join("retained"), retained).unwrap();
            let root = parent.path().join("journal");
            std::os::unix::fs::symlink(&target, &root).unwrap();
            assert!(Journal::open(&root, owner(), writer).is_err());
            assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
            assert_eq!(std::fs::read(target.join("retained")).unwrap(), retained);
            assert_eq!(std::fs::read_link(&root).unwrap(), target);
        }
    }

    #[test]
    fn replacing_a_parent_alias_ends_original_namespace_admission() {
        for writer in [false, true] {
            let parent = tempfile::tempdir().unwrap();
            let original = parent.path().join("original");
            let replacement = parent.path().join("replacement");
            std::fs::create_dir(&original).unwrap();
            std::fs::create_dir(&replacement).unwrap();
            std::fs::create_dir(replacement.join("journal")).unwrap();
            let alias = parent.path().join("alias");
            std::os::unix::fs::symlink(&original, &alias).unwrap();
            let mut journal = Journal::open(&alias.join("journal"), owner(), writer).unwrap();
            journal.check().unwrap();
            std::fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink(&replacement, &alias).unwrap();
            assert!(journal.check().is_err());
            assert!(journal.mark_deleted("sess-1").is_err());
            assert!(!original.join("journal/deletions.json").exists());
            assert_eq!(
                std::fs::read_dir(replacement.join("journal"))
                    .unwrap()
                    .count(),
                0
            );
        }
    }

    #[test]
    fn namespace_has_one_owner_and_replaced_directory_ends_admission() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("journal");
        let mut journal = Journal::open(&root, owner(), true).unwrap();
        assert!(Journal::open(&root, owner(), true).is_err());
        std::fs::rename(&root, parent.path().join("retained")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(journal.mark_deleted("sess-1").is_err());
        assert!(!root.join("deletions.json").exists());
    }

    #[test]
    fn storage_failure_fences_further_writes_without_claiming_commit() {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path(), owner(), true).unwrap();
        std::fs::create_dir(root.path().join("deletions.json")).unwrap();
        assert!(journal.mark_deleted("sess-1").is_err());
        assert!(journal.deleted().is_empty());
        std::fs::remove_dir(root.path().join("deletions.json")).unwrap();
        assert!(journal.mark_deleted("sess-2").is_err());
        assert!(journal.check().is_err());
    }

    #[test]
    fn special_files_and_unknown_fields_are_not_legacy_empty_state() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside.json");
        std::fs::write(&outside, "{}").unwrap();
        symlink(&outside, root.path().join("deletions.json")).unwrap();
        assert!(Journal::open(root.path(), owner(), false).is_err());
        std::fs::remove_file(root.path().join("deletions.json")).unwrap();
        let record = Record {
            schema: 1,
            owner: owner(),
            deleted: Vec::new(),
        };
        let mut json = serde_json::to_value(&record).unwrap();
        json["future_authority"] = serde_json::json!(true);
        std::fs::write(
            root.path().join("deletions.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        assert!(Journal::open(root.path(), owner(), false).is_err());
    }

    #[test]
    fn uncommitted_staging_is_never_replayed_and_record_budget_never_evicts() {
        let root = tempfile::tempdir().unwrap();
        let record = Record {
            schema: 1,
            owner: owner(),
            deleted: vec!["sess-staged".into()],
        };
        std::fs::write(
            root.path().join(".pending-crashed"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let mut journal = Journal::open(root.path(), owner(), true).unwrap();
        assert!(journal.deleted().is_empty());
        journal.deleted = (0..MAX_RECORDS).map(|n| format!("sess-{n}")).collect();
        assert!(journal.mark_deleted("sess-over-budget").is_err());
        assert_eq!(journal.deleted().len(), MAX_RECORDS);
        assert!(!root.path().join("deletions.json").exists());
    }
}

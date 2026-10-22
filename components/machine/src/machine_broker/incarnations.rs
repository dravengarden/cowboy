//! Machine-owned namespace of durable Session incarnations.
//!
//! Schema 1 is a closed, bounded record naming one random lineage per Session
//! slot. Every build reads and validates it. Only the dedicated writer build,
//! after component-owner admission behind the incarnation reader floor, may mint,
//! rotate or end an incarnation; otherwise the store refuses every mutation.
//! A stored value is storage identity, never authorization or a filesystem
//! identity. The terminal-deletion journal stays the only permanent fence and
//! wins any disagreement. See `docs/plugin-session-incarnation-design.md`.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use super::deletions::Owner;
#[cfg(test)]
use super::namespace::Stage;
use super::namespace::{Namespace, valid_id};

const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const FILE: &str = "incarnations.json";
const NOUN: &str = "Session incarnation namespace";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Origin {
    Minted,
    Adopted,
    Reset,
    #[allow(
        dead_code,
        reason = "valid on disk; written once workspace or binding changes reach the Machine"
    )]
    Rebound,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub session_id: String,
    pub incarnation: String,
    pub epoch: u64,
    pub origin: Origin,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u16,
    owner: Owner,
    entries: Vec<Entry>,
}

pub(super) struct Store {
    namespace: Namespace,
    owner: Owner,
    entries: BTreeMap<String, Entry>,
    writer_enabled: bool,
    poisoned: bool,
    #[cfg(test)]
    checkpoint: Option<Box<dyn Fn(Stage) + Send>>,
}

/// 128 random bits as 32 lowercase hex digits.
fn valid_incarnation(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Store {
    pub(super) fn open(path: &Path, owner: &Owner, writer_enabled: bool) -> Result<Self> {
        let namespace = Namespace::open(path, owner, NOUN)?;
        let mut entries = BTreeMap::new();
        if let Some(bytes) = namespace.read(FILE, MAX_BYTES)? {
            let record: Record =
                serde_json::from_slice(&bytes).context("invalid incarnation record")?;
            ensure!(record.schema == 1, "unsupported incarnation record schema");
            ensure!(
                record.owner == *owner,
                "incarnation record belongs to another Machine or Service"
            );
            ensure!(
                record.entries.len() <= MAX_RECORDS,
                "incarnation record exceeds record limit"
            );
            let mut values = HashSet::new();
            for entry in record.entries {
                ensure!(
                    valid_id(&entry.session_id)
                        && valid_incarnation(&entry.incarnation)
                        && values.insert(entry.incarnation.clone()),
                    "invalid or duplicate incarnation identity"
                );
                ensure!(
                    entries.insert(entry.session_id.clone(), entry).is_none(),
                    "duplicate incarnation Session identity"
                );
            }
        }
        Ok(Self {
            namespace,
            owner: owner.clone(),
            entries,
            writer_enabled,
            poisoned: false,
            #[cfg(test)]
            checkpoint: None,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn writer_enabled(&self) -> bool {
        self.writer_enabled
    }

    pub(super) fn get(&self, session_id: &str) -> Option<&Entry> {
        self.entries.get(session_id)
    }

    #[cfg(test)]
    pub(super) fn set_checkpoint(&mut self, checkpoint: impl Fn(Stage) + Send + 'static) {
        self.checkpoint = Some(Box::new(checkpoint));
    }

    #[cfg(test)]
    pub(super) fn check(&self) -> Result<()> {
        self.namespace.check()
    }

    fn admit_write(&self) -> Result<()> {
        ensure!(
            !self.poisoned,
            "Session incarnation writer is fenced after a storage failure"
        );
        self.namespace.check()?;
        ensure!(
            self.writer_enabled,
            "durable Session incarnation writer is not admitted"
        );
        Ok(())
    }

    fn fresh_value(&self) -> String {
        loop {
            let value = format!("{:032x}", rand::random::<u128>());
            if self
                .entries
                .values()
                .all(|entry| entry.incarnation != value)
            {
                return value;
            }
        }
    }

    /// The Session's current incarnation, creating one only when none exists. An
    /// existing lineage is returned unchanged and writes nothing, so replaying a
    /// declaration (adoption, reconnect, wake) never rotates it.
    pub(super) fn mint(&mut self, session_id: &str, origin: Origin) -> Result<String> {
        ensure!(valid_id(session_id), "invalid Session identity");
        if let Some(entry) = self.entries.get(session_id) {
            return Ok(entry.incarnation.clone());
        }
        self.admit_write()?;
        ensure!(
            self.entries.len() < MAX_RECORDS,
            "Session incarnation record budget exhausted"
        );
        let entry = Entry {
            session_id: session_id.to_owned(),
            incarnation: self.fresh_value(),
            epoch: 1,
            origin,
        };
        let value = entry.incarnation.clone();
        let mut next = self.entries.clone();
        next.insert(session_id.to_owned(), entry);
        self.commit(next)?;
        Ok(value)
    }

    /// Start a new lineage for a Session that is being reset: a fresh random
    /// value and a higher epoch, committed before the reset's first effect. A
    /// Session with no record is minted as a reset lineage.
    pub(super) fn rotate(&mut self, session_id: &str) -> Result<String> {
        ensure!(valid_id(session_id), "invalid Session identity");
        self.admit_write()?;
        let epoch = self
            .entries
            .get(session_id)
            .map_or(Some(1), |entry| entry.epoch.checked_add(1))
            .context("Session incarnation epoch exhausted")?;
        if !self.entries.contains_key(session_id) {
            ensure!(
                self.entries.len() < MAX_RECORDS,
                "Session incarnation record budget exhausted"
            );
        }
        let entry = Entry {
            session_id: session_id.to_owned(),
            incarnation: self.fresh_value(),
            epoch,
            origin: Origin::Reset,
        };
        let value = entry.incarnation.clone();
        let mut next = self.entries.clone();
        next.insert(session_id.to_owned(), entry);
        self.commit(next)?;
        Ok(value)
    }

    /// Remove the record of a terminally deleted Session. The deletion journal is
    /// authoritative; a leftover record for a journal-deleted ID is harmless.
    pub(super) fn end(&mut self, session_id: &str) -> Result<()> {
        if !self.entries.contains_key(session_id) {
            return Ok(());
        }
        self.admit_write()?;
        let mut next = self.entries.clone();
        next.remove(session_id);
        self.commit(next)
    }

    fn commit(&mut self, next: BTreeMap<String, Entry>) -> Result<()> {
        let bytes = serde_json::to_vec(&Record {
            schema: 1,
            owner: self.owner.clone(),
            entries: next.values().cloned().collect(),
        })?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "Session incarnation byte budget exhausted"
        );
        let result = self.namespace.commit(FILE, &bytes, &mut |stage| {
            #[cfg(test)]
            if let Some(checkpoint) = &self.checkpoint {
                checkpoint(stage);
            }
            #[cfg(not(test))]
            let _ = stage;
        });
        if let Err(error) = result {
            // Unconfirmed, never rolled back: no further write is admitted.
            self.poisoned = true;
            return Err(error.context("durable Session incarnation was not confirmed"));
        }
        self.entries = next;
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

    fn entry(id: &str, incarnation: char, epoch: u64, origin: Origin) -> Entry {
        Entry {
            session_id: id.into(),
            incarnation: incarnation.to_string().repeat(32),
            epoch,
            origin,
        }
    }

    fn write(root: &Path, entries: Vec<Entry>) {
        let record = Record {
            schema: 1,
            owner: owner(),
            entries,
        };
        std::fs::write(root.join(FILE), serde_json::to_vec(&record).unwrap()).unwrap();
    }

    /// Sibling tests may fork while a lock descriptor is open; the inherited
    /// copy lives only until that child execs, so retry just that window.
    fn open(path: &Path) -> Result<Store> {
        for _ in 0..300 {
            match Store::open(path, &owner(), false) {
                Err(error) if format!("{error:#}").contains("already owned") => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other,
            }
        }
        panic!("incarnation namespace stayed owned");
    }

    fn writer(path: &Path) -> Store {
        for _ in 0..300 {
            match Store::open(path, &owner(), true) {
                Err(error) if format!("{error:#}").contains("already owned") => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other.unwrap(),
            }
        }
        panic!("incarnation namespace stayed owned");
    }

    #[test]
    fn writer_mints_once_rotates_with_a_higher_epoch_and_ends() {
        let root = tempfile::tempdir().unwrap();
        let mut store = writer(root.path());
        let first = store.mint("sess-1", Origin::Minted).unwrap();
        assert_eq!(first.len(), 32);
        let bytes = std::fs::read(root.path().join(FILE)).unwrap();
        // Replaying a declaration returns the same lineage and writes nothing.
        assert_eq!(store.mint("sess-1", Origin::Adopted).unwrap(), first);
        assert_eq!(std::fs::read(root.path().join(FILE)).unwrap(), bytes);
        assert_eq!(store.get("sess-1").unwrap().origin, Origin::Minted);
        let second = store.mint("sess-2", Origin::Adopted).unwrap();
        assert_ne!(first, second);

        let rotated = store.rotate("sess-1").unwrap();
        assert_ne!(rotated, first);
        let entry = store.get("sess-1").unwrap();
        assert_eq!((entry.epoch, entry.origin), (2, Origin::Reset));
        // Resetting a Session that has no record mints a reset lineage.
        store.rotate("sess-3").unwrap();
        assert_eq!(store.get("sess-3").unwrap().epoch, 1);

        store.end("sess-2").unwrap();
        store.end("sess-unknown").unwrap();
        assert!(store.get("sess-2").is_none());
        drop(store);
        let reader = open(root.path()).unwrap();
        assert_eq!(reader.len(), 2);
        assert_eq!(reader.get("sess-1").unwrap().incarnation, rotated);
        assert_eq!(reader.get("sess-3").unwrap().origin, Origin::Reset);
    }

    #[test]
    fn a_reader_returns_existing_lineages_but_never_mutates() {
        let root = tempfile::tempdir().unwrap();
        let mut store = writer(root.path());
        let existing = store.mint("sess-1", Origin::Minted).unwrap();
        drop(store);
        let bytes = std::fs::read(root.path().join(FILE)).unwrap();
        let mut reader = open(root.path()).unwrap();
        assert!(!reader.writer_enabled());
        assert_eq!(reader.mint("sess-1", Origin::Minted).unwrap(), existing);
        for refused in [
            reader.mint("sess-2", Origin::Minted).map(drop),
            reader.rotate("sess-1").map(drop),
            reader.end("sess-1"),
        ] {
            let error = refused.unwrap_err();
            assert!(
                format!("{error:#}").contains("writer is not admitted"),
                "{error:#}"
            );
        }
        assert_eq!(std::fs::read(root.path().join(FILE)).unwrap(), bytes);
        assert_eq!(reader.get("sess-1").unwrap().incarnation, existing);
    }

    #[test]
    fn commit_stages_publish_only_after_the_file_is_synced() {
        let root = tempfile::tempdir().unwrap();
        let mut store = writer(root.path());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = root.path().join(FILE);
        let log = std::sync::Arc::clone(&seen);
        store.set_checkpoint(move |stage| {
            log.lock()
                .unwrap()
                .push((format!("{stage:?}"), record.exists()))
        });
        store.mint("sess-1", Origin::Minted).unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            [
                ("Staged".to_owned(), false),
                ("FileSynced".to_owned(), false),
                ("Renamed".to_owned(), true),
                ("DirectorySynced".to_owned(), true),
            ]
        );
    }

    #[test]
    fn storage_failure_fences_writes_and_keeps_the_committed_set() {
        let root = tempfile::tempdir().unwrap();
        let mut store = writer(root.path());
        let first = store.mint("sess-1", Origin::Minted).unwrap();
        std::fs::remove_file(root.path().join(FILE)).unwrap();
        std::fs::create_dir(root.path().join(FILE)).unwrap();
        let error = store.mint("sess-2", Origin::Minted).unwrap_err();
        assert!(format!("{error:#}").contains("was not confirmed"));
        // Unconfirmed is not rolled back: memory keeps the last confirmed state.
        assert_eq!(store.get("sess-1").unwrap().incarnation, first);
        assert!(store.get("sess-2").is_none());
        std::fs::remove_dir(root.path().join(FILE)).unwrap();
        for refused in [
            store.mint("sess-3", Origin::Minted).map(drop),
            store.rotate("sess-1").map(drop),
            store.end("sess-1"),
        ] {
            assert!(
                format!("{:#}", refused.unwrap_err()).contains("fenced after a storage failure")
            );
        }
        // An existing lineage is still readable while writes are fenced.
        assert_eq!(store.mint("sess-1", Origin::Minted).unwrap(), first);
    }

    #[test]
    fn exhausted_budget_refuses_new_lineages_without_evicting() {
        let root = tempfile::tempdir().unwrap();
        let mut store = writer(root.path());
        store.entries = (0..MAX_RECORDS as u64)
            .map(|n| {
                let id = format!("sess-{n}");
                (
                    id.clone(),
                    Entry {
                        session_id: id,
                        incarnation: format!("{n:032x}"),
                        epoch: 1,
                        origin: Origin::Minted,
                    },
                )
            })
            .collect();
        assert!(store.mint("sess-over", Origin::Minted).is_err());
        assert!(store.rotate("sess-over").is_err());
        assert_eq!(store.len(), MAX_RECORDS);
        assert!(!root.path().join(FILE).exists());
        // An existing lineage can still rotate and end at the budget.
        store.rotate("sess-1").unwrap();
        store.end("sess-2").unwrap();
    }

    #[test]
    fn a_committed_record_is_read_and_an_absent_one_is_empty() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(open(root.path()).unwrap().len(), 0);
        write(
            root.path(),
            vec![
                entry("sess-1", 'a', 1, Origin::Minted),
                entry("sess-2", 'b', 4, Origin::Reset),
                entry("sess-3", 'c', 2, Origin::Adopted),
                entry("sess-4", 'd', 3, Origin::Rebound),
            ],
        );
        let reader = open(root.path()).unwrap();
        assert_eq!(reader.len(), 4);
        assert_eq!(reader.get("sess-2").unwrap().epoch, 4);
        assert_eq!(reader.get("sess-4").unwrap().origin, Origin::Rebound);
        assert!(reader.get("sess-9").is_none());
    }

    #[test]
    fn foreign_owner_schema_and_malformed_records_are_never_adopted() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), vec![entry("sess-1", 'a', 1, Origin::Minted)]);
        let good = std::fs::read(root.path().join(FILE)).unwrap();
        let value = |edit: &dyn Fn(&mut serde_json::Value)| {
            let mut json: serde_json::Value = serde_json::from_slice(&good).unwrap();
            edit(&mut json);
            serde_json::to_vec(&json).unwrap()
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("truncated", b"{".to_vec()),
            ("schema", value(&|j| j["schema"] = serde_json::json!(2))),
            (
                "machine",
                value(&|j| j["owner"]["machine_id"] = serde_json::json!("other")),
            ),
            (
                "service",
                value(&|j| j["owner"]["service_id"] = serde_json::Value::Null),
            ),
            (
                "unknown",
                value(&|j| j["future_authority"] = serde_json::json!(true)),
            ),
            (
                "entry-unknown",
                value(&|j| j["entries"][0]["lease"] = serde_json::json!(1)),
            ),
            (
                "origin",
                value(&|j| j["entries"][0]["origin"] = serde_json::json!("restored")),
            ),
            (
                "epoch",
                value(&|j| j["entries"][0]["epoch"] = serde_json::json!(-1)),
            ),
            (
                "short",
                value(&|j| j["entries"][0]["incarnation"] = serde_json::json!("abc")),
            ),
            (
                "uppercase",
                value(&|j| j["entries"][0]["incarnation"] = serde_json::json!("A".repeat(32))),
            ),
            (
                "not-hex",
                value(&|j| j["entries"][0]["incarnation"] = serde_json::json!("g".repeat(32))),
            ),
            (
                "empty-id",
                value(&|j| j["entries"][0]["session_id"] = serde_json::json!("")),
            ),
            (
                "duplicate-id",
                value(&|j| {
                    let first = j["entries"][0].clone();
                    let mut second = first.clone();
                    second["incarnation"] = serde_json::json!("b".repeat(32));
                    j["entries"] = serde_json::json!([first, second]);
                }),
            ),
            (
                "shared-lineage",
                value(&|j| {
                    let mut second = j["entries"][0].clone();
                    second["session_id"] = serde_json::json!("sess-2");
                    let first = j["entries"][0].clone();
                    j["entries"] = serde_json::json!([first, second]);
                }),
            ),
            ("oversized", vec![b' '; MAX_BYTES + 1]),
        ];
        for (name, bytes) in cases {
            std::fs::write(root.path().join(FILE), &bytes).unwrap();
            let error = open(root.path())
                .err()
                .unwrap_or_else(|| panic!("accepted {name}"));
            assert!(
                !format!("{error:#}").contains("already owned"),
                "{name} refused only because of the lock: {error:#}"
            );
            // Refusal never rewrites the evidence it refused.
            assert_eq!(
                std::fs::read(root.path().join(FILE)).unwrap(),
                bytes,
                "{name}"
            );
        }
        let over: Vec<Entry> = (0..=MAX_RECORDS)
            .map(|n| Entry {
                session_id: format!("sess-{n}"),
                incarnation: format!("{n:032x}"),
                epoch: 1,
                origin: Origin::Minted,
            })
            .collect();
        write(root.path(), over);
        assert!(open(root.path()).is_err());
    }

    #[test]
    fn linked_and_special_inputs_refuse_without_effects() {
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("retained"), b"unrelated").unwrap();
        let link = parent.path().join("incarnations");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(Store::open(&link, &owner(), false).is_err());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);

        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside.json");
        std::fs::write(&outside, "{}").unwrap();
        std::os::unix::fs::symlink(&outside, root.path().join(FILE)).unwrap();
        assert!(open(root.path()).is_err());
        std::fs::remove_file(root.path().join(FILE)).unwrap();
        std::fs::create_dir(root.path().join(FILE)).unwrap();
        assert!(open(root.path()).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"{}");
    }

    #[test]
    fn namespace_has_one_owner_and_replacement_ends_admission() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("incarnations");
        let reader = Store::open(&root, &owner(), false).unwrap();
        assert!(
            format!("{:#}", Store::open(&root, &owner(), false).err().unwrap())
                .contains("already owned")
        );
        reader.check().unwrap();
        std::fs::rename(&root, parent.path().join("retained")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(reader.check().is_err());
    }

    #[test]
    fn a_replaced_parent_alias_ends_admission_and_staging_is_never_read() {
        let parent = tempfile::tempdir().unwrap();
        let original = parent.path().join("original");
        let replacement = parent.path().join("replacement");
        std::fs::create_dir(&original).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        std::fs::create_dir(replacement.join("incarnations")).unwrap();
        let alias = parent.path().join("alias");
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        let reader = Store::open(&alias.join("incarnations"), &owner(), false).unwrap();
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&replacement, &alias).unwrap();
        assert!(reader.check().is_err());

        let root = tempfile::tempdir().unwrap();
        let staged = Record {
            schema: 1,
            owner: owner(),
            entries: vec![entry("sess-staged", 'a', 1, Origin::Minted)],
        };
        std::fs::write(
            root.path().join(".pending-crashed"),
            serde_json::to_vec(&staged).unwrap(),
        )
        .unwrap();
        assert_eq!(open(root.path()).unwrap().len(), 0);
        assert!(!root.path().join(FILE).exists());
    }
}

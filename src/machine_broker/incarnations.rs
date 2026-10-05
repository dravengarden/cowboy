//! Reader-first Machine-owned namespace of durable Session incarnations.
//!
//! Schema 1 is a closed, bounded record naming one random lineage per Session
//! slot. This module only reads and validates it: no writer exists, so a release
//! declares `writerSchema: 0` and nothing mints, rotates or ends an incarnation.
//! Its purpose is to let a floor be anchored on a reader before any writer is
//! admitted. A stored value is storage identity, never authorization.
//! See `docs/plugin-session-incarnation-design.md`.

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use super::deletions::Owner;

const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const FILE: &str = "incarnations.json";

#[allow(
    dead_code,
    reason = "validated schema vocabulary; no consumer exists until a writer is admitted"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Origin {
    Minted,
    Adopted,
    Reset,
    Rebound,
}

#[allow(
    dead_code,
    reason = "validated schema fields; no consumer exists until a writer is admitted"
)]
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

pub(super) struct Reader {
    root: PathBuf,
    root_handle: File,
    lock: File,
    entries: BTreeMap<String, Entry>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.contains('\0')
}

/// 128 random bits as 32 lowercase hex digits.
fn valid_incarnation(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Reader {
    pub(super) fn open(path: &Path, owner: &Owner) -> Result<Self> {
        ensure!(
            valid_id(&owner.machine_id),
            "invalid incarnation namespace Machine identity"
        );
        ensure!(
            owner.service_id.as_deref().is_none_or(valid_id),
            "invalid incarnation namespace Service identity"
        );
        match std::fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        // Keep the caller's path unresolved: a linked namespace must refuse, and
        // replacing a parent alias must end this owner's admission.
        let root = std::path::absolute(path)?;
        let root_handle = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&root)
            .context("opening incarnation namespace without following links")?;
        File::open(
            root.parent()
                .context("incarnation namespace has no parent")?,
        )?
        .sync_all()?;
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
            "incarnation namespace lock is not a regular file"
        );
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("Session incarnation namespace already owned")?;
        let mut entries = BTreeMap::new();
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join(FILE))
        {
            Ok(file) => {
                ensure!(
                    file.metadata()?.is_file(),
                    "incarnation record is not a regular file"
                );
                let mut bytes = Vec::new();
                file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= MAX_BYTES,
                    "incarnation record exceeds byte limit"
                );
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let reader = Self {
            root,
            root_handle,
            lock,
            entries,
        };
        reader.check()?;
        Ok(reader)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn get(&self, session_id: &str) -> Option<&Entry> {
        self.entries.get(session_id)
    }

    pub(super) fn check(&self) -> Result<()> {
        let root = std::fs::symlink_metadata(&self.root)?;
        let held = self.root_handle.metadata()?;
        ensure!(
            root.is_dir() && root.dev() == held.dev() && root.ino() == held.ino(),
            "incarnation namespace directory was replaced"
        );
        let lock = std::fs::symlink_metadata(self.root.join(".lock"))?;
        let held = self.lock.metadata()?;
        ensure!(
            lock.is_file() && lock.dev() == held.dev() && lock.ino() == held.ino(),
            "incarnation namespace lock was replaced"
        );
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
    fn open(path: &Path) -> Result<Reader> {
        for _ in 0..300 {
            match Reader::open(path, &owner()) {
                Err(error) if format!("{error:#}").contains("already owned") => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other,
            }
        }
        panic!("incarnation namespace stayed owned");
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
        assert!(Reader::open(&link, &owner()).is_err());
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
        let reader = Reader::open(&root, &owner()).unwrap();
        assert!(
            format!("{:#}", Reader::open(&root, &owner()).err().unwrap()).contains("already owned")
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
        let reader = Reader::open(&alias.join("incarnations"), &owner()).unwrap();
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

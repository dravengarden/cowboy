//! Shared mechanics of the Machine-owned durable namespaces.
//!
//! The terminal-deletion journal, the incarnation namespace and the cleanup
//! continuation each own one directory below the Machine state directory. They
//! share how it is opened and kept (no links, one exclusive lock, retained
//! handles that end admission when replaced), how a record is read (regular,
//! non-blocking, bounded) and how one is committed (staged file, sync, atomic
//! rename, directory sync). What a record means, who may write it and how it
//! parses stay with each dataset. See `docs/plugin-state-dataset-design.md`.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};

use super::deletions::Owner;

/// A point in [`Namespace::commit`] that tests may interrupt.
#[derive(Clone, Copy, Debug)]
pub(super) enum Stage {
    Staged,
    FileSynced,
    Renamed,
    DirectorySynced,
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.contains('\0')
}

pub(super) struct Namespace {
    root: PathBuf,
    root_handle: File,
    lock: File,
    /// Names the dataset in every refusal, e.g. "deletion journal".
    noun: &'static str,
}

impl Namespace {
    pub(super) fn open(path: &Path, owner: &Owner, noun: &'static str) -> Result<Self> {
        ensure!(
            valid_id(&owner.machine_id),
            "invalid {noun} Machine identity"
        );
        ensure!(
            owner.service_id.as_deref().is_none_or(valid_id),
            "invalid {noun} Service identity"
        );
        // Machine state already owns the parent. Flush the newly created
        // namespace entry too; syncing only files inside it would not make a
        // first committed record survive loss of the parent entry.
        match std::fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        // Retain the caller's namespace path without resolving links. Resolving
        // it first would both admit a linked directory and erase the path whose
        // replacement must end this owner's admission.
        let root = std::path::absolute(path)?;
        let root_handle = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&root)
            .with_context(|| {
                format!("opening {noun} directory without following namespace links")
            })?;
        File::open(
            root.parent()
                .with_context(|| format!("{noun} namespace has no parent"))?,
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
            "{noun} lock is not a regular file"
        );
        fs2::FileExt::try_lock_exclusive(&lock).with_context(|| format!("{noun} already owned"))?;
        let namespace = Self {
            root,
            root_handle,
            lock,
            noun,
        };
        namespace.check()?;
        Ok(namespace)
    }

    /// The record's bytes, or `None` when it was never committed. A link, a
    /// special file or more than `limit` bytes refuse; staging files are never
    /// read.
    pub(super) fn read(&self, file: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let noun = self.noun;
        match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.root.join(file))
        {
            Ok(record) => {
                ensure!(record.metadata()?.is_file(), "{noun} is not a regular file");
                let mut bytes = Vec::new();
                record.take((limit + 1) as u64).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= limit, "{noun} exceeds byte limit");
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Ownership is intact only while the original directory and lock objects
    /// still sit at the retained paths.
    pub(super) fn check(&self) -> Result<()> {
        let noun = self.noun;
        let root = std::fs::symlink_metadata(&self.root)?;
        let held = self.root_handle.metadata()?;
        ensure!(
            root.is_dir() && root.dev() == held.dev() && root.ino() == held.ino(),
            "{noun} directory was replaced"
        );
        let lock = std::fs::symlink_metadata(self.root.join(".lock"))?;
        let held = self.lock.metadata()?;
        ensure!(
            lock.is_file() && lock.dev() == held.dev() && lock.ino() == held.ino(),
            "{noun} lock was replaced"
        );
        Ok(())
    }

    /// Publish `bytes` as `file`: stage, sync, verify ownership, rename, sync the
    /// directory, verify again. A failure leaves the outcome unconfirmed, never
    /// rolled back; `hook` lets tests stop between the steps.
    pub(super) fn commit(
        &self,
        file: &str,
        bytes: &[u8],
        hook: &mut dyn FnMut(Stage),
    ) -> Result<()> {
        let pending = self
            .root
            .join(format!(".pending-{:032x}", rand::random::<u128>()));
        let mut staged = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&pending)?;
        staged.write_all(bytes)?;
        hook(Stage::Staged);
        staged.sync_all()?;
        hook(Stage::FileSynced);
        self.check()?;
        std::fs::rename(&pending, self.root.join(file))?;
        hook(Stage::Renamed);
        self.root_handle.sync_all()?;
        hook(Stage::DirectorySynced);
        self.check()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> Owner {
        Owner {
            machine_id: "fixture-machine".into(),
            service_id: None,
        }
    }

    #[test]
    fn commit_publishes_through_the_four_stages_in_order() {
        let root = tempfile::tempdir().unwrap();
        let namespace = Namespace::open(root.path(), &owner(), "fixture dataset").unwrap();
        assert!(namespace.read("record.json", 64).unwrap().is_none());
        let mut stages = Vec::new();
        namespace
            .commit("record.json", b"{}", &mut |stage| stages.push(stage))
            .unwrap();
        assert!(matches!(
            stages.as_slice(),
            [
                Stage::Staged,
                Stage::FileSynced,
                Stage::Renamed,
                Stage::DirectorySynced
            ]
        ));
        assert_eq!(namespace.read("record.json", 64).unwrap().unwrap(), b"{}");
        // Staging never survives a successful commit.
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".pending-")
        }));
    }

    #[test]
    fn reads_are_bounded_regular_and_unlinked() {
        let root = tempfile::tempdir().unwrap();
        let namespace = Namespace::open(root.path(), &owner(), "fixture dataset").unwrap();
        std::fs::write(root.path().join("big.json"), vec![b' '; 65]).unwrap();
        let error = namespace.read("big.json", 64).unwrap_err();
        assert!(format!("{error:#}").contains("fixture dataset exceeds byte limit"));
        std::fs::write(root.path().join("exact.json"), vec![b' '; 64]).unwrap();
        assert_eq!(namespace.read("exact.json", 64).unwrap().unwrap().len(), 64);
        std::os::unix::fs::symlink(
            root.path().join("exact.json"),
            root.path().join("link.json"),
        )
        .unwrap();
        assert!(namespace.read("link.json", 64).is_err());
        std::fs::create_dir(root.path().join("dir.json")).unwrap();
        let error = namespace.read("dir.json", 64).unwrap_err();
        assert!(format!("{error:#}").contains("fixture dataset is not a regular file"));
    }

    #[test]
    fn refusals_name_the_dataset_and_a_second_owner_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let namespace = Namespace::open(root.path(), &owner(), "fixture dataset").unwrap();
        let error = Namespace::open(root.path(), &owner(), "fixture dataset")
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("fixture dataset already owned"));
        let mut bad = owner();
        bad.machine_id.clear();
        let error = Namespace::open(root.path(), &bad, "fixture dataset")
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("invalid fixture dataset Machine identity"));
        // A replaced lock ends admission and a failed commit publishes nothing.
        std::fs::rename(root.path().join(".lock"), root.path().join("retained-lock")).unwrap();
        std::fs::write(root.path().join(".lock"), b"replacement").unwrap();
        let error = namespace.check().unwrap_err();
        assert!(format!("{error:#}").contains("fixture dataset lock was replaced"));
        assert!(namespace.commit("record.json", b"{}", &mut |_| {}).is_err());
        assert!(!root.path().join("record.json").exists());
    }
}

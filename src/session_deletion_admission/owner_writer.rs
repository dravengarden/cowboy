//! Finite Hawk component-owned writer admission. This is a startup check of
//! administrator-owned selection, not a lease against independent sudo use.

use std::fs::OpenOptions;
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use serde::Deserialize;

const PROFILE: &str = "/nix/var/nix/profiles/columbus-components/cowboy-machine";
const DELETION_FLOOR: &str =
    "/var/lib/hawk-component-deployments/cowboy-machine/session-deletion-reader-floor.json";
const INCARNATION_FLOOR: &str =
    "/var/lib/hawk-component-deployments/cowboy-machine/session-incarnation-reader-floor.json";

/// The durable Machine datasets whose writer the component owner can admit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dataset {
    Deletion,
    Incarnation,
}

impl Dataset {
    fn floor(self) -> &'static str {
        match self {
            Self::Deletion => DELETION_FLOOR,
            Self::Incarnation => INCARNATION_FLOOR,
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Deletion => "Session deletion",
            Self::Incarnation => "Session incarnation",
        }
    }

    /// Compile-time build selector. Runtime environment can never enable a writer.
    fn build(self) -> Option<&'static str> {
        match self {
            Self::Deletion => option_env!("COWBOY_SESSION_DELETION_WRITER_BUILD"),
            Self::Incarnation => option_env!("COWBOY_SESSION_INCARNATION_WRITER_BUILD"),
        }
    }
}
const LIMIT: u64 = 8192;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Floor {
    schema: u16,
    machine: String,
    dataset: PathBuf,
    #[serde(rename = "readerSchema")]
    reader_schema: u16,
    release: PathBuf,
    revision: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Declaration {
    reader_schema: u16,
    writer_schema: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Source {
    schema: u16,
    component: String,
    lane: String,
    repository: String,
    revision: String,
    dirty: bool,
    #[serde(default)]
    bootstrap: bool,
    worker_generation: String,
    session_deletion_journal: Declaration,
    /// Reader-only: no build may declare an incarnation writer yet.
    #[serde(default)]
    session_incarnations: Option<Declaration>,
}

fn revision_valid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn store_root(path: &Path) -> bool {
    path.to_str()
        .and_then(|value| value.strip_prefix("/nix/store/"))
        .is_some_and(|name| !name.is_empty() && !name.contains('/') && name != "." && name != "..")
}

impl Floor {
    fn validate(&self, namespace: &Path, machine: &str) -> Result<()> {
        ensure!(
            self.schema == 1
                && self.reader_schema == 1
                && self.machine == machine
                && self.dataset == std::path::absolute(namespace)?
                && store_root(&self.release)
                && revision_valid(&self.revision),
            "component Session deletion writer floor identity is invalid"
        );
        Ok(())
    }
}

impl Source {
    fn validate(&self, revision: &str, dataset: Dataset) -> Result<()> {
        // An incarnation writer additionally needs the deletion writer: ending a
        // lineage follows the committed terminal deletion.
        let incarnations = self.session_incarnations.as_ref();
        ensure!(
            match dataset {
                Dataset::Deletion => incarnations.is_none_or(|declaration| {
                    declaration.reader_schema == 1 && declaration.writer_schema <= 1
                }),
                Dataset::Incarnation => incarnations.is_some_and(|declaration| {
                    declaration.reader_schema == 1 && declaration.writer_schema == 1
                }),
            },
            "component {} writer source does not declare this dataset's writer",
            dataset.noun()
        );
        ensure!(
            revision_valid(revision)
                && self.schema == 1
                && self.component == "cowboy"
                && self.lane == "machine"
                && self.repository == "git@github.com:dravengarden/cowboy.git"
                && self.revision == revision
                && !self.dirty
                && !self.bootstrap
                && self
                    .worker_generation
                    .strip_prefix("worker-")
                    .is_some_and(|id| {
                        id.len() == 20
                            && id
                                .bytes()
                                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    })
                && self.session_deletion_journal.reader_schema == 1
                && self.session_deletion_journal.writer_schema == 1,
            "component {} writer source does not match this native build",
            dataset.noun()
        );
        Ok(())
    }
}

fn trusted_metadata(metadata: &std::fs::Metadata, uid: u32) -> Result<()> {
    ensure!(
        metadata.uid() == uid && metadata.mode() & 0o022 == 0,
        "component Session deletion writer authority is not administrator-owned"
    );
    Ok(())
}

fn trusted_parents(path: &Path) -> Result<()> {
    for parent in path.ancestors().skip(1) {
        let metadata = std::fs::symlink_metadata(parent)?;
        ensure!(
            metadata.is_dir(),
            "linked component writer authority parent"
        );
        trusted_metadata(&metadata, 0)?;
    }
    Ok(())
}

fn read_regular(path: &Path, uid: u32) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    trusted_metadata(&metadata, uid)?;
    ensure!(
        metadata.is_file() && metadata.len() <= LIMIT,
        "component writer authority must be a bounded regular file"
    );
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "component writer authority exceeds limit"
    );
    Ok(bytes)
}

fn selected_release(profile: &Path) -> Result<PathBuf> {
    trusted_parents(profile)?;
    let parent = profile
        .parent()
        .context("component profile has no parent")?;
    let mut path = profile.to_path_buf();
    for _ in 0..16 {
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.is_symlink() {
            ensure!(
                metadata.uid() == 0 && path.parent() == Some(parent),
                "untrusted component writer selection link"
            );
            let target = std::fs::read_link(&path)?;
            path = if target.is_absolute() {
                target
            } else {
                parent.join(target)
            };
            ensure!(
                store_root(&path) || path.parent() == Some(parent),
                "component writer selection leaves its administrator-owned namespace"
            );
        } else {
            ensure!(
                metadata.is_dir() && store_root(&path),
                "component writer selection is not an immutable release"
            );
            trusted_metadata(&metadata, 0)?;
            return Ok(path);
        }
    }
    bail!("component writer selection link limit exceeded")
}

/// Default builds never consult these authorities or admit a writer. Only the
/// dedicated clean Nix writer build can pass the fixed root selection and floor.
pub(crate) fn admitted(namespace: &Path, machine: &str) -> Result<bool> {
    admitted_dataset(Dataset::Deletion, namespace, machine)
}

pub(crate) fn admitted_dataset(dataset: Dataset, namespace: &Path, machine: &str) -> Result<bool> {
    match dataset.build() {
        None => return Ok(false),
        Some("schema1") => {}
        Some(_) => bail!("unsupported compiled {} writer build", dataset.noun()),
    }
    let revision = option_env!("COWBOY_SESSION_DELETION_WRITER_REVISION")
        .with_context(|| format!("{} writer build has no source revision", dataset.noun()))?;
    let profile = Path::new(PROFILE);
    let release = selected_release(profile)
        .with_context(|| format!("reading component {} writer selection", dataset.noun()))?;
    let source: Source = serde_json::from_slice(&read_regular(
        &release.join("etc/cowboy-release/source.json"),
        0,
    )?)?;
    source.validate(revision, dataset)?;
    let wrapper = release.join("libexec/cowboy-machine").canonicalize()?;
    let native = wrapper
        .parent()
        .context("Machine wrapper has no parent")?
        .join(".cowboy-machine-wrapped");
    ensure!(
        native.canonicalize()? == std::env::current_exe()?.canonicalize()?,
        "component Session writer native executable is not selected"
    );
    let native_metadata = std::fs::symlink_metadata(&native)?;
    ensure!(
        native_metadata.is_file(),
        "component writer native is not a regular file"
    );
    trusted_metadata(&native_metadata, 0)?;
    let floor_path = Path::new(dataset.floor());
    trusted_parents(floor_path)?;
    let floor: Floor = serde_json::from_slice(&read_regular(floor_path, 0)?)
        .with_context(|| format!("reading component {} writer floor", dataset.noun()))?;
    floor.validate(namespace, machine)?;
    ensure!(
        selected_release(profile)? == release,
        "component writer selection changed during admission"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_build_stays_read_only_without_authority_or_namespace_effects() {
        let root = tempfile::tempdir().unwrap();
        assert!(!admitted(&root.path().join("absent"), "fixture").unwrap());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }

    #[test]
    fn floor_is_closed_and_binds_machine_and_exact_dataset() {
        let bytes = br#"{"schema":1,"machine":"fixture","dataset":"/private/session-deletions","readerSchema":1,"release":"/nix/store/accepted-reader","revision":"0123456789012345678901234567890123456789"}"#;
        let floor: Floor = serde_json::from_slice(bytes).unwrap();
        floor
            .validate(Path::new("/private/session-deletions"), "fixture")
            .unwrap();
        assert!(
            floor
                .validate(Path::new("/other/session-deletions"), "fixture")
                .is_err()
        );
        assert!(
            floor
                .validate(Path::new("/private/session-deletions"), "foreign")
                .is_err()
        );
        for replacement in [
            "/tmp/mutable",
            "/nix/store/root/subdir",
            "/nix/store/../root",
            "/nix/store//root",
        ] {
            assert!(!store_root(Path::new(replacement)));
        }
        for replacement in [
            r#""readerSchema":1,"readerSchema":1"#,
            r#""readerSchema":1,"writer":true"#,
        ] {
            let changed = String::from_utf8(bytes.to_vec())
                .unwrap()
                .replace(r#""readerSchema":1"#, replacement);
            assert!(serde_json::from_str::<Floor>(&changed).is_err());
        }
    }

    #[test]
    fn source_requires_exact_clean_machine_writer_revision() {
        let bytes = r#"{"schema":1,"component":"cowboy","lane":"machine","repository":"git@github.com:dravengarden/cowboy.git","revision":"0123456789012345678901234567890123456789","dirty":false,"workerGeneration":"worker-01234567890123456789","sessionDeletionJournal":{"readerSchema":1,"writerSchema":1}}"#;
        let source: Source = serde_json::from_str(bytes).unwrap();
        source
            .validate(&source.revision, Dataset::Deletion)
            .unwrap();
        assert!(
            source
                .validate(
                    "1123456789012345678901234567890123456789",
                    Dataset::Deletion
                )
                .is_err()
        );
        for (old, new) in [
            (r#""dirty":false"#, r#""dirty":true"#),
            (r#""writerSchema":1"#, r#""writerSchema":0"#),
            (r#""lane":"machine""#, r#""lane":"controller""#),
        ] {
            let changed: Source = serde_json::from_str(&bytes.replace(old, new)).unwrap();
            assert!(
                changed
                    .validate(&source.revision, Dataset::Deletion)
                    .is_err()
            );
        }
    }

    #[test]
    fn incarnation_declaration_selects_each_writer_and_stays_closed() {
        let bytes = r#"{"schema":1,"component":"cowboy","lane":"machine","repository":"git@github.com:dravengarden/cowboy.git","revision":"0123456789012345678901234567890123456789","dirty":false,"workerGeneration":"worker-0123456789abcdef0123","sessionDeletionJournal":{"readerSchema":1,"writerSchema":1}}"#;
        let with = |declaration: &str| {
            format!(
                "{},\"sessionIncarnations\":{declaration}}}",
                &bytes[..bytes.len() - 1]
            )
        };
        let validate = |text: &str, dataset| {
            let source: Source = serde_json::from_str(text).unwrap();
            source.validate(&source.revision, dataset)
        };
        // The deletion writer tolerates an absent, reader-only or writer declaration.
        for text in [
            bytes.to_owned(),
            with(r#"{"readerSchema":1,"writerSchema":0}"#),
            with(r#"{"readerSchema":1,"writerSchema":1}"#),
        ] {
            validate(&text, Dataset::Deletion).unwrap();
        }
        for refused in [
            r#"{"readerSchema":0,"writerSchema":0}"#,
            r#"{"readerSchema":2,"writerSchema":0}"#,
            r#"{"readerSchema":1,"writerSchema":2}"#,
        ] {
            assert!(
                validate(&with(refused), Dataset::Deletion).is_err(),
                "{refused}"
            );
        }
        // The incarnation writer needs its own writer declaration, not merely a reader.
        validate(
            &with(r#"{"readerSchema":1,"writerSchema":1}"#),
            Dataset::Incarnation,
        )
        .unwrap();
        for text in [
            bytes.to_owned(),
            with(r#"{"readerSchema":1,"writerSchema":0}"#),
            with(r#"{"readerSchema":0,"writerSchema":1}"#),
        ] {
            assert!(validate(&text, Dataset::Incarnation).is_err());
        }
        // ...and the deletion writer it depends on.
        let deletion_reader_only = with(r#"{"readerSchema":1,"writerSchema":1}"#).replace(
            r#""sessionDeletionJournal":{"readerSchema":1,"writerSchema":1}"#,
            r#""sessionDeletionJournal":{"readerSchema":1,"writerSchema":0}"#,
        );
        assert!(validate(&deletion_reader_only, Dataset::Incarnation).is_err());
        for malformed in [
            r#"{"readerSchema":1}"#,
            r#"{"readerSchema":1,"writerSchema":0,"enableWriter":true}"#,
            r#"{"readerSchema":1,"readerSchema":1,"writerSchema":0}"#,
        ] {
            assert!(
                serde_json::from_str::<Source>(&with(malformed)).is_err(),
                "{malformed}"
            );
        }
    }

    #[test]
    fn authority_read_rejects_links_special_files_size_and_mutable_permissions() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("authority");
        let uid = std::fs::metadata(root.path()).unwrap().uid();
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(read_regular(&path, uid).unwrap(), b"{}");
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_regular(&link, uid).is_err());
        assert!(read_regular(root.path(), uid).is_err());
        assert!(read_regular(&path, uid.wrapping_add(1)).is_err());
        std::fs::write(&path, vec![0; LIMIT as usize + 1]).unwrap();
        assert!(read_regular(&path, uid).is_err());
        std::fs::write(&path, "{}").unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(read_regular(&path, uid).is_err());
    }
}

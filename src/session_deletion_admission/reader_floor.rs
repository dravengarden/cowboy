//! Monotonic portable reader floor; committed-state and bootstrap admission
//! remain closed. Independent administrator-selected older tools are not fenced.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

pub(crate) const NAME: &str = "portable-session-deletion-reader-floor.json";

pub(crate) fn require_absent_for_install(state: &Path) -> Result<()> {
    match std::fs::symlink_metadata(state.join(NAME)) {
        Ok(_) => bail!(
            "portable reader floor requires signed refresh with an authenticated selected host; bootstrap recovery remains closed"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("inspecting portable reader floor before installation"),
    }
}

#[cfg(feature = "machine-host")]
pub(crate) use record::{Floor, read, retain};

#[cfg(feature = "machine-host")]
mod record {
    use std::fs::{File, OpenOptions};
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    use std::path::{Component, Path, PathBuf};

    use anyhow::{Context as _, Result, ensure};
    use serde::{Deserialize, Serialize};
    use sha2::{Digest as _, Sha256};

    use super::NAME;
    use crate::machine_protocol::DesiredComponent;

    const LIMIT: u64 = 8192;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct Floor {
        schema: u32,
        state_dir: PathBuf,
        dataset: String,
        reader_schema: u32,
        publisher_key_sha256: String,
        pub(crate) anchor_version: String,
        pub(crate) anchor_digest: String,
        pub(crate) anchor_generation: String,
        pub(crate) anchor_proof_sha256: String,
    }

    impl Floor {
        pub(crate) fn new(
            state: &Path,
            desired: &DesiredComponent,
            publisher: &str,
            anchor_proof_sha256: String,
        ) -> Result<Self> {
            desired
                .validate_session_deletion_declaration()
                .map_err(anyhow::Error::msg)?;
            ensure!(
                desired.session_deletion_journal.is_some(),
                "portable reader floor requires a declared reader anchor"
            );
            let floor = Self {
                schema: 1,
                state_dir: state.canonicalize()?,
                dataset: "session-deletions".into(),
                reader_schema: 1,
                publisher_key_sha256: publisher_digest(publisher)?,
                anchor_version: desired.version.clone(),
                anchor_digest: desired.digest.to_ascii_lowercase(),
                anchor_generation: desired.generation.clone(),
                anchor_proof_sha256,
            };
            floor.validate(state)?;
            Ok(floor)
        }

        fn validate(&self, state: &Path) -> Result<()> {
            ensure!(
                self.schema == 1 && self.reader_schema == 1 && self.dataset == "session-deletions",
                "unsupported portable reader floor"
            );
            ensure!(
                self.state_dir == state.canonicalize()?,
                "portable reader floor belongs to another state directory"
            );
            let mut version = Path::new(&self.anchor_version).components();
            ensure!(
                matches!(version.next(), Some(Component::Normal(_))) && version.next().is_none(),
                "unsafe portable reader floor anchor version"
            );
            for digest in [
                &self.anchor_digest,
                &self.publisher_key_sha256,
                &self.anchor_proof_sha256,
            ] {
                ensure!(
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                    "invalid portable reader floor digest"
                );
            }
            ensure!(
                !self.anchor_generation.is_empty(),
                "portable reader floor has no anchor generation"
            );
            Ok(())
        }

        pub(crate) fn check_publisher(&self, publisher: &str) -> Result<()> {
            ensure!(
                self.publisher_key_sha256 == publisher_digest(publisher)?,
                "portable reader floor publisher differs from configured key"
            );
            Ok(())
        }

        pub(crate) fn anchor_path(&self, root: &Path) -> PathBuf {
            root.join("payloads/machine_host")
                .join(&self.anchor_version)
                .join(&self.anchor_digest)
        }
    }

    fn publisher_digest(publisher: &str) -> Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(crate::machine_auth::validate_public_key(publisher)?.as_bytes())
        ))
    }

    pub(crate) fn read(state: &Path) -> Result<Option<Floor>> {
        read_inner(state).context("reading portable reader floor")
    }

    fn read_inner(state: &Path) -> Result<Option<Floor>> {
        let path = state.join(NAME);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => require_private_file(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        require_private_file(&file.metadata()?)?;
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "portable reader floor exceeds size limit"
        );
        let floor: Floor = serde_json::from_slice(&bytes)?;
        floor.validate(state)?;
        Ok(Some(floor))
    }

    fn require_private_file(metadata: &std::fs::Metadata) -> Result<()> {
        ensure!(
            metadata.is_file()
                && metadata.uid() == rustix::process::geteuid().as_raw()
                && metadata.mode() & 0o077 == 0,
            "portable reader floor is not a private owned regular file"
        );
        Ok(())
    }

    fn sync_floor(state: &Path) -> Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(state.join(NAME))?;
        require_private_file(&file.metadata()?)?;
        file.sync_all()?;
        File::open(state)?.sync_all()?;
        if let Some(parent) = state.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }

    // The first validated anchor wins. Never overwrite, repair, remove or
    // advance a committed floor; failures may leave a floor fencing selection.
    pub(crate) fn retain(state: &Path, proposed: &Floor) -> Result<Floor> {
        proposed.validate(state)?;
        if let Some(floor) = read(state)? {
            sync_floor(state)?;
            return Ok(floor);
        }
        let bytes = serde_json::to_vec_pretty(proposed)?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "portable reader floor exceeds size limit"
        );
        let temporary = state.join(format!(
            ".portable-reader-floor-{:032x}.partial",
            rand::random::<u128>()
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            match std::fs::hard_link(&temporary, state.join(NAME)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let floor = read(state)?.context("portable reader floor disappeared after commit")?;
            sync_floor(state)?;
            Ok(floor)
        })();
        let _ = std::fs::remove_file(&temporary);
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn fixture(state: &Path) -> Floor {
            Floor {
                schema: 1,
                state_dir: state.canonicalize().unwrap(),
                dataset: "session-deletions".into(),
                reader_schema: 1,
                publisher_key_sha256: "a".repeat(64),
                anchor_version: "accepted".into(),
                anchor_digest: "b".repeat(64),
                anchor_generation: "g1".into(),
                anchor_proof_sha256: "c".repeat(64),
            }
        }

        #[test]
        fn floor_is_closed_bounded_and_never_repaired() {
            use std::os::unix::fs::PermissionsExt as _;
            for case in [
                "unknown",
                "duplicate",
                "wrong-schema",
                "wrong-owner",
                "wrong-dataset",
                "unsafe-version",
                "missing-anchor",
                "oversized",
                "link",
                "directory",
                "fifo",
                "shared-mode",
            ] {
                let state = tempfile::tempdir().unwrap();
                let floor = fixture(state.path());
                let path = state.path().join(NAME);
                let mut value = serde_json::to_value(&floor).unwrap();
                match case {
                    "unknown" => value["unexpected"] = true.into(),
                    "wrong-schema" => value["reader_schema"] = 0.into(),
                    "wrong-owner" => value["state_dir"] = "/foreign".into(),
                    "wrong-dataset" => value["dataset"] = "foreign".into(),
                    "unsafe-version" => value["anchor_version"] = "../foreign".into(),
                    "missing-anchor" => {
                        value.as_object_mut().unwrap().remove("anchor_generation");
                    }
                    _ => {}
                }
                let mut bytes = serde_json::to_vec(&value).unwrap();
                match case {
                    "duplicate" => {
                        bytes.pop();
                        bytes.extend_from_slice(b",\"schema\":1}");
                    }
                    "oversized" => bytes.resize(8193, b' '),
                    "link" => {
                        std::os::unix::fs::symlink(state.path().join("absent"), &path).unwrap()
                    }
                    "directory" => std::fs::create_dir(&path).unwrap(),
                    "fifo" => rustix::fs::mkfifoat(
                        rustix::fs::CWD,
                        &path,
                        rustix::fs::Mode::from_raw_mode(0o600),
                    )
                    .unwrap(),
                    _ => {}
                }
                if !matches!(case, "link" | "directory" | "fifo") {
                    std::fs::write(&path, &bytes).unwrap();
                    std::fs::set_permissions(
                        &path,
                        std::fs::Permissions::from_mode(if case == "shared-mode" {
                            0o644
                        } else {
                            0o600
                        }),
                    )
                    .unwrap();
                }
                let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
                assert!(read(state.path()).is_err(), "{case}");
                assert!(
                    retain(state.path(), &floor).is_err(),
                    "{case}: invalid floor was repaired"
                );
                assert_eq!(std::fs::symlink_metadata(&path).unwrap().file_type(), kind);
                if kind.is_file() {
                    assert_eq!(std::fs::read(&path).unwrap(), bytes);
                }
                assert_eq!(std::fs::read_dir(state.path()).unwrap().count(), 1);
            }
        }

        #[test]
        fn first_floor_is_retained_and_absent_reads_create_nothing() {
            use std::os::unix::fs::PermissionsExt as _;
            let state = tempfile::tempdir().unwrap();
            let absent = state.path().join("absent");
            assert!(read(&absent).unwrap().is_none());
            assert!(!absent.exists());
            let first = fixture(state.path());
            assert_eq!(retain(state.path(), &first).unwrap(), first);
            let bytes = std::fs::read(state.path().join(NAME)).unwrap();
            let mut second = first.clone();
            second.anchor_version = "new".into();
            assert_eq!(retain(state.path(), &second).unwrap(), first);
            assert_eq!(std::fs::read(state.path().join(NAME)).unwrap(), bytes);
            assert_eq!(
                std::fs::metadata(state.path().join(NAME))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(std::fs::read_dir(state.path()).unwrap().count(), 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_floor_entry_refuses_installation_without_writes() {
        for kind in ["file", "directory", "dangling"] {
            let state = tempfile::tempdir().unwrap();
            require_absent_for_install(&state.path().join("absent")).unwrap();
            let path = state.path().join(NAME);
            match kind {
                "file" => std::fs::write(&path, b"malformed").unwrap(),
                "directory" => std::fs::create_dir(&path).unwrap(),
                _ => std::os::unix::fs::symlink(state.path().join("absent"), &path).unwrap(),
            }
            assert!(require_absent_for_install(state.path()).is_err());
            assert_eq!(std::fs::read_dir(state.path()).unwrap().count(), 1);
        }
    }
}

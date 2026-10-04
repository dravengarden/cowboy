//! Offline restoration of the exact signed package bound by an existing floor.
//! Existing damaged generations and unrelated selections are never replaced.

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    restore_portable_host_anchor, restore_portable_host_anchor_with_quarantine,
};

#[cfg(not(target_os = "linux"))]
pub(crate) fn restore_portable_host_anchor(
    _state: &std::path::Path,
    _key: &std::path::Path,
    _manifest: &std::path::Path,
    _artifact: &std::path::Path,
) -> anyhow::Result<()> {
    anyhow::bail!("anchor package restoration requires Linux no-replace publication")
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn restore_portable_host_anchor_with_quarantine(
    _state: &std::path::Path,
    _key: &std::path::Path,
    _manifest: &std::path::Path,
    _artifact: &std::path::Path,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    anyhow::bail!("anchor quarantine restoration requires Linux atomic exchange")
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::fs::{DirBuilder, File, OpenOptions};
    use std::io::{Read as _, Write as _};
    use std::os::unix::fs::{
        DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
    };
    use std::path::{Component, Path, PathBuf};

    use anyhow::{Context as _, Result, ensure};
    use serde::Deserialize;
    use sha2::{Digest as _, Sha256};

    use super::super::cached_host::{authenticate_floor, read_regular};
    use crate::machine_protocol::{
        ArtifactFormat, ComponentId, ComponentKind, ComponentProbe, DesiredComponent,
        SessionDeletionReader,
    };
    use crate::session_deletion_admission::{self, reader_floor};

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Manifest {
        id: Id,
        version: String,
        generation: String,
        artifact_url: String,
        digest: String,
        #[serde(default)]
        artifact_format: ArtifactFormat,
        #[serde(default)]
        entrypoint: Option<String>,
        signature: String,
        session_deletion_journal: SessionDeletionReader,
        #[serde(default)]
        probe: Option<Probe>,
        #[serde(default)]
        automatic: bool,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Id {
        kind: ComponentKind,
        #[serde(default)]
        slot: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Probe {
        #[serde(default)]
        args: Vec<String>,
        #[serde(default = "probe_timeout")]
        timeout_ms: u64,
    }
    fn probe_timeout() -> u64 {
        10_000
    }

    impl From<Manifest> for DesiredComponent {
        fn from(value: Manifest) -> Self {
            Self {
                id: ComponentId {
                    kind: value.id.kind,
                    slot: value.id.slot,
                },
                version: value.version,
                generation: value.generation,
                artifact_url: value.artifact_url,
                digest: value.digest,
                artifact_format: value.artifact_format,
                entrypoint: value.entrypoint,
                signature: Some(value.signature),
                session_deletion_journal: Some(value.session_deletion_journal),
                probe: value.probe.map(|p| ComponentProbe {
                    args: p.args,
                    timeout_ms: p.timeout_ms,
                }),
                automatic: value.automatic,
            }
        }
    }

    struct Payload {
        bytes: Vec<u8>,
        mode: u32,
    }
    type Contents = BTreeMap<PathBuf, Option<Payload>>;

    fn contents(desired: &DesiredComponent, artifact: &[u8]) -> Result<(Contents, PathBuf)> {
        if desired.artifact_format == ArtifactFormat::Raw {
            ensure!(
                desired.entrypoint.is_none(),
                "raw recovery anchor must not declare an archive entrypoint"
            );
            return Ok((
                BTreeMap::from([(
                    PathBuf::from("bin"),
                    Some(Payload {
                        bytes: artifact.to_vec(),
                        mode: 0o755,
                    }),
                )]),
                PathBuf::from("bin"),
            ));
        }
        let entrypoint = PathBuf::from(
            desired
                .entrypoint
                .as_deref()
                .context("recovery archive requires entrypoint")?,
        );
        ensure!(
            !entrypoint.as_os_str().is_empty()
                && entrypoint
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "unsafe recovery entrypoint"
        );
        let decoder = flate2::read::GzDecoder::new(artifact).take(512 * 1024 * 1024 + 64 * 1024);
        let mut tree: Contents = BTreeMap::new();
        let mut remaining = 512 * 1024 * 1024;
        for entry in tar::Archive::new(decoder).entries()? {
            let mut entry = entry?;
            let kind = entry.header().entry_type();
            ensure!(
                kind.is_file() || kind.is_dir(),
                "recovery archive requires regular files and directories"
            );
            let mut relative = PathBuf::new();
            for part in entry.path()?.components() {
                match part {
                    Component::Normal(name) => relative.push(name),
                    Component::CurDir => {}
                    _ => anyhow::bail!("unsafe recovery archive path"),
                }
            }
            if relative.as_os_str().is_empty() {
                ensure!(kind.is_dir(), "empty recovery file path");
                continue;
            }
            ensure!(
                relative.as_os_str().as_encoded_bytes().len() <= 4096
                    && relative.components().count() <= 128,
                "recovery archive path exceeds bounds"
            );
            for parent in relative
                .ancestors()
                .skip(1)
                .filter(|p| !p.as_os_str().is_empty())
            {
                ensure!(
                    tree.entry(parent.to_path_buf()).or_insert(None).is_none(),
                    "recovery archive directory conflicts with file"
                );
            }
            if kind.is_dir() {
                ensure!(
                    tree.entry(relative).or_insert(None).is_none(),
                    "recovery directory conflicts with file"
                );
            } else {
                let mode = (entry.header().mode()? & 0o555) | 0o600;
                let mut bytes = Vec::new();
                (&mut entry).take(remaining + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= remaining,
                    "recovery archive expanded payload exceeds bounds"
                );
                remaining -= bytes.len() as u64;
                ensure!(
                    tree.insert(relative, Some(Payload { bytes, mode }))
                        .is_none(),
                    "duplicate recovery archive file"
                );
            }
            ensure!(
                tree.len() <= 65_536,
                "recovery archive entry count exceeds bounds"
            );
        }
        ensure!(
            tree.get(&entrypoint).is_some_and(Option::is_some),
            "recovery archive entrypoint is missing"
        );
        let executable = PathBuf::from("content").join(entrypoint);
        Ok((
            tree.into_iter()
                .map(|(p, v)| (PathBuf::from("content").join(p), v))
                .chain([(PathBuf::from("content"), None)])
                .collect(),
            executable,
        ))
    }

    fn check_paths(state: &Path, generation: &Path, executable: &Path) -> Result<()> {
        let root = state.join("components");
        for path in [
            root.clone(),
            root.join("active"),
            root.join("commands"),
            root.join("payloads"),
            root.join("payloads/machine_host"),
            generation
                .parent()
                .context("anchor has no parent")?
                .to_path_buf(),
        ] {
            match std::fs::symlink_metadata(path) {
                Ok(m) => ensure!(m.is_dir(), "recovery parent is not a regular directory"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        for (pointer, target) in [
            (root.join("active/machine_host"), generation),
            (root.join("commands/cowboy-machine"), executable),
        ] {
            match std::fs::symlink_metadata(&pointer) {
                Ok(m) => ensure!(
                    m.is_symlink()
                        && std::fs::read_link(pointer)?.as_os_str() == target.as_os_str(),
                    "anchor recovery refuses an existing different selection"
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }

    pub(crate) fn restore_portable_host_anchor(
        state: &Path,
        key: &Path,
        manifest: &Path,
        artifact: &Path,
    ) -> Result<()> {
        restore_anchor(state, key, manifest, artifact, false).map(|_| ())
    }

    pub(crate) fn restore_portable_host_anchor_with_quarantine(
        state: &Path,
        key: &Path,
        manifest: &Path,
        artifact: &Path,
    ) -> Result<Option<PathBuf>> {
        restore_anchor(state, key, manifest, artifact, true)
    }

    fn private_quarantine(path: &Path) -> Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(m) => ensure!(
                m.is_dir()
                    && m.uid() == rustix::process::geteuid().as_raw()
                    && m.permissions().mode() & 0o077 == 0,
                "anchor quarantine must be an owned private regular directory"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    fn restore_anchor(
        state: &Path,
        key: &Path,
        manifest: &Path,
        artifact: &Path,
        quarantine_damage: bool,
    ) -> Result<Option<PathBuf>> {
        session_deletion_admission::require_empty_portable_namespace(state)?;
        let state = state.canonicalize()?;
        let floor =
            reader_floor::read(&state)?.context("anchor recovery requires an existing floor")?;
        let floor_bytes = read_regular(&state.join(reader_floor::NAME), Some(8192))?;
        let publisher = String::from_utf8(read_regular(key, Some(16 * 1024))?)?;
        floor.check_publisher(&publisher)?;
        let manifest_bytes = read_regular(manifest, Some(64 * 1024))?;
        let desired: DesiredComponent = serde_json::from_slice::<Manifest>(&manifest_bytes)?.into();
        desired
            .validate_session_deletion_declaration()
            .map_err(anyhow::Error::msg)?;
        let proof = crate::component_proof::component_proof(&desired);
        ensure!(
            desired.version == floor.anchor_version
                && desired.generation == floor.anchor_generation
                && desired.digest.to_ascii_lowercase() == floor.anchor_digest
                && format!("{:x}", Sha256::digest(&proof)) == floor.anchor_proof_sha256,
            "recovery candidate differs from original floor proof"
        );
        ensure!(
            crate::machine_auth::verify(
                &publisher,
                &proof,
                desired.signature.as_deref().expect("signature present")
            )?,
            "recovery publisher signature rejected"
        );
        let artifact_bytes = read_regular(artifact, Some(256 * 1024 * 1024))?;
        ensure!(
            format!("{:x}", Sha256::digest(&artifact_bytes)) == floor.anchor_digest,
            "recovery artifact digest mismatch"
        );
        let (tree, relative_executable) = contents(&desired, &artifact_bytes)?;
        let root = state.join("components");
        let generation = floor.anchor_path(&root);
        let executable = generation.join(&relative_executable);
        check_paths(&state, &generation, &executable)?;
        let damaged = match std::fs::symlink_metadata(&generation) {
            Ok(m) => {
                if authenticate_floor(&root, &floor, &publisher).is_ok() {
                    return Ok(None);
                }
                ensure!(
                    quarantine_damage,
                    "damaged anchor requires explicit quarantine restoration"
                );
                ensure!(m.is_dir(), "damaged anchor must be a regular directory");
                Some((m.dev(), m.ino()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let quarantine = state.join("component-anchor-quarantine");
        if damaged.is_some() {
            private_quarantine(&quarantine)?;
        }
        let recheck = || -> Result<()> {
            session_deletion_admission::require_empty_portable_namespace(&state)?;
            ensure!(
                reader_floor::read(&state)?.as_ref() == Some(&floor)
                    && read_regular(&state.join(reader_floor::NAME), Some(8192))? == floor_bytes,
                "floor changed during anchor recovery"
            );
            let current_key = String::from_utf8(read_regular(key, Some(16 * 1024))?)?;
            floor.check_publisher(&current_key)?;
            check_paths(&state, &generation, &executable)
        };
        recheck()?;
        let parent = generation.parent().expect("validated anchor parent");
        let mut directory = state.clone();
        for part in parent.strip_prefix(&state)?.components() {
            ensure!(
                matches!(part, Component::Normal(_)),
                "unsafe recovery parent"
            );
            directory.push(part);
            match DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ensure!(
                    std::fs::symlink_metadata(&directory)?.is_dir(),
                    "recovery parent is not regular"
                ),
                Err(e) => return Err(e.into()),
            }
            File::open(directory.parent().context("recovery parent missing")?)?.sync_all()?;
        }
        let staging_parent = if damaged.is_some() {
            match DirBuilder::new().mode(0o700).create(&quarantine) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            private_quarantine(&quarantine)?;
            File::open(&state)?.sync_all()?;
            &quarantine
        } else {
            parent
        };
        let staged = staging_parent.join(if damaged.is_some() {
            format!(
                "anchor-{}-{:032x}.retained",
                floor.anchor_digest,
                rand::random::<u128>()
            )
        } else {
            format!(".anchor-recovery-{:032x}.partial", rand::random::<u128>())
        });
        DirBuilder::new().mode(0o700).create(&staged)?;
        for (relative, payload) in tree {
            let path = staged.join(relative);
            if let Some(payload) = payload {
                write_file(&path, &payload.bytes, payload.mode)?;
            } else {
                DirBuilder::new().mode(0o700).create(path)?;
            }
        }
        std::fs::set_permissions(
            staged.join(&relative_executable),
            std::fs::Permissions::from_mode(0o755),
        )?;
        write_file(&staged.join("manifest.json"), &manifest_bytes, 0o600)?;
        write_file(&staged.join("artifact"), &artifact_bytes, 0o600)?;
        super::super::host_payload::HostPayload::from_authenticated(&desired, &artifact_bytes)?
            .context("recovery candidate is not a host")?
            .verify(&staged)?;
        sync_tree(&staged)?;
        File::open(staging_parent)?.sync_all()?;
        recheck()?;
        if let Some(identity) = damaged {
            private_quarantine(&quarantine)?;
            let current = std::fs::symlink_metadata(&generation)?;
            ensure!(
                current.is_dir() && (current.dev(), current.ino()) == identity,
                "damaged anchor changed before quarantine exchange"
            );
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                &staged,
                rustix::fs::CWD,
                &generation,
                rustix::fs::RenameFlags::EXCHANGE,
            )?;
        } else {
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                &staged,
                rustix::fs::CWD,
                &generation,
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
        }
        File::open(parent)?.sync_all()?;
        File::open(staging_parent)?.sync_all()?;
        recheck()?;
        authenticate_floor(&root, &floor, &publisher)?;
        Ok(damaged.map(|_| staged))
    }

    fn sync_tree(path: &Path) -> Result<()> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                sync_tree(&entry.path())?;
            } else {
                File::open(entry.path())?.sync_all()?;
            }
        }
        File::open(path)?.sync_all()?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ambiguous_unsafe_and_non_regular_archives_refuse_before_staging() {
            let desired: DesiredComponent = serde_json::from_value::<Manifest>(serde_json::json!({
                "id": {"kind": "machine_host"}, "version": "anchor", "generation": "g1",
                "artifact_url": "https://unused.invalid", "digest": "0".repeat(64),
                "artifact_format": "tar_gz", "entrypoint": "bin/host", "signature": "fixture",
                "session_deletion_journal": {"reader_schema": 1, "writer_schema": 0}
            }))
            .unwrap()
            .into();
            for case in [
                "duplicate",
                "file-parent",
                "traversal",
                "symlink",
                "missing-entrypoint",
                "directory-file",
            ] {
                let encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                let mut builder = tar::Builder::new(encoder);
                let entries = match case {
                    "duplicate" => vec![
                        ("bin/host", tar::EntryType::Regular),
                        ("bin/host", tar::EntryType::Regular),
                    ],
                    "file-parent" => vec![
                        ("bin", tar::EntryType::Regular),
                        ("bin/host", tar::EntryType::Regular),
                    ],
                    "traversal" => vec![("../outside", tar::EntryType::Regular)],
                    "symlink" => vec![("bin/host", tar::EntryType::Symlink)],
                    "missing-entrypoint" => vec![("elsewhere", tar::EntryType::Regular)],
                    _ => vec![
                        ("bin/host", tar::EntryType::Directory),
                        ("bin/host", tar::EntryType::Regular),
                    ],
                };
                for (name, kind) in entries {
                    let bytes = if kind.is_file() {
                        b"payload".as_slice()
                    } else {
                        b"".as_slice()
                    };
                    let mut header = tar::Header::new_gnu();
                    header.set_size(bytes.len() as u64);
                    header.set_entry_type(kind);
                    header.set_mode(0o755);
                    header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
                    header.set_cksum();
                    builder.append(&header, bytes).unwrap();
                }
                let bytes = builder.into_inner().unwrap().finish().unwrap();
                assert!(contents(&desired, &bytes).is_err(), "{case}");
            }
        }

        #[test]
        fn linux_no_replace_publication_preserves_a_conflicting_generation() {
            let root = tempfile::tempdir().unwrap();
            let staged = root.path().join("staged");
            let accepted = root.path().join("accepted");
            std::fs::create_dir(&staged).unwrap();
            std::fs::create_dir(&accepted).unwrap();
            std::fs::write(staged.join("bin"), b"candidate").unwrap();
            std::fs::write(accepted.join("bin"), b"retained").unwrap();
            assert!(
                rustix::fs::renameat_with(
                    rustix::fs::CWD,
                    &staged,
                    rustix::fs::CWD,
                    &accepted,
                    rustix::fs::RenameFlags::NOREPLACE
                )
                .is_err()
            );
            assert_eq!(std::fs::read(staged.join("bin")).unwrap(), b"candidate");
            assert_eq!(std::fs::read(accepted.join("bin")).unwrap(), b"retained");
        }
    }
}

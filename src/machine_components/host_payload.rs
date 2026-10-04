//! Check host bytes against an authenticated artifact at staging or startup.
//! The caller authenticates the envelope; this is not a concurrent-writer fence.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, bail};
use sha2::{Digest as _, Sha256};

use crate::machine_protocol::{ArtifactFormat, ComponentKind, DesiredComponent};

pub(super) enum HostPayload {
    Raw(Vec<u8>),
    Archive(Contents),
}

impl HostPayload {
    // Caller has already verified the publisher signature and artifact digest.
    // Prepare expectations before extraction, refusing ambiguous host archives.
    pub(super) fn from_authenticated(
        desired: &DesiredComponent,
        authenticated: &[u8],
    ) -> anyhow::Result<Option<Self>> {
        if desired.id.kind != ComponentKind::MachineHost {
            return Ok(None);
        }
        Ok(Some(match desired.artifact_format {
            ArtifactFormat::Raw => Self::Raw(Sha256::digest(authenticated).to_vec()),
            ArtifactFormat::TarGz => Self::Archive(archive_contents(authenticated)?),
        }))
    }

    pub(super) fn verify(&self, generation: &Path) -> anyhow::Result<()> {
        match self {
            Self::Raw(expected) => {
                let digest = regular_digest(&generation.join("bin"))?;
                if &digest != expected {
                    bail!("staged Machine host bytes differ from signed artifact");
                }
            }
            Self::Archive(expected) => {
                let mut actual = BTreeMap::new();
                let root = generation.join("content");
                directory_contents(&root, Path::new(""), &mut actual)?;
                if &actual != expected {
                    bail!("staged Machine host tree differs from signed archive");
                }
            }
        }
        Ok(())
    }
}

type Contents = BTreeMap<PathBuf, Option<Vec<u8>>>;

fn archive_contents(bytes: &[u8]) -> anyhow::Result<Contents> {
    let mut contents = Contents::new();
    let decoder = flate2::read::GzDecoder::new(bytes);
    for entry in tar::Archive::new(decoder).entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            bail!("Machine host archive permits only regular files and directories");
        }
        let mut path = PathBuf::new();
        for part in entry.path()?.components() {
            match part {
                Component::Normal(name) => path.push(name),
                Component::CurDir => {}
                _ => bail!("Machine host archive contains an unsafe path"),
            }
        }
        if path.as_os_str().is_empty() {
            if kind.is_dir() {
                continue;
            }
            bail!("Machine host archive file has an empty path");
        }
        for parent in path.ancestors().skip(1) {
            if !parent.as_os_str().is_empty()
                && contents
                    .entry(parent.to_path_buf())
                    .or_insert(None)
                    .is_some()
            {
                bail!("Machine host archive directory conflicts with a file");
            }
        }
        if kind.is_dir() {
            if contents.entry(path).or_insert(None).is_some() {
                bail!("Machine host archive directory conflicts with a file");
            }
        } else if contents
            .insert(path, Some(reader_digest(&mut entry)?))
            .is_some()
        {
            bail!("Machine host archive contains a duplicate or conflicting file");
        }
    }
    Ok(contents)
}

fn directory_contents(root: &Path, relative: &Path, contents: &mut Contents) -> anyhow::Result<()> {
    let directory = root.join(relative);
    if !std::fs::symlink_metadata(&directory)?.is_dir() {
        bail!("staged Machine host directory is not a regular directory");
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = relative.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            contents.insert(path.clone(), None);
            directory_contents(root, &path, contents)?;
        } else if metadata.is_file() {
            contents.insert(path, Some(regular_digest(&entry.path())?));
        } else {
            bail!("staged Machine host tree contains a non-regular entry");
        }
    }
    Ok(())
}

fn regular_digest(path: &Path) -> anyhow::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt as _;

    if !std::fs::symlink_metadata(path)?.is_file() {
        bail!("staged Machine host payload is not a regular file");
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .context("opening staged Machine host bytes without following links")?;
    if !file.metadata()?.is_file() {
        bail!("staged Machine host payload is not a regular file");
    }
    reader_digest(&mut file)
}

fn reader_digest(reader: &mut impl std::io::Read) -> anyhow::Result<Vec<u8>> {
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = reader.read(&mut buffer)?;
        if length == 0 {
            return Ok(hash.finalize().to_vec());
        }
        hash.update(&buffer[..length]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_and_special_archive_entries_are_not_host_payloads() {
        for case in [
            "duplicate",
            "normalized-duplicate",
            "file-parent",
            "directory-file",
            "fifo",
        ] {
            let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let paths = match case {
                "duplicate" => ["bin/host", "bin/host"],
                "normalized-duplicate" => ["bin/host", "./bin/host"],
                "file-parent" => ["bin", "bin/host"],
                "directory-file" => ["bin/host", "bin"],
                "fifo" => ["bin/host", "pipe"],
                _ => unreachable!(),
            };
            for path in paths {
                let mut header = tar::Header::new_gnu();
                let bytes: &[u8] = if case == "fifo" && path == "pipe" {
                    header.set_entry_type(tar::EntryType::Fifo);
                    b""
                } else {
                    header.set_entry_type(tar::EntryType::Regular);
                    b"payload"
                };
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                archive.append_data(&mut header, path, bytes).unwrap();
            }
            let bytes = archive.into_inner().unwrap().finish().unwrap();
            assert!(archive_contents(&bytes).is_err(), "{case}");
        }
    }
}

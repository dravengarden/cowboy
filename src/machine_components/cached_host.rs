//! Read-only authentication before the portable launcher selects cached code.
//! The installer-selected bootstrap/key remain trusted; this is not a floor,
//! signed bootstrap admission, or a fence against concurrent administrators.

use std::io::Read as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, bail, ensure};
use sha2::{Digest as _, Sha256};

use super::{component_executable, component_proof, host_payload::HostPayload};
use crate::machine_protocol::{ComponentKind, DesiredComponent};

pub(crate) fn check_portable_host_cache(state: &Path, key: Option<&Path>) -> anyhow::Result<()> {
    let root = state.join("components");
    // Do not construct a ComponentStore: this check creates no cache or stores.
    for path in [&root, &root.join("active"), &root.join("commands")] {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => ensure!(
                metadata.is_dir(),
                "Machine host selection directory is not regular"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let active = root.join("active/machine_host");
    let command = root.join("commands/cowboy-machine");
    let present = |path: &Path| -> anyhow::Result<bool> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_symlink(),
                    "Machine host selection pointer is not a symlink"
                );
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    };
    match (present(&active)?, present(&command)?) {
        (false, false) => return Ok(()),
        (true, true) => {}
        _ => bail!("Machine host selection pointers are incomplete"),
    }
    let root = root.canonicalize()?;
    let generation = active
        .canonicalize()
        .context("resolving cached Machine host")?;
    let publisher =
        std::fs::read_to_string(key.context("cached Machine host requires a publisher key")?)?;
    let desired = verify_generation(&root, &generation, &publisher)?;
    let executable = component_executable(&generation, &desired)?;
    ensure!(
        command.canonicalize()? == executable
            && std::fs::metadata(&executable)?.permissions().mode() & 0o111 != 0,
        "Machine host command does not select the authenticated executable"
    );
    rustix::fs::faccessat(
        rustix::fs::CWD,
        &executable,
        rustix::fs::Access::EXEC_OK,
        rustix::fs::AtFlags::EACCESS,
    )
    .context("cached Machine host is not executable by the launcher")?;
    Ok(())
}

pub(super) fn verify_generation(
    root: &Path,
    generation: &Path,
    publisher: &str,
) -> anyhow::Result<DesiredComponent> {
    let root = root.canonicalize()?;
    let generation = generation.canonicalize()?;
    let relative = generation
        .strip_prefix(&root)
        .context("cached Machine host is outside component store")?;
    let mut directory = root.clone();
    for part in relative.components() {
        ensure!(
            matches!(part, Component::Normal(_)),
            "unsafe cached Machine host directory"
        );
        directory.push(part);
        ensure!(
            std::fs::symlink_metadata(&directory)?.is_dir(),
            "cached Machine host directory is not regular"
        );
    }
    let manifest = read_regular(&generation.join("manifest.json"), Some(64 * 1024))?;
    let desired: DesiredComponent = serde_json::from_slice(&manifest)?;
    ensure!(
        desired.id.kind == ComponentKind::MachineHost && desired.id.slot.is_empty(),
        "cached component is not the singleton Machine host"
    );
    let mut version = Path::new(&desired.version).components();
    ensure!(
        matches!(version.next(), Some(Component::Normal(_))) && version.next().is_none(),
        "unsafe cached Machine host version"
    );
    ensure!(
        desired.digest.len() == 64 && desired.digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid cached Machine host digest"
    );
    let expected: PathBuf = root
        .join("payloads/machine_host")
        .join(&desired.version)
        .join(desired.digest.to_ascii_lowercase());
    ensure!(
        generation == expected,
        "cached Machine host manifest does not match its generation path"
    );
    desired
        .validate_session_deletion_declaration()
        .map_err(anyhow::Error::msg)?;
    ensure!(
        crate::machine_auth::verify(
            publisher,
            &component_proof(&desired),
            desired
                .signature
                .as_deref()
                .context("cached Machine host is unsigned")?
        )?,
        "cached Machine host signature is invalid"
    );
    let artifact = read_regular(&generation.join("artifact"), None)?;
    ensure!(
        format!("{:x}", Sha256::digest(&artifact)) == desired.digest.to_ascii_lowercase(),
        "cached Machine host artifact digest mismatch"
    );
    HostPayload::from_authenticated(&desired, &artifact)?
        .context("cached component is not a host")?
        .verify(&generation)?;
    Ok(desired)
}

fn read_regular(path: &Path, limit: Option<u64>) -> anyhow::Result<Vec<u8>> {
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "cached Machine host proof is not a regular file"
    );
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "cached Machine host proof is not a regular file"
    );
    let mut bytes = Vec::new();
    if let Some(limit) = limit {
        file.take(limit + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= limit,
            "cached Machine host manifest exceeds limit"
        );
    } else {
        file.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}

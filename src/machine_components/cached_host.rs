//! Read-only authentication before the portable launcher selects cached code.
//! An established reader floor retains its signed anchor and forbids downgrade.
//! Explicit restoration selects only the intact signed floor anchor.
//! Bootstrap fallback, lost-anchor recovery and administrator fencing stay closed.

use std::io::Read as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, bail, ensure};
use sha2::{Digest as _, Sha256};

use super::{component_executable, component_proof, host_payload::HostPayload};
use crate::machine_protocol::{ComponentKind, DesiredComponent};
use crate::session_deletion_admission::reader_floor::{self, Floor};

pub(crate) fn check_portable_host_cache(state: &Path, key: Option<&Path>) -> anyhow::Result<()> {
    let floor = reader_floor::read(state)?;
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
        (false, false) if floor.is_none() => return Ok(()),
        (false, false) => bail!("portable reader floor refuses unadmitted bootstrap fallback"),
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
    if let Some(floor) = &floor {
        authenticate_floor(&root, floor, &publisher)?;
        require_declared_reader(&desired)?;
    }
    let executable = component_executable(&generation, &desired)?;
    ensure!(
        command.canonicalize()? == executable
            && std::fs::metadata(&executable)?.permissions().mode() & 0o111 != 0,
        "Machine host command does not select the authenticated executable"
    );
    rustix::fs::accessat(
        rustix::fs::CWD,
        &executable,
        rustix::fs::Access::EXEC_OK,
        rustix::fs::AtFlags::EACCESS,
    )
    .context("cached Machine host is not executable by the launcher")?;
    Ok(())
}

fn require_declared_reader(desired: &DesiredComponent) -> anyhow::Result<()> {
    desired
        .validate_session_deletion_declaration()
        .map_err(anyhow::Error::msg)?;
    ensure!(
        desired.session_deletion_journal.is_some(),
        "portable reader floor refuses undeclared Machine host"
    );
    Ok(())
}

fn proof_digest(desired: &DesiredComponent) -> String {
    format!("{:x}", Sha256::digest(component_proof(desired)))
}

pub(super) fn authenticate_floor(
    root: &Path,
    floor: &Floor,
    publisher: &str,
) -> anyhow::Result<()> {
    floor.check_publisher(publisher)?;
    let anchor = verify_generation(root, &floor.anchor_path(root), publisher)
        .context("authenticating portable reader floor anchor")?;
    require_declared_reader(&anchor)?;
    ensure!(
        anchor.generation == floor.anchor_generation
            && proof_digest(&anchor) == floor.anchor_proof_sha256,
        "portable reader floor anchor differs from accepted signed proof"
    );
    Ok(())
}

pub(super) fn check_floor_candidate(
    root: &Path,
    desired: &DesiredComponent,
    publisher: Option<&str>,
) -> anyhow::Result<()> {
    let state = root
        .parent()
        .context("component store has no Machine state parent")?;
    if let Some(floor) = reader_floor::read(state)? {
        require_declared_reader(desired)?;
        authenticate_floor(
            root,
            &floor,
            publisher.context("portable reader floor requires publisher key")?,
        )?;
        if desired.version == floor.anchor_version
            && desired.digest.to_ascii_lowercase() == floor.anchor_digest
        {
            ensure!(
                proof_digest(desired) == floor.anchor_proof_sha256,
                "portable reader floor refuses replacement of its accepted anchor proof"
            );
        }
    }
    Ok(())
}

pub(super) fn retain_reader_floor(
    root: &Path,
    desired: &DesiredComponent,
    publisher: &str,
) -> anyhow::Result<()> {
    if desired.session_deletion_journal.is_none() {
        return Ok(());
    }
    require_declared_reader(desired)?;
    let state = root
        .parent()
        .context("component store has no Machine state parent")?;
    let proposed = Floor::new(state, desired, publisher, proof_digest(desired))?;
    let floor = reader_floor::retain(state, &proposed)?;
    authenticate_floor(root, &floor, publisher)
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
    HostPayload::verify_cached(&desired, &generation.join("artifact"), &generation)?;
    Ok(desired)
}

pub(super) fn read_regular(path: &Path, limit: Option<u64>) -> anyhow::Result<Vec<u8>> {
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

fn recovery_target(state: &Path, key: &Path) -> anyhow::Result<(PathBuf, PathBuf, Vec<u8>)> {
    crate::session_deletion_admission::require_empty_portable_namespace(state)?;
    let floor = reader_floor::read(state)?
        .context("floor selection restoration requires an existing floor")?;
    let root = state.join("components");
    ensure!(
        std::fs::symlink_metadata(&root)?.is_dir(),
        "component recovery root is not regular"
    );
    let publisher = String::from_utf8(read_regular(key, Some(16 * 1024))?)?;
    authenticate_floor(&root, &floor, &publisher)?;
    let generation = floor.anchor_path(&root).canonicalize()?;
    let desired = verify_generation(&root, &generation, &publisher)?;
    let executable = component_executable(&generation, &desired)?;
    rustix::fs::accessat(
        rustix::fs::CWD,
        &executable,
        rustix::fs::Access::EXEC_OK,
        rustix::fs::AtFlags::EACCESS,
    )?;
    for (directory, name, target) in [
        ("active", "machine_host", &generation),
        ("commands", "cowboy-machine", &executable),
    ] {
        let parent = root.join(directory);
        match std::fs::symlink_metadata(&parent) {
            Ok(metadata) => ensure!(
                metadata.is_dir(),
                "recovery selection directory is not regular"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        let link = parent.join(name);
        match std::fs::symlink_metadata(&link) {
            Ok(metadata) => ensure!(
                metadata.is_symlink() && link.canonicalize()? == *target,
                "floor restoration refuses an existing different selection"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok((
        generation,
        executable,
        read_regular(&state.join(reader_floor::NAME), Some(8192))?,
    ))
}

/// Administrator-invoked, idempotent repair. Never overwrite a pointer or floor;
/// an interruption between links remains fail-closed and can be resumed.
pub(crate) fn restore_portable_host_selection(state: &Path, key: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    let accepted = recovery_target(state, key)?;
    let root = state.join("components");
    for (directory, name, target) in [
        ("active", "machine_host", &accepted.0),
        ("commands", "cowboy-machine", &accepted.1),
    ] {
        ensure!(
            recovery_target(state, key)? == accepted,
            "floor recovery evidence changed before publication"
        );
        let parent = root.join(directory);
        match std::fs::DirBuilder::new().mode(0o700).create(&parent) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        ensure!(
            recovery_target(state, key)? == accepted,
            "floor recovery evidence changed during publication"
        );
        match std::os::unix::fs::symlink(target, parent.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        std::fs::File::open(&parent)?.sync_all()?;
        std::fs::File::open(&root)?.sync_all()?;
    }
    ensure!(
        recovery_target(state, key)? == accepted,
        "floor recovery evidence changed after publication"
    );
    check_portable_host_cache(state, Some(key))?;
    std::fs::File::open(state)?.sync_all()?;
    Ok(())
}

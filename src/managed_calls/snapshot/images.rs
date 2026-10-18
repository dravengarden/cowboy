//! An image call snapshots only its explicit files, not the repository.
//! Deliveries live beside the read-only workspace, owned by the Machine.

use super::*;
use crate::managed_calls::media::{Delivery, MAX_IMAGE_BYTES, MAX_IMAGES};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};
use std::io::Read as _;

fn read_input(root: &Path, relative: &str) -> Result<Vec<u8>, &'static str> {
    if !matches!(
        Path::new(relative)
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp")
    ) {
        return Err("context_unsupported");
    }
    use rustix::fs::{Mode, OFlags, openat};
    let mut dir = std::fs::File::open(root).map_err(|_| "context_unreadable")?;
    let parts: Vec<_> = relative.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        dir = openat(
            &dir,
            *part,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(std::fs::File::from)
        .map_err(|_| "context_unsupported")?;
    }
    let file = openat(
        &dir,
        parts[parts.len() - 1],
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(std::fs::File::from)
    .map_err(|_| "context_unreadable")?;
    let metadata = file.metadata().map_err(|_| "context_unreadable")?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err("context_unsupported");
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "context_unreadable")?;
    // Native view_image owns decoding input formats; a typed image signature
    // avoids copying arbitrary source/text and project policy into this profile.
    if bytes.len() > MAX_IMAGE_BYTES
        || !(bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || bytes.starts_with(b"\xff\xd8\xff")
            || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")))
    {
        return Err("context_unsupported");
    }
    Ok(bytes)
}

pub(super) fn prepare(
    directory: &Path,
    source: &Path,
    round: &ChildRound,
) -> Result<ChildPrepared, &'static str> {
    if round.files.len() > MAX_IMAGES || round.output_schema.is_some() {
        return Err("invalid_request");
    }
    let mut captured = Vec::new();
    for file in &round.files {
        captured.push((file, read_input(source, file)?));
    }
    for (file, bytes) in &captured {
        if read_input(source, file)? != *bytes {
            return Err("input_changed");
        }
    }
    let revision = super::super::canonical_digest(&serde_json::json!({
        "profile":"image_v1", "files":captured.iter().map(|(name, bytes)|
            serde_json::json!({"name":name,"sha256":format!("{:x}", Sha256::digest(bytes))})).collect::<Vec<_>>()
    }));
    let workspace = directory.join("workspace");
    if workspace.exists() {
        private_directory(&workspace).map_err(|_| "identity_mismatch")?;
        std::fs::remove_dir_all(&workspace).map_err(|_| "preparation_failed")?;
    }
    private_directory(&workspace).map_err(|_| "preparation_failed")?;
    for (name, bytes) in captured {
        let mut destination = workspace.clone();
        let parts: Vec<_> = name.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            destination.push(part);
            private_directory(&destination).map_err(|_| "preparation_failed")?;
        }
        write_private(&workspace.join(name), &bytes).map_err(|_| "preparation_failed")?;
    }
    let marker = RoundMarker {
        schema: 1,
        call_id: round.call_id.clone(),
        input_revision: revision.clone(),
        head: String::new(),
        index_tree: String::new(),
        worktree_tree: String::new(),
        output_schema: None,
    };
    write_private(
        &directory.join("round.json"),
        &serde_json::to_vec(&marker).map_err(|_| "preparation_failed")?,
    )
    .map_err(|_| "preparation_failed")?;
    Ok(ChildPrepared {
        cwd: workspace.display().to_string(),
        input_revision: revision,
        head: String::new(),
        index_tree: String::new(),
        worktree_tree: String::new(),
    })
}

/// No caller-chosen destination: exact parent/child/current round, fixed
/// digest filename, private Machine directory. Replay verifies existing bytes.
pub fn deliver(managed: &Path, machine: &str, input: &Delivery) -> Result<String, &'static str> {
    if ![
        &input.parent_session_id,
        &input.child_session_id,
        &input.call_id,
    ]
    .into_iter()
    .all(|id| crate::managed_calls::valid_id(id))
        || input.data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4
    {
        return Err("invalid_request");
    }
    let directory = managed.join(&input.child_session_id);
    let _lock = ChildLock::acquire(&directory.display().to_string()).ok_or("child_busy")?;
    private_directory(&directory).map_err(|_| "identity_mismatch")?;
    let identity = read_private(&directory.join("snapshot.json"), 16 * 1024)
        .and_then(|bytes| serde_json::from_slice::<ManagedChildV1>(&bytes).ok())
        .ok_or("identity_mismatch")?;
    let round =
        super::super::round::read_round(&directory.join("workspace")).ok_or("identity_mismatch")?;
    if identity.parent_session_id != input.parent_session_id
        || identity.session_id != input.child_session_id
        || identity.machine_id != machine
        || identity.profile != cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1
        || round.call_id != input.call_id
    {
        return Err("identity_mismatch");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&input.data)
        .map_err(|_| "artifact_invalid")?;
    if crate::managed_calls::media::inspect(&bytes)? != input.artifact {
        return Err("artifact_invalid");
    }
    let artifacts = directory.join("artifacts");
    private_directory(&artifacts).map_err(|_| "artifact_storage_failed")?;
    let path = artifacts.join(&input.artifact.id);
    if path.exists() {
        if read_private(&path, MAX_IMAGE_BYTES as u64).as_deref() != Some(bytes.as_slice()) {
            return Err("artifact_conflict");
        }
    } else {
        write_private(&path, &bytes).map_err(|_| "artifact_storage_failed")?;
    }
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_calls::media;

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut writer = png::Encoder::new(&mut bytes, 1, 1).write_header().unwrap();
        writer.write_image_data(&[1]).unwrap();
        drop(writer);
        bytes
    }

    #[tokio::test]
    async fn images_copy_only_explicit_inputs_and_deliver_to_the_exact_owned_round() {
        let base = tempfile::tempdir().unwrap();
        let source = base.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("private-source.rs"), "not image context").unwrap();
        let bytes = png();
        std::fs::write(source.join("input.png"), &bytes).unwrap();
        let roots = crate::session_workspace::WorktreeRoots::single(base.path().to_owned());
        let managed = base.path().join("managed");
        let mut round = ChildRound {
            child_session_id: "child".into(),
            parent_session_id: "parent".into(),
            call_id: "call-1".into(),
            workspace_id: "project".into(),
            source_cwd: source.display().to_string(),
            files: vec!["input.png".into()],
            output_schema: None,
            profile: cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1,
        };
        let prepared = super::super::prepare(&managed, "machine", &round, &roots, &[])
            .await
            .unwrap();
        let workspace = PathBuf::from(&prepared.cwd);
        assert_eq!(std::fs::read_dir(&workspace).unwrap().count(), 1);
        assert_eq!(std::fs::read(workspace.join("input.png")).unwrap(), bytes);
        assert!(!workspace.join("private-source.rs").exists());
        let delivery = Delivery {
            parent_session_id: "parent".into(),
            child_session_id: "child".into(),
            call_id: "call-1".into(),
            artifact: media::inspect(&bytes).unwrap(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        };
        let path = deliver(&managed, "machine", &delivery).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(deliver(&managed, "machine", &delivery).unwrap(), path);
        let mut wrong = delivery.clone();
        wrong.parent_session_id = "other".into();
        assert!(deliver(&managed, "machine", &wrong).is_err());
        wrong = delivery.clone();
        wrong.call_id = "call-2".into();
        assert!(deliver(&managed, "machine", &wrong).is_err());
        wrong = delivery.clone();
        wrong.artifact.sha256 = "0".repeat(64);
        assert!(deliver(&managed, "machine", &wrong).is_err());
        std::fs::write(&path, b"changed").unwrap();
        assert_eq!(
            deliver(&managed, "machine", &delivery).unwrap_err(),
            "artifact_conflict"
        );
        round.call_id = "call-2".into();
        round.files.clear();
        let next = super::super::prepare(&managed, "machine", &round, &roots, &[])
            .await
            .unwrap();
        assert_ne!(next.input_revision, prepared.input_revision);
        assert_eq!(std::fs::read_dir(&workspace).unwrap().count(), 0);
        assert!(Path::new(&path).exists());
        assert!(deliver(&managed, "machine", &delivery).is_err());
    }

    #[test]
    fn input_capture_never_follows_a_symlink_or_copies_a_text_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("input.png"), png()).unwrap();
        std::fs::write(root.path().join("fake.png"), b"private text").unwrap();
        std::os::unix::fs::symlink("input.png", root.path().join("link.png")).unwrap();
        std::os::unix::fs::symlink(root.path(), root.path().join("dir")).unwrap();
        assert!(read_input(root.path(), "input.png").is_ok());
        for path in ["fake.png", "link.png", "dir/input.png"] {
            assert!(read_input(root.path(), path).is_err());
        }
    }
}

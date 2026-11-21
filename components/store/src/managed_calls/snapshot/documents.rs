//! A context ask's round workspace holds only the Controller-exported history
//! documents: no repository capture, no parent files.

use super::*;
use crate::managed_calls::history::DOCUMENT_DIR;
use sha2::{Digest as _, Sha256};

pub(super) fn prepare(directory: &Path, round: &ChildRound) -> Result<ChildPrepared, &'static str> {
    if round.profile != cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1
        || !round.files.is_empty()
        || round.output_schema.is_some()
    {
        return Err("invalid_request");
    }
    let revision = super::super::canonical_digest(&serde_json::json!({
        "profile": "read_only_v1",
        "documents": round.documents.iter().map(|document| serde_json::json!({
            "name": document.name,
            "sha256": format!("{:x}", Sha256::digest(document.text.as_bytes())),
        })).collect::<Vec<_>>(),
    }));
    let workspace = directory.join("workspace");
    if workspace.exists() {
        private_directory(&workspace).map_err(|_| "identity_mismatch")?;
        std::fs::remove_dir_all(&workspace).map_err(|_| "preparation_failed")?;
    }
    private_directory(&workspace).map_err(|_| "preparation_failed")?;
    let history = workspace.join(DOCUMENT_DIR);
    private_directory(&history).map_err(|_| "preparation_failed")?;
    for document in &round.documents {
        // Names are validated flat `<id>.txt`, so this never leaves `history`.
        write_private(&history.join(&document.name), document.text.as_bytes())
            .map_err(|_| "preparation_failed")?;
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
    // The marker is written last: an existing marker proves the documents are.
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
        files: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_calls::history::ContextDocument;

    fn round(call: &str, text: &str) -> ChildRound {
        ChildRound {
            child_session_id: "child-1".into(),
            parent_session_id: "parent-1".into(),
            call_id: call.into(),
            workspace_id: "workspace-1".into(),
            source_cwd: "/nonexistent".into(),
            files: Vec::new(),
            output_schema: None,
            profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
            documents: vec![ContextDocument {
                name: "sess-1.txt".into(),
                text: text.into(),
            }],
        }
    }

    #[test]
    fn writes_only_the_documents_and_replaces_an_earlier_round() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("child-1");
        private_directory(&directory).unwrap();
        let first = prepare(&directory, &round("call-1", "[1] user: hi\n")).unwrap();
        let workspace = Path::new(&first.cwd);
        std::fs::write(workspace.join("stale"), b"x").unwrap();
        let second = prepare(&directory, &round("call-2", "[2] user: bye\n")).unwrap();
        assert_eq!(first.cwd, second.cwd);
        assert_ne!(first.input_revision, second.input_revision);
        let entries: Vec<_> = std::fs::read_dir(workspace)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [DOCUMENT_DIR]);
        assert_eq!(
            std::fs::read_to_string(workspace.join("history/sess-1.txt")).unwrap(),
            "[2] user: bye\n"
        );
        let marker = read_private(&directory.join("round.json"), MAX_ROUND_BYTES).unwrap();
        let marker: RoundMarker = serde_json::from_slice(&marker).unwrap();
        assert_eq!(marker.call_id, "call-2");
        let mut images = round("call-3", "x");
        images.profile = cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1;
        assert_eq!(prepare(&directory, &images).unwrap_err(), "invalid_request");
    }
}

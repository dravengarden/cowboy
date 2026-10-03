//! Temporary closed gate for portable host selection, not reader admission.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

pub(crate) fn require_empty_portable_namespace(state: &Path) -> Result<()> {
    let namespace = state.join("session-deletions");
    match std::fs::symlink_metadata(&namespace) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => bail!("portable Session deletion namespace is not an owned directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("inspecting portable Session deletion namespace"),
    }
    match std::fs::symlink_metadata(namespace.join("deletions.json")) {
        Ok(_) => bail!(
            "portable Session deletion reader admission is not established; committed journal refuses host replacement and bootstrap selection"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("inspecting portable committed Session deletion entry"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_empty_and_staging_only_namespaces_are_read_only() {
        let state = tempfile::tempdir().unwrap();
        require_empty_portable_namespace(state.path()).unwrap();
        assert_eq!(std::fs::read_dir(state.path()).unwrap().count(), 0);
        let root = state.path().join("session-deletions");
        std::fs::create_dir(&root).unwrap();
        require_empty_portable_namespace(state.path()).unwrap();
        std::fs::write(root.join(".pending-fixture"), "not committed").unwrap();
        require_empty_portable_namespace(state.path()).unwrap();
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 1);
    }

    #[test]
    fn every_committed_entry_and_invalid_namespace_refuses_without_changes() {
        for case in [
            "file",
            "directory",
            "dangling",
            "namespace-file",
            "namespace-link",
        ] {
            let state = tempfile::tempdir().unwrap();
            let root = state.path().join("session-deletions");
            match case {
                "namespace-file" => std::fs::write(&root, "invalid").unwrap(),
                "namespace-link" => {
                    std::os::unix::fs::symlink(state.path().join("absent"), &root).unwrap();
                }
                _ => {
                    std::fs::create_dir(&root).unwrap();
                    let committed = root.join("deletions.json");
                    match case {
                        "file" => std::fs::write(committed, "{}").unwrap(),
                        "directory" => std::fs::create_dir(committed).unwrap(),
                        "dangling" => {
                            std::os::unix::fs::symlink(state.path().join("absent"), committed)
                                .unwrap();
                        }
                        _ => unreachable!(),
                    }
                }
            }
            let original = std::fs::symlink_metadata(&root).unwrap();
            assert!(
                require_empty_portable_namespace(state.path()).is_err(),
                "{case}"
            );
            assert_eq!(
                std::fs::symlink_metadata(root).unwrap().file_type(),
                original.file_type()
            );
        }
    }
}

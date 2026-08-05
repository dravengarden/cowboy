//! Shared trust rules for configured Machine workspace roots.

use std::path::{Component, Path, PathBuf};

pub(crate) fn resolve_configured_root(path: &Path) -> std::io::Result<(PathBuf, bool)> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "workspace path must be normalized and absolute",
        ));
    }

    let mut cursor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(cursor) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok((canonical, missing.is_empty()));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "workspace path has no existing ancestor",
                    )
                })?;
                missing.push(name.to_os_string());
                cursor = cursor.parent().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "workspace path has no existing ancestor",
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn canonical_target_within_root(target: &Path, configured_root: &Path) -> bool {
    configured_root.canonicalize().is_ok_and(|active_root| {
        active_root == configured_root
            && active_root.is_dir()
            && (target == active_root || target.starts_with(&active_root))
    })
}

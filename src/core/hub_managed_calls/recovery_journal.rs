//! Atomic local-disk journal for unresolved durable recovery-draft parks.
//!
//! When a `RemoteRuntime` disconnects or an ambiguous prompt rejection occurs
//! while PostgreSQL is temporarily unavailable, the prompt payload must survive
//! graceful Controller shutdown/restart without replaying it to any worker.
//! Entries are written atomically before the pending runtime entry is retired
//! and pruned once their durable `UpdatePending` commits or the session is
//! deleted.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct JournalRecoveryItem {
    pub text: String,
    pub content: Vec<serde_json::Value>,
    pub cmid: Option<String>,
    pub reason: String,
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub(super) fn load_recovery_journal(path: &Path) -> HashMap<String, Vec<JournalRecoveryItem>> {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<HashMap<String, Vec<JournalRecoveryItem>>>(&raw) {
            Ok(map) => map,
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    %error,
                    "failed to parse durable recovery park journal"
                );
                HashMap::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => {
            tracing::error!(
                path = %path.display(),
                %error,
                "failed to read durable recovery park journal"
            );
            HashMap::new()
        }
    }
}

pub(super) fn save_recovery_journal(
    path: &Path,
    entries: &HashMap<String, Vec<JournalRecoveryItem>>,
) -> std::io::Result<()> {
    let non_empty: HashMap<&String, &Vec<JournalRecoveryItem>> = entries
        .iter()
        .filter(|(_, items)| !items.is_empty())
        .collect();
    if non_empty.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_vec(&non_empty).map_err(std::io::Error::other)?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("recovery-parks.journal.json"),
        std::process::id(),
        seq
    );
    let tmp_path = path.with_file_name(tmp_name);
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(&payload)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_roundtrips_atomically_and_prunes_file_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery-parks.journal.json");
        assert!(load_recovery_journal(&path).is_empty());

        let mut map = HashMap::new();
        map.insert(
            "sess-j1".to_owned(),
            vec![JournalRecoveryItem {
                text: "keep me".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "keep me"})],
                cmid: Some("cmid-j1".to_owned()),
                reason: "store unavailable".to_owned(),
            }],
        );
        save_recovery_journal(&path, &map).unwrap();
        assert!(path.exists());
        assert_eq!(load_recovery_journal(&path), map);

        map.clear();
        save_recovery_journal(&path, &map).unwrap();
        assert!(!path.exists());
        assert!(load_recovery_journal(&path).is_empty());
    }
}

//! The per-round marker a managed child worker reads beside its workspace.
//! It is written only by the Machine's snapshot producer.

use std::ffi::OsStr;
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MAX_ROUND_BYTES: u64 = 256 * 1024;

/// Per-round marker the child worker reads to apply its native turn
/// constraint. It is Machine-written and never supplied by the child.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundMarker {
    pub schema: u16,
    pub call_id: String,
    pub input_revision: String,
    pub head: String,
    pub index_tree: String,
    pub worktree_tree: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

pub(crate) fn read_private(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
        || metadata.len() > limit
    {
        return None;
    }
    std::fs::read(path).ok()
}

/// Read the current round for a managed child workspace. The marker sits
/// beside the workspace in the Machine-owned child directory.
pub fn read_round(workspace: &Path) -> Option<RoundMarker> {
    let directory = workspace.parent()?;
    if workspace.file_name() != Some(OsStr::new("workspace")) {
        return None;
    }
    let bytes = read_private(&directory.join("round.json"), MAX_ROUND_BYTES)?;
    let marker: RoundMarker = serde_json::from_slice(&bytes).ok()?;
    (marker.schema == 1).then_some(marker)
}


/// ACP prompt metadata for a managed child turn. Adapters that declare the
/// managed profile apply `outputSchema` as a native turn constraint.
pub const PROMPT_META_KEY: &str = "cowboy.dev/managedCall";

impl RoundMarker {
    #[must_use]
    pub fn prompt_meta(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut meta = serde_json::Map::new();
        meta.insert(
            PROMPT_META_KEY.to_owned(),
            serde_json::json!({
                "callId": self.call_id,
                "inputRevision": self.input_revision,
                "outputSchema": self.output_schema,
            }),
        );
        meta
    }
}

//! One side of a diff by blob id (docs/desktop-code-review.md "Server and
//! API"): the exact text a patch was computed from, for whole-file context.

use serde::{Deserialize, Serialize};

use super::git_output;
use crate::code_review::{LocalCodeProvider, ensure_git_worktree, safe_oid};

/// The bytes one side of a diff may carry to expand its context.
const MAX_BLOB_BYTES: usize = 4 * 1024 * 1024;

/// One side of a diff, read by the object id its `index` line names, so the
/// text is the exact content the patch was computed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDocument {
    pub oid: String,
    pub text: String,
}

impl LocalCodeProvider {
    /// A text blob from the repository's object store by its (possibly
    /// abbreviated) id. Git refuses an ambiguous prefix and a non-blob
    /// object, so the answer is the blob the diff named or nothing.
    pub fn blob(&self, oid: &str) -> Result<BlobDocument, String> {
        let oid = safe_oid(oid).map_err(|_| "invalid blob oid".to_owned())?;
        ensure_git_worktree(&self.root)?;
        let bytes = git_output(&self.root, &["cat-file", "blob", oid], MAX_BLOB_BYTES + 1)
            .map_err(|_| "blob not found".to_owned())?;
        if bytes.len() > MAX_BLOB_BYTES {
            return Err("blob too large".to_owned());
        }
        if bytes.contains(&0) {
            return Err("binary file".to_owned());
        }
        let text = String::from_utf8(bytes).map_err(|_| "file is not UTF-8".to_owned())?;
        Ok(BlobDocument {
            oid: oid.to_owned(),
            text,
        })
    }
}

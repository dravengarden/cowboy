//! One side of a diff by blob id (docs/desktop-code-review.md "Server and
//! API"): the exact text a patch was computed from, for whole-file context.

use serde::{Deserialize, Serialize};

use super::git_output;
use crate::code_review::{LocalCodeProvider, ensure_git_worktree, safe_oid, safe_relative};

/// The bytes one side of a diff may carry to expand its context.
const MAX_BLOB_BYTES: usize = 4 * 1024 * 1024;
/// Objects sharing a diff's abbreviation that are inspected for their type.
const MAX_CANDIDATES: usize = 16;

/// One side of a diff, read by the object id its `index` line names, so the
/// text is the exact content the patch was computed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDocument {
    pub oid: String,
    pub text: String,
}

/// A worktree file's Git blob id as `git add` would store it now, so a
/// reader can compare it with the blob a review captured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileBlob {
    pub path: String,
    pub oid: String,
}

impl LocalCodeProvider {
    pub fn file_blob(&self, relative: &str) -> Result<FileBlob, String> {
        let relative = safe_relative(relative)?;
        ensure_git_worktree(&self.root)?;
        let file = self.resolved_file(&relative)?;
        if !file.is_file() {
            return Err("file not found".to_owned());
        }
        let path = relative.to_string_lossy().replace('\\', "/");
        let output = git_output(&self.root, &["hash-object", "--", &path], 128)
            .map_err(|_| "file not found".to_owned())?;
        let oid = String::from_utf8_lossy(&output).trim().to_owned();
        safe_oid(&oid).map_err(|_| "file not found".to_owned())?;
        Ok(FileBlob { path, oid })
    }

    /// A text blob from the repository's object store by its (possibly
    /// abbreviated) id. The prefix is resolved among objects only (never a
    /// ref of the same name), must name exactly one blob, and replacement
    /// refs are ignored, so the answer is the blob the diff named or nothing.
    pub fn blob(&self, oid: &str) -> Result<BlobDocument, String> {
        let oid = safe_oid(oid).map_err(|_| "invalid blob oid".to_owned())?;
        ensure_git_worktree(&self.root)?;
        let full = self.resolve_blob(oid)?;
        let bytes = git_output(
            &self.root,
            &["--no-replace-objects", "cat-file", "blob", &full],
            MAX_BLOB_BYTES + 1,
        )
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

    /// The single blob whose full id starts with `prefix`.
    fn resolve_blob(&self, prefix: &str) -> Result<String, String> {
        let prefix = prefix.to_ascii_lowercase();
        let listed = git_output(
            &self.root,
            &[
                "--no-replace-objects",
                "rev-parse",
                &format!("--disambiguate={prefix}"),
            ],
            64 * 1024,
        )
        .map_err(|_| "blob not found".to_owned())?;
        let listed = String::from_utf8_lossy(&listed);
        let blobs = listed
            .lines()
            .map(str::trim)
            .filter(|candidate| candidate.starts_with(&prefix))
            .take(MAX_CANDIDATES)
            .filter(|candidate| {
                git_output(
                    &self.root,
                    &["--no-replace-objects", "cat-file", "-t", candidate],
                    16,
                )
                .is_ok_and(|kind| kind.trim_ascii() == b"blob")
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        match blobs.as_slice() {
            [blob] => Ok(blob.clone()),
            _ => Err("blob not found".to_owned()),
        }
    }
}

//! Canonical completion-notice formatting shared across the Call runner and
//! runtime non-delivery recovery so requeued notices always rebuild their
//! payload from the authoritative Call ledger record rather than trusting
//! worker-echoed content.

use crate::managed_calls::{lifecycle::Record, review::NOTIFY};

/// Whether a Call opted into parent completion notification (`labels.notify == "parent"`).
#[must_use]
pub fn wants_notice(record: &Record) -> bool {
    record
        .labels
        .get(NOTIFY)
        .is_some_and(|value| value == "parent")
}

/// How to correct a call the target refused before any agent ran. Fixed
/// text keyed by refusal detail: it never echoes the caller's paths, but an
/// agent can fix its input in one retry instead of guessing.
#[must_use]
pub fn input_hint(detail: &str) -> Option<&'static str> {
    Some(match detail {
        "context_unsupported" => {
            "every files entry must be a regular file inside root, not a directory or a symlink; list a directory's files one by one"
        }
        "context_unreadable" => {
            "a files entry does not exist under root on the target machine; pass existing paths relative to root"
        }
        "source_unavailable" => {
            "root must be an existing Git worktree inside a workspace this session may read"
        }
        "subdirectory_unsupported" => {
            "root must be the top of a Git worktree (git rev-parse --show-toplevel), not a subdirectory"
        }
        "not_a_repository" => "root must be a Git worktree",
        "unborn_head" => "root needs at least one commit",
        "submodule_unsupported" => {
            "a submodule has local changes or an unsupported path; clean submodules are retained only as commit pointers, so review submodule contents from that submodule's own Git root"
        }
        "symlink_unsupported" => {
            "the worktree contains a symlink pointing outside root, which a snapshot cannot carry"
        }
        _ => return None,
    })
}

/// Format the canonical completion notice prompt text for a terminal Call.
#[must_use]
pub fn call_notice(record: &Record) -> String {
    let title = record
        .labels
        .get("title")
        .map(|title| format!(" ({title})"))
        .unwrap_or_default();
    let error = record
        .error
        .as_ref()
        .map(|error| format!(", error {}", error.code))
        .unwrap_or_default();
    let hint = record
        .error
        .as_ref()
        .and_then(|error| error.detail.as_deref())
        .and_then(input_hint)
        .map(|hint| format!(" No agent ran: {hint}, then start a new call."))
        .unwrap_or_default();
    format!(
        "[Cowboy] Managed call {}{title} finished: {}{error}. Read it with cowboy_read: \
         cowboy.calls.result(\"{}\",{{offset:0,chars:12000}}). A running, failed or partial \
         review is never evidence of a clean review.{hint}",
        record.call_id,
        record.state.code(),
        record.call_id
    )
}

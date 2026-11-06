//! Call input fault hints and the request envelope shared by admission.

use super::*;

pub(super) fn error_view(error: Option<&crate::managed_calls::lifecycle::CallError>) -> Value {
    let Some(error) = error else {
        return Value::Null;
    };
    let mut value = json!(error);
    if let Some(hint) = error.detail.as_deref().and_then(input_hint) {
        value["hint"] = json!(hint);
    }
    if error.code == "provider_startup_failed" {
        value["hint"] = json!(
            "the child runtime crashed or was interrupted before configuration was confirmed; inspect its lifecycle events for memory admission or authentication failures before retrying; do not change the requested preset to work around a startup failure"
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_failure_explains_recovery_without_changing_the_preset() {
        let error =
            crate::managed_calls::lifecycle::CallError::new("provider_startup_failed", None);
        let view = error_view(Some(&error));
        assert_eq!(view["code"], "provider_startup_failed");
        assert!(view["hint"].as_str().unwrap().contains("lifecycle events"));
        assert!(error_view(None).is_null());
    }
}

pub(crate) fn envelope(record: &Record) -> Value {
    let next = if record.state.terminal() {
        json!({"argv":["cowboy","call","result",record.call_id]})
    } else {
        json!({"argv":["cowboy","call","wait",record.call_id]})
    };
    json!({
        "schema": 1,
        "state": record.state.code(),
        "call_id": record.call_id,
        "request_id": record.request_id,
        "resolved": {
            "provider": record.provider,
            "provider_version": record.provider_version,
            "runtime_machine_id": record.runtime_machine_id,
            "machine_id": record.placement.machine_id,
            "workspace_id": record.placement.workspace_id,
            "child_session_id": record.summary().reused_child_session_id.unwrap_or(&record.child_session_id),
            "input_revision": record.input_revision,
            "preset": record.preset,
            "configuration": record.configuration,
            "provider_account_id": record.provider_account_id,
            "selection": record.selection,
        },
        "created_at_ms": record.created_at_ms,
        "started_at_ms": record.started_at_ms,
        "finished_at_ms": record.finished_at_ms,
        "updated_at_ms": record.updated_at_ms,
        "labels": record.labels,
        "usage": record.summary().usage,
        "reused_from": record.summary().reused_from,
        "cancel_requested": record.cancel_requested_at_ms.is_some(),
        "error": error_view(record.error.as_ref()),
        "next": next,
    })
}

/// How to correct a call the target refused before any agent ran. Fixed
/// text keyed by refusal detail: it never echoes the caller's paths, but an
/// agent can fix its input in one retry instead of guessing.
pub(in crate::server) fn input_hint(detail: &str) -> Option<&'static str> {
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
            "the worktree contains a submodule, which a snapshot cannot carry"
        }
        "symlink_unsupported" => {
            "the worktree contains a symlink pointing outside root, which a snapshot cannot carry"
        }
        _ => return None,
    })
}

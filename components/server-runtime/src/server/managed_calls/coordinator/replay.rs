//! A repeated request id observes the original call.

use std::sync::Arc;

use serde_json::Value;

use super::{envelope, error, wait_started};
use crate::managed_calls::lifecycle::Record;
use crate::managed_calls::service::Ledger;
use crate::managed_calls::{Conversation, Request};

use super::super::super::AppState;
use super::super::super::agent_tools;

/// A repeated request id names the same call: an explicit agent must match,
/// and Auto matches a call Auto chose. A continuation keeps its child's own
/// agent and the selection the child was opened with, so Auto names it too.
fn same_request(
    original: &Record,
    requested: &str,
    preset: Option<&str>,
    continuation: bool,
) -> bool {
    if preset.is_some_and(|preset| original.preset.as_deref() != Some(preset)) {
        return false;
    }
    if requested == agent_tools::AUTO {
        continuation || original.selection.as_deref() == Some(agent_tools::AUTO)
    } else {
        original.provider == requested
    }
}

/// Whether `request` matches `original.request_digest`. For an unbound Auto
/// review, toggling Antigravity in Tools flips only Cowboy's internal output
/// format between `Text` and the fixed review schema; accept that exact flip
/// while still requiring every caller-supplied field to match.
fn digest_matches(original: &Record, requested: &str, request: &Request) -> bool {
    if original.request_digest == request.digest() {
        return true;
    }
    let unbound_auto_review = requested == agent_tools::AUTO
        && original.selection.as_deref() == Some(agent_tools::AUTO)
        && request.purpose.is_review()
        && !request
            .labels
            .contains_key(crate::managed_calls::review::TASK)
        && !request
            .labels
            .contains_key(crate::managed_calls::review::BASE);
    if !unbound_auto_review {
        return false;
    }
    let mut alternate = request.clone();
    alternate.output = match &request.output {
        crate::managed_calls::OutputFormat::Text {} => {
            crate::managed_calls::OutputFormat::JsonSchema {
                schema: crate::managed_calls::review::output_schema(),
            }
        }
        crate::managed_calls::OutputFormat::JsonSchema { schema }
            if *schema == crate::managed_calls::review::output_schema() =>
        {
            crate::managed_calls::OutputFormat::Text {}
        }
        _ => return false,
    };
    original.request_digest == alternate.digest()
}

/// Map a ledger admission refusal, delegating concurrent-duplicate conflicts
/// to [`replay`] so a racing Tools toggle resolves cleanly to the winning call.
pub async fn admit_error(
    state: &Arc<AppState>,
    ledger: &Ledger,
    text: &str,
    submitted: (&str, Option<&str>),
    request: &Request,
    deadline: tokio::time::Instant,
) -> Value {
    if text.contains("conflict")
        && let Ok(Some(original)) = ledger.observe(&request.request_id).await
    {
        return replay(state, ledger, &original, submitted, request, deadline).await;
    }
    let code = if text.contains("conflict") {
        "request_conflict"
    } else if text.contains("unavailable or incompatible") || text.contains("not owned") {
        "conversation_unavailable"
    } else {
        "conversation_busy"
    };
    error(code, "not_submitted")
}

/// The original call of a repeated request id, even if the Provider has since
/// become unavailable or the policy changed; conflicting inputs are refused.
pub async fn replay(
    state: &Arc<AppState>,
    ledger: &Ledger,
    original: &Record,
    (requested, preset): (&str, Option<&str>),
    request: &Request,
    deadline: tokio::time::Instant,
) -> Value {
    let continuation = matches!(request.conversation, Conversation::Continue { .. });
    if !digest_matches(original, requested, request)
        || !same_request(original, requested, preset, continuation)
    {
        return error("request_conflict", "not_submitted");
    }
    super::super::runner::ensure_runner(state, original);
    envelope(&wait_started(state, ledger, original, deadline).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(provider: &str, selection: Option<&str>, preset: Option<&str>) -> Record {
        serde_json::from_value(json!({
            "schema":1,"call_id":"call-1","request_id":"req-1","request_digest":"d",
            "provider":provider,"purpose":"review","access":"read-only","labels":{},
            "placement":{"service_id":"svc","parent_session_id":"p","parent_revision":"r1",
                "machine_id":"hawk","workspace_id":"w","cwd":"/workspace"},
            "child_session_id":"child-1","state":"queued","revision":1,
            "created_at_ms":1,"updated_at_ms":1,"input_revision":null,"result":null,
            "preset":preset,"selection":selection
        }))
        .unwrap()
    }

    #[test]
    fn a_repeated_request_matches_what_it_asked_for() {
        let auto = record("claude-code", Some("auto"), None);
        let explicit = record("claude-code", Some("explicit"), None);
        assert!(same_request(&auto, "auto", None, false));
        assert!(!same_request(&explicit, "auto", None, false));
        assert!(same_request(&explicit, "claude-code", None, false));
        assert!(!same_request(&explicit, "codex", None, false));
        assert!(!same_request(
            &explicit,
            "claude-code",
            Some("opus-high"),
            false
        ));
        // A continuation inherits its child's explicit selection: Auto, or the
        // child's own agent, replays; another agent never does.
        assert!(same_request(&explicit, "auto", None, true));
        assert!(same_request(&explicit, "claude-code", None, true));
        assert!(!same_request(&explicit, "codex", None, true));
    }

    #[test]
    fn unbound_auto_review_replay_tolerates_only_the_tools_derived_review_schema_flip() {
        let make_req = |instruction: &str, output: Value, labels: Value| {
            Request::parse(
                serde_json::to_string(&json!({
                    "schema": 1,
                    "request_id": "op-1",
                    "purpose": "review",
                    "instruction": instruction,
                    "context": {"scope": "current-worktree"},
                    "access": "read-only",
                    "conversation": {"mode": "fresh"},
                    "labels": labels,
                    "output": output
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap()
        };
        let text_req = make_req("check diff", json!({"format": "text"}), json!({}));
        let schema_req = make_req(
            "check diff",
            json!({"format": "json_schema", "schema": crate::managed_calls::review::output_schema()}),
            json!({}),
        );
        let mut original = record("codex", Some("auto"), None);
        original.request_digest = text_req.digest();
        assert!(digest_matches(&original, "auto", &text_req));
        assert!(digest_matches(&original, "auto", &schema_req));
        // Mutating any caller argument still fails conflict check.
        let mutated = make_req(
            "different diff",
            json!({"format": "json_schema", "schema": crate::managed_calls::review::output_schema()}),
            json!({}),
        );
        assert!(!digest_matches(&original, "auto", &mutated));
        // Explicit provider requests never cross output formats.
        assert!(!digest_matches(&original, "codex", &schema_req));
        // Bound reviews always carry the schema and never flip to text.
        let bound_schema = Request::parse(
            serde_json::to_string(&json!({
                "schema": 1,
                "request_id": "op-1",
                "purpose": "review",
                "instruction": "check diff",
                "context": {"scope": "current-worktree", "root": "/tasks/example"},
                "access": "read-only",
                "conversation": {"mode": "fresh"},
                "labels": {"review_task": "t1", "review_base": "a".repeat(40), "review_reuse": "true"},
                "output": {"format": "json_schema", "schema": crate::managed_calls::review::output_schema()}
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
        let mut bound_text = bound_schema.clone();
        bound_text.output = crate::managed_calls::OutputFormat::Text {};
        let mut bound_orig = record("codex", Some("auto"), None);
        bound_orig.request_digest = bound_schema.digest();
        assert!(!digest_matches(&bound_orig, "auto", &bound_text));
    }
}

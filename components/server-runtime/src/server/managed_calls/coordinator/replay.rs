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
    if original.request_digest != request.digest()
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
}

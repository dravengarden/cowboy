//! Context-only call targets (Antigravity): admission and discovery.
//!
//! A context-only agent runs only when a caller names it: one fresh read-only
//! round (a context ask, an analysis, a review or a design review). It runs on
//! a person's own named account, guarded by that account's own controls
//! (rate-limit back-off, suspension) and the parent's call budget, and it is
//! never an Auto choice, a Tools target or an approval.

use serde_json::{Value, json};

use super::super::super::AppState;
use super::super::super::agent_tools::{self, CallsPolicy, Choice};
use crate::managed_calls::service::Ledger;
use crate::managed_calls::{Conversation, Request};

/// Every agent discovery describes (each listed once).
pub fn discoverable() -> impl Iterator<Item = &'static str> {
    agent_tools::CALL_TARGETS.into_iter()
}

/// The context-ask decisions that precede ordinary agent choice: a history
/// ask to a connected Machine too old to receive it is refused before it
/// spends the session budget, and a context-only agent is chosen here.
/// `None` leaves the choice to `agent_tools::choose`.
pub fn resolve(
    state: &AppState,
    ledger: &Ledger,
    policy: &CallsPolicy,
    caller: &str,
    requested: &str,
    request: &Request,
) -> Option<Choice> {
    if history_refusal(state, ledger, request) {
        return Some(Choice::Closed("machine_upgrade_required"));
    }
    let continued = matches!(request.conversation, Conversation::Continue { .. });
    choose(policy, caller, requested, request, continued)
}

/// `requested`'s admission decision when it is a context-only agent.
pub fn choose(
    policy: &CallsPolicy,
    caller: &str,
    requested: &str,
    request: &Request,
    continued: bool,
) -> Option<Choice> {
    agent_tools::context_only(requested).then(|| {
        match agent_tools::choose_context_only(policy, caller, requested, request, continued) {
            Ok(target) => Choice::Candidates {
                targets: vec![target],
                selection: "explicit",
            },
            // Never a question: approval cannot grant a context-only agent.
            Err(code) => Choice::Closed(code),
        }
    })
}

/// Whether `machine` can receive the history documents of a context ask.
pub fn history_capable(state: &AppState, machine: &str) -> bool {
    state
        .machine_control
        .connection_protocol(machine)
        .is_some_and(|(_, protocol)| {
            protocol >= crate::machine_protocol::MANAGED_CONTEXT_PROTOCOL_VERSION
        })
}

/// A context ask refused before it consumes the session's call budget: its
/// execution Machine is connected but too old to receive history. A
/// disconnected Machine is left to the dispatch-time check, which retries.
fn history_refusal(state: &AppState, ledger: &Ledger, request: &Request) -> bool {
    let observed = state
        .machine_control
        .connection_protocol(&ledger.placement().machine_id);
    let too_old = observed.is_some_and(|(_, protocol)| {
        protocol < crate::machine_protocol::MANAGED_CONTEXT_PROTOCOL_VERSION
    });
    !request.context.sessions.is_empty() && too_old
}

/// Why discovery lists `provider` as unavailable before readiness, matching
/// what admission refuses: a context-only agent needs no Tools target and
/// may serve its own family, since a context ask reads history for the caller.
pub fn policy_reason(
    policy: &CallsPolicy,
    caller: &str,
    provider: &str,
    targeted: bool,
) -> Option<&'static str> {
    let context_only = agent_tools::context_only(provider);
    // The caller's own vendor is callable after the others while any target
    // is on (docs/review-routing.md); reviews exclude its own model.
    let own = agent_tools::same_family(provider, caller) && !policy.targets.is_empty();
    if !policy.enabled {
        Some("calls_disabled")
    } else if !targeted && !context_only && !own {
        Some("policy_denied")
    } else {
        None
    }
}

/// Whether a context ask may use `provider`: admission admits the caller's
/// own family and context-only agents without a Tools target.
pub fn serves_context_asks(
    policy: &CallsPolicy,
    caller: &str,
    provider: &str,
    targeted: bool,
) -> bool {
    policy.enabled
        && (targeted
            || agent_tools::context_only(provider)
            || agent_tools::same_family(provider, caller))
}

/// Narrow a context-only provider's discovery entry to what admission
/// accepts: fresh rounds only, named explicitly (a context ask answers in
/// text). It is listed among the read-only providers too; `selection:
/// explicit` says it runs only when named. It never reviews for its own
/// family, and only its context asks need a Machine that receives history.
pub fn restrict(
    entry: &mut Value,
    targeted: bool,
    structured_output: bool,
    managed_continue: bool,
    history_capable: bool,
) {
    if !managed_continue {
        entry["conversation"] = json!(["fresh"]);
    }
    if !targeted {
        entry["selection"] = json!("explicit");
    }
    let reviews = structured_output
        && (entry["review_candidates"]
            .as_array()
            .is_some_and(|candidates| !candidates.is_empty())
            || entry["review_available"].as_bool().unwrap_or(false));
    if !reviews {
        entry["purposes"] = json!(["analysis"]);
        entry["review_candidates"] = json!([]);
        entry["review_routing"] = Value::Null;
        entry["review_available"] = json!(false);
        entry["output"] = json!(["text"]);
    } else {
        entry["purposes"] = json!(["analysis", "review", "design_review"]);
        // Structured output is a review's own schema; analyses answer in text.
        if entry["output"] == json!(["text", "json_schema"]) {
            entry["json_schema"] = json!("review_schema_only");
        }
    }
    if !history_capable {
        entry["context_ask_reason"] = json!("machine_upgrade_required");
    }
}

/// The discovery entry of the context-ask capability.
pub fn capability(policy: &CallsPolicy, providers: &[&str]) -> Value {
    json!({"id": "context_ask", "tasks": ["analysis"], "requires": "sessions",
        "enabled": policy.enabled, "providers": providers})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_reflects_tools_opt_in_continuation_output_contract_and_eligible_reviews() {
        let mut entry = json!({
            "id": "antigravity",
            "conversation": ["fresh", "continue"],
            "review_candidates": ["gemini-neon-high", "claude-opus-high"]
        });
        restrict(&mut entry, false, true, true, false);
        assert_eq!(entry["conversation"], json!(["fresh", "continue"]));
        assert_eq!(entry["selection"], json!("explicit"));
        assert_eq!(
            entry["purposes"],
            json!(["analysis", "review", "design_review"])
        );
        assert_eq!(
            entry["review_candidates"],
            json!(["gemini-neon-high", "claude-opus-high"])
        );
        assert_eq!(entry["context_ask_reason"], "machine_upgrade_required");

        let mut legacy = json!({
            "id": "antigravity",
            "conversation": ["fresh", "continue"],
            "review_candidates": ["gemini-neon-high"],
            "review_available": true,
            "output": ["text"]
        });
        restrict(&mut legacy, false, false, false, true);
        assert_eq!(legacy["conversation"], json!(["fresh"]));
        assert_eq!(legacy["purposes"], json!(["analysis"]));
        assert_eq!(legacy["review_candidates"], json!([]));
        assert_eq!(legacy["review_routing"], Value::Null);
        assert_eq!(legacy["review_available"], json!(false));

        let mut opted_in = json!({
            "id": "antigravity",
            "conversation": ["fresh", "continue"],
            "review_candidates": ["claude-opus-high"]
        });
        restrict(&mut opted_in, true, true, true, true);
        assert!(opted_in.get("selection").is_none());
        assert_eq!(
            opted_in["purposes"],
            json!(["analysis", "review", "design_review"])
        );
        assert!(opted_in.get("context_ask_reason").is_none());

        let mut none = json!({"id": "antigravity", "review_candidates": []});
        restrict(&mut none, false, true, true, true);
        assert_eq!(none["purposes"], json!(["analysis"]));
        assert_eq!(none["review_candidates"], json!([]));
        assert_eq!(none["output"], json!(["text"]));
    }

    fn policy(enabled: bool) -> CallsPolicy {
        let mut calls = agent_tools::AgentTools::builtin("codex").calls;
        calls.enabled = enabled;
        calls
    }

    #[test]
    fn discovery_matches_context_admission() {
        let on = policy(true);
        // Antigravity needs no Tools target and serves its own family.
        assert_eq!(
            policy_reason(&on, "antigravity", "antigravity", false),
            None
        );
        assert!(serves_context_asks(
            &on,
            "antigravity",
            "antigravity",
            false
        ));
        // A Codex parent may call Codex after the other vendors (reviews then
        // exclude its own model), while any target is on.
        assert_eq!(policy_reason(&on, "codex", "codex", true), None);
        assert!(serves_context_asks(&on, "codex", "codex", false));
        assert_eq!(
            policy_reason(&on, "codex", "claude-code", false),
            Some("policy_denied")
        );
        assert!(!serves_context_asks(&on, "codex", "claude-code", false));
        let off = policy(false);
        assert_eq!(
            policy_reason(&off, "codex", "antigravity", false),
            Some("calls_disabled")
        );
        assert!(!serves_context_asks(&off, "codex", "antigravity", false));
    }

    #[test]
    fn a_context_only_refusal_is_never_an_approval_question() {
        let document = format!(
            r#"{{"schema":1,"request_id":"r","purpose":"review","instruction":"Review","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}},"output":{{"format":"json_schema","schema":{}}}}}"#,
            crate::managed_calls::review::output_schema()
        );
        let request = Request::parse(document.as_bytes()).unwrap();
        assert!(matches!(
            choose(&policy(true), "codex", "antigravity", &request, false),
            Some(Choice::Candidates {
                selection: "explicit",
                ..
            })
        ));
        assert!(matches!(
            choose(&policy(true), "codex", "antigravity", &request, true),
            Some(Choice::Candidates {
                selection: "explicit",
                ..
            })
        ));
        assert!(matches!(
            choose(&policy(true), "antigravity", "antigravity", &request, false),
            Some(Choice::Candidates {
                selection: "explicit",
                ..
            })
        ));
        assert!(matches!(
            choose(&policy(false), "codex", "antigravity", &request, false),
            Some(Choice::Closed(_))
        ));
        assert!(choose(&policy(true), "codex", "codex", &request, false).is_none());
    }
}

//! Context-only call targets (Antigravity): admission and discovery.
//!
//! A context-only agent answers only an explicit context ask: a fresh,
//! text-output `analysis` over recorded history. It runs on a person's own
//! named account, guarded by that account's own controls (rate-limit
//! back-off, suspension) and the parent's call budget.

use serde_json::{Value, json};

use super::super::super::AppState;
use super::super::super::agent_tools::{self, CallsPolicy, Choice};
use crate::managed_calls::Request;
use crate::managed_calls::service::Ledger;

/// Every agent discovery describes: call targets, then context-only agents.
pub(super) fn discoverable() -> impl Iterator<Item = &'static str> {
    agent_tools::CALL_TARGETS
        .into_iter()
        .chain(agent_tools::CONTEXT_ONLY_TARGETS)
}

/// The context-ask decisions that precede ordinary agent choice: a history
/// ask to a connected Machine too old to receive it is refused before it
/// spends the session budget, and a context-only agent is chosen here.
/// `None` leaves the choice to `agent_tools::choose`.
pub(super) fn resolve(
    state: &AppState,
    ledger: &Ledger,
    policy: &CallsPolicy,
    requested: &str,
    request: &Request,
    continued: &Option<String>,
) -> Option<Choice> {
    if history_refusal(state, ledger, request) {
        return Some(Choice::Closed("machine_upgrade_required"));
    }
    choose(policy, requested, request, continued.is_some())
}

/// `requested`'s admission decision when it is a context-only agent.
pub(super) fn choose(
    policy: &CallsPolicy,
    requested: &str,
    request: &Request,
    continued: bool,
) -> Option<Choice> {
    agent_tools::context_only(requested).then(|| {
        match agent_tools::choose_context_only(policy, requested, request, continued) {
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
pub(super) fn history_capable(state: &AppState, machine: &str) -> bool {
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
pub(super) fn policy_reason(
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

/// A context-only agent serves nothing when the execution Machine cannot
/// receive history, so discovery marks it unavailable.
pub(super) fn history_reason(provider: &str, history_capable: bool) -> Option<&'static str> {
    (agent_tools::context_only(provider) && !history_capable).then_some("machine_upgrade_required")
}

/// Whether a context ask may use `provider`: admission admits the caller's
/// own family and context-only agents without a Tools target.
pub(super) fn serves_context_asks(
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

/// Narrow a context-only provider's discovery entry to what it accepts.
pub(super) fn restrict(entry: &mut Value) {
    entry["review_candidates"] = json!([]);
    entry["output"] = json!(["text"]);
    entry["conversation"] = json!(["fresh"]);
    entry["purposes"] = json!(["analysis"]);
    entry["requires"] = json!("sessions");
}

/// The discovery entry of the context-ask capability.
pub(super) fn capability(policy: &CallsPolicy, providers: &[&str]) -> Value {
    json!({"id": "context_ask", "tasks": ["analysis"], "requires": "sessions",
        "enabled": policy.enabled, "providers": providers})
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let request = Request::parse(br#"{"schema":1,"request_id":"r","purpose":"review","instruction":"Review","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#).unwrap();
        assert!(matches!(
            choose(&policy(true), "antigravity", &request, false),
            Some(Choice::Closed("unsupported_capability"))
        ));
        assert!(matches!(
            choose(&policy(false), "antigravity", &request, false),
            Some(Choice::Closed(_))
        ));
        assert!(choose(&policy(true), "codex", &request, false).is_none());
    }
}

//! Agent calls a session's policy refused, waiting for a person.
//!
//! An agent whose call is refused because calls are off (or its target is not
//! allowed) keeps resubmitting the same request while it waits. Each refusal
//! refreshes one prompt per parent session that aggregates every waiting
//! request. Every waiting request has a stable `approval_request_id`, and the
//! prompt carries the `request_ids` it presents and a `prompt_revision` that
//! advances whenever that set changes. A decision applies only to the
//! presented requests still waiting: one that arrived after the person saw
//! the prompt never inherits an earlier "once" and gets a refreshed prompt
//! instead (docs/background-tasks.md §5). A request nobody resubmits drops out
//! after `WAITING_TTL`, so the prompt disappears when the agent stops waiting,
//! and a recorded decision older than `DECIDED_TTL` is not applied. Nothing
//! here is durable: a Controller restart forgets pending prompts and the
//! agents ask again.

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::managed_calls::Request;

const WAITING_TTL: Duration = Duration::from_secs(15);
const DECIDED_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_WAITING: usize = 32;
const MAX_SHOWN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::server) enum Decision {
    /// Admit the waiting requests without changing the session's policy.
    Once,
    /// Turn calls on for this session and allow the requested agents.
    Session,
    Decline,
}

/// What a refused request may do now.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Gate {
    Approved,
    Declined,
    Pending,
}

struct Waiting {
    approval_id: String,
    agent: String,
    purpose: Value,
    summary: String,
    reason: &'static str,
    seen: Instant,
}

#[derive(Default)]
struct Parent {
    waiting: HashMap<String, Waiting>,
    order: Vec<String>,
    /// Decided request id -> (decision, when, its approval request id).
    decided: HashMap<String, (Decision, Instant, String)>,
    /// Advances whenever the waiting set changes.
    revision: u64,
}

impl Parent {
    fn prune(&mut self, now: Instant) {
        let before = self.waiting.len();
        self.waiting
            .retain(|_, waiting| now.duration_since(waiting.seen) < WAITING_TTL);
        if self.waiting.len() != before {
            self.revision += 1;
        }
        self.order.retain(|id| self.waiting.contains_key(id));
        self.decided
            .retain(|_, (_, at, _)| now.duration_since(*at) < DECIDED_TTL);
    }

    fn is_empty(&self) -> bool {
        self.waiting.is_empty() && self.decided.is_empty()
    }
}

#[derive(Default)]
pub(in crate::server) struct Approvals {
    parents: Mutex<HashMap<String, Parent>>,
}

fn approval_id() -> String {
    let mut bytes = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    format!(
        "apr-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}

/// A short, human label: the request's labels when it has any, else the
/// first line of its instruction.
fn summary(request: &Request) -> String {
    let text = if request.labels.is_empty() {
        request
            .instruction
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_owned()
    } else {
        request
            .labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut short: String = text.chars().take(120).collect();
    if short.len() < text.len() {
        short.push('…');
    }
    short
}

impl Approvals {
    /// Record a refused request and report what a person decided for it.
    pub(super) fn refused(
        &self,
        parent: &str,
        request: &Request,
        agent: &str,
        reason: &'static str,
    ) -> Gate {
        let now = Instant::now();
        let mut parents = self.parents.lock();
        let entry = parents.entry(parent.to_owned()).or_default();
        entry.prune(now);
        match entry.decided.get(&request.request_id) {
            Some((Decision::Decline, _, _)) => return Gate::Declined,
            Some(_) => return Gate::Approved,
            None => {}
        }
        if let Some(waiting) = entry.waiting.get_mut(&request.request_id) {
            waiting.seen = now;
            waiting.reason = reason;
            return Gate::Pending;
        }
        if entry.waiting.len() >= MAX_WAITING {
            // Still pending for this agent, just not listed.
            return Gate::Pending;
        }
        entry.order.push(request.request_id.clone());
        entry.revision += 1;
        entry.waiting.insert(
            request.request_id.clone(),
            Waiting {
                approval_id: approval_id(),
                agent: agent.to_owned(),
                purpose: serde_json::to_value(request.purpose).unwrap_or(Value::Null),
                summary: summary(request),
                reason,
                seen: now,
            },
        );
        Gate::Pending
    }

    /// An admitted request no longer needs its decision. Returns the approval
    /// request id a person decided for it, if one did.
    pub(super) fn admitted(&self, parent: &str, request_id: &str) -> Option<String> {
        let mut parents = self.parents.lock();
        let entry = parents.get_mut(parent)?;
        let approval = entry.decided.remove(request_id).map(|(_, _, id)| id);
        if entry.waiting.remove(request_id).is_some() {
            entry.revision += 1;
        }
        entry.order.retain(|id| id != request_id);
        if entry.is_empty() {
            parents.remove(parent);
        }
        approval
    }

    /// Apply a decision. With `presented`, only the presented requests that
    /// are still waiting are decided; later arrivals stay pending. Without it
    /// (a Web client older than the binding) every waiting request is decided,
    /// as before. Returns the decided requests' agents, or `None` when none of
    /// the targeted requests is waiting any more.
    pub(in crate::server) fn decide(
        &self,
        parent: &str,
        decision: Decision,
        presented: Option<&[String]>,
    ) -> Option<Vec<String>> {
        let now = Instant::now();
        let mut parents = self.parents.lock();
        let entry = parents.get_mut(parent)?;
        entry.prune(now);
        let targeted: Vec<String> = entry
            .waiting
            .iter()
            .filter(|(_, waiting)| {
                presented.is_none_or(|presented| presented.contains(&waiting.approval_id))
            })
            .map(|(request_id, _)| request_id.clone())
            .collect();
        if targeted.is_empty() {
            return None;
        }
        let mut agents = BTreeSet::new();
        for request_id in targeted {
            if let Some(waiting) = entry.waiting.remove(&request_id) {
                agents.insert(waiting.agent);
                entry
                    .decided
                    .insert(request_id, (decision, now, waiting.approval_id));
            }
        }
        entry.order.retain(|id| entry.waiting.contains_key(id));
        entry.revision += 1;
        Some(agents.into_iter().collect())
    }

    /// The parent's prompt, or `None` when nothing waits.
    pub(in crate::server) fn view(&self, parent: &str, caller: &str) -> Option<Value> {
        let now = Instant::now();
        let mut parents = self.parents.lock();
        let entry = parents.get_mut(parent)?;
        entry.prune(now);
        if entry.waiting.is_empty() {
            if entry.is_empty() {
                parents.remove(parent);
            }
            return None;
        }
        let waiting: Vec<&Waiting> = entry
            .order
            .iter()
            .filter_map(|id| entry.waiting.get(id))
            .collect();
        let agents: BTreeSet<&str> = waiting.iter().map(|item| item.agent.as_str()).collect();
        // Off for the session outranks a target that is merely not allowed.
        let reason = if waiting.iter().any(|item| item.reason == "calls_disabled") {
            "calls_disabled"
        } else {
            "policy_denied"
        };
        Some(json!({
            "schema": 1,
            "caller": caller,
            "reason": reason,
            "requests": waiting.len(),
            "agents": agents,
            "items": waiting.iter().take(MAX_SHOWN).map(|item| json!({
                "approval_request_id": item.approval_id,
                "agent": item.agent,
                "purpose": item.purpose,
                "summary": item.summary,
            })).collect::<Vec<_>>(),
            // Every request this prompt presents, including those beyond the
            // shown items: a decision binds to exactly this set.
            "request_ids": waiting.iter().map(|item| item.approval_id.as_str()).collect::<Vec<_>>(),
            "prompt_revision": entry.revision,
            "ttl_ms": WAITING_TTL.as_millis() as u64,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> Request {
        serde_json::from_value(json!({
            "schema": 1,
            "request_id": id,
            "purpose": "review",
            "instruction": "\n  Review the change\nmore",
            "context": {"scope": "current-worktree"},
            "access": "read-only",
            "conversation": {"mode": "fresh"},
        }))
        .unwrap()
    }

    #[test]
    fn waiting_requests_share_one_prompt_and_one_decision() {
        let approvals = Approvals::default();
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Pending
        );
        assert_eq!(
            approvals.refused("p", &request("r2"), "claude-code", "policy_denied"),
            Gate::Pending
        );
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Pending
        );
        let view = approvals.view("p", "claude-code").unwrap();
        assert_eq!(view["requests"], 2);
        assert_eq!(view["reason"], "calls_disabled");
        assert_eq!(view["items"][0]["summary"], "Review the change");
        assert_eq!(
            approvals.decide("p", Decision::Once, None),
            Some(vec!["claude-code".to_owned(), "codex".to_owned()])
        );
        assert!(approvals.view("p", "claude-code").is_none());
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Approved
        );
        assert!(
            approvals
                .admitted("p", "r1")
                .is_some_and(|id| id.starts_with("apr-"))
        );
        // Admission consumes a once-approval.
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Pending
        );
        assert_eq!(
            approvals
                .decide("p", Decision::Decline, None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Declined
        );
        assert!(approvals.decide("other", Decision::Once, None).is_none());
    }

    // T10: a request that arrives after the prompt was presented is not
    // admitted by the earlier "once"; it gets a refreshed prompt.
    #[test]
    fn t10_a_decision_binds_to_the_presented_requests_only() {
        let approvals = Approvals::default();
        approvals.refused("p", &request("r1"), "codex", "calls_disabled");
        let shown = approvals.view("p", "claude-code").unwrap();
        let presented: Vec<String> = shown["request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(presented.len(), 1);
        assert_eq!(
            shown["items"][0]["approval_request_id"],
            presented[0].as_str()
        );
        let revision = shown["prompt_revision"].as_u64().unwrap();
        // r2 arrives after the person saw the prompt.
        approvals.refused("p", &request("r2"), "claude-code", "calls_disabled");
        let refreshed = approvals.view("p", "claude-code").unwrap();
        assert!(refreshed["prompt_revision"].as_u64().unwrap() > revision);
        assert_eq!(
            approvals.decide("p", Decision::Once, Some(&presented)),
            Some(vec!["codex".to_owned()])
        );
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Approved
        );
        assert_eq!(
            approvals.refused("p", &request("r2"), "claude-code", "calls_disabled"),
            Gate::Pending,
            "the late request did not inherit the once"
        );
        let after = approvals.view("p", "claude-code").unwrap();
        assert_eq!(after["requests"], 1);
        assert_eq!(after["items"][0]["agent"], "claude-code");
        // A stale presented set (nothing it names still waits) decides nothing.
        assert_eq!(
            approvals.decide("p", Decision::Once, Some(&presented)),
            None
        );
        assert_eq!(
            approvals.refused("p", &request("r2"), "claude-code", "calls_disabled"),
            Gate::Pending
        );
    }

    #[test]
    fn t10_expired_prompts_and_decisions_are_not_actionable() {
        let approvals = Approvals::default();
        approvals.refused("p", &request("r1"), "codex", "calls_disabled");
        let presented: Vec<String> = approvals.view("p", "codex").unwrap()["request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        {
            let mut parents = approvals.parents.lock();
            let entry = parents.get_mut("p").unwrap();
            for waiting in entry.waiting.values_mut() {
                waiting.seen = Instant::now().checked_sub(WAITING_TTL).unwrap();
            }
        }
        assert!(
            approvals.view("p", "codex").is_none(),
            "an expired prompt disappears"
        );
        assert_eq!(
            approvals.decide("p", Decision::Once, Some(&presented)),
            None
        );
        approvals.refused("p", &request("r3"), "codex", "calls_disabled");
        approvals.decide("p", Decision::Once, None).unwrap();
        {
            let mut parents = approvals.parents.lock();
            let entry = parents.get_mut("p").unwrap();
            for decided in entry.decided.values_mut() {
                decided.1 = Instant::now().checked_sub(DECIDED_TTL).unwrap();
            }
        }
        assert_eq!(
            approvals.refused("p", &request("r3"), "codex", "calls_disabled"),
            Gate::Pending,
            "a decision older than 30 minutes is not applied"
        );
    }
}

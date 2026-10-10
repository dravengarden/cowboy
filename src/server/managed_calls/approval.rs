//! Agent calls a session's policy refused, waiting for a person.
//!
//! An agent whose call is refused because calls are off (or its target is not
//! allowed) keeps resubmitting the same request while it waits. Each refusal
//! refreshes one prompt per parent session that aggregates every waiting
//! request; a decision applies to the requests waiting at that moment. A
//! request nobody resubmits drops out after `WAITING_TTL`, so the prompt
//! disappears when the agent stops waiting. Nothing here is durable: a
//! Controller restart forgets pending prompts and the agents ask again.

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
    decided: HashMap<String, (Decision, Instant)>,
}

impl Parent {
    fn prune(&mut self, now: Instant) {
        self.waiting
            .retain(|_, waiting| now.duration_since(waiting.seen) < WAITING_TTL);
        self.order.retain(|id| self.waiting.contains_key(id));
        self.decided
            .retain(|_, (_, at)| now.duration_since(*at) < DECIDED_TTL);
    }

    fn is_empty(&self) -> bool {
        self.waiting.is_empty() && self.decided.is_empty()
    }
}

#[derive(Default)]
pub(in crate::server) struct Approvals {
    parents: Mutex<HashMap<String, Parent>>,
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
            Some((Decision::Decline, _)) => return Gate::Declined,
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
        entry.waiting.insert(
            request.request_id.clone(),
            Waiting {
                agent: agent.to_owned(),
                purpose: serde_json::to_value(request.purpose).unwrap_or(Value::Null),
                summary: summary(request),
                reason,
                seen: now,
            },
        );
        Gate::Pending
    }

    /// An admitted request no longer needs its decision.
    pub(super) fn admitted(&self, parent: &str, request_id: &str) {
        let mut parents = self.parents.lock();
        if let Some(entry) = parents.get_mut(parent) {
            entry.decided.remove(request_id);
            entry.waiting.remove(request_id);
            entry.order.retain(|id| id != request_id);
            if entry.is_empty() {
                parents.remove(parent);
            }
        }
    }

    /// Apply a decision to every request waiting now; returns their agents,
    /// or `None` when nothing was waiting.
    pub(in crate::server) fn decide(
        &self,
        parent: &str,
        decision: Decision,
    ) -> Option<Vec<String>> {
        let now = Instant::now();
        let mut parents = self.parents.lock();
        let entry = parents.get_mut(parent)?;
        entry.prune(now);
        if entry.waiting.is_empty() {
            return None;
        }
        let agents: BTreeSet<String> = entry
            .waiting
            .values()
            .map(|waiting| waiting.agent.clone())
            .collect();
        for (request_id, _) in entry.waiting.drain() {
            entry.decided.insert(request_id, (decision, now));
        }
        entry.order.clear();
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
                "agent": item.agent,
                "purpose": item.purpose,
                "summary": item.summary,
            })).collect::<Vec<_>>(),
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
            approvals.decide("p", Decision::Once),
            Some(vec!["claude-code".to_owned(), "codex".to_owned()])
        );
        assert!(approvals.view("p", "claude-code").is_none());
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Approved
        );
        approvals.admitted("p", "r1");
        // Admission consumes a once-approval.
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Pending
        );
        assert_eq!(approvals.decide("p", Decision::Decline).unwrap().len(), 1);
        assert_eq!(
            approvals.refused("p", &request("r1"), "codex", "calls_disabled"),
            Gate::Declined
        );
        assert!(approvals.decide("other", Decision::Once).is_none());
    }
}

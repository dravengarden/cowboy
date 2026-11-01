//! Task notification occurrences (docs/task-notifications.md).

use serde_json::Value;

/// How long an occurrence may still be delivered.
const INPUT_TTL_MS: i64 = 60 * 60 * 1_000;
const FAILURE_TTL_MS: i64 = 60 * 60 * 1_000;
const COMPLETION_TTL_MS: i64 = 15 * 60 * 1_000;
/// Failures and completions wait for their burst.
const AGGREGATION_WINDOW_MS: i64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Input,
    Failure,
    Completion,
}

impl Category {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Failure => "failure",
            Self::Completion => "completion",
        }
    }

    #[must_use]
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "input" => Some(Self::Input),
            "failure" => Some(Self::Failure),
            "completion" => Some(Self::Completion),
            _ => None,
        }
    }

    #[must_use]
    pub fn delay_ms(self) -> i64 {
        match self {
            Self::Input => 0,
            Self::Failure | Self::Completion => AGGREGATION_WINDOW_MS,
        }
    }

    #[must_use]
    pub fn ttl_ms(self) -> i64 {
        match self {
            Self::Input => INPUT_TTL_MS,
            Self::Failure => FAILURE_TTL_MS,
            Self::Completion => COMPLETION_TTL_MS,
        }
    }
}

/// One occurrence as its producer knows it. The owner comes from the session
/// row, so a system or unowned session writes nothing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Occurrence {
    pub session_id: String,
    pub dedupe_key: String,
    pub category: Category,
    /// A task key, or empty when the session itself is the destination.
    pub destination: String,
    /// The task's acknowledgement marker, or empty.
    pub marker: String,
}

impl Occurrence {
    #[must_use]
    pub fn permission(session_id: &str, seq: u64) -> Self {
        Self {
            session_id: session_id.to_owned(),
            dedupe_key: format!("req:perm:{session_id}:{seq}"),
            category: Category::Input,
            destination: String::new(),
            marker: String::new(),
        }
    }

    #[must_use]
    pub fn approval(parent: &str, request_id: &str) -> Self {
        Self {
            session_id: parent.to_owned(),
            dedupe_key: format!("{APPROVAL_PREFIX}{parent}:{request_id}"),
            category: Category::Input,
            destination: String::new(),
            marker: String::new(),
        }
    }

    #[must_use]
    pub fn incident(session_id: &str, incident_id: &str) -> Self {
        Self {
            session_id: session_id.to_owned(),
            dedupe_key: format!("incident:{incident_id}"),
            category: Category::Failure,
            destination: String::new(),
            marker: String::new(),
        }
    }

    #[must_use]
    pub fn subagent(session_id: &str, child: &str, terminal_seq: u64, category: Category) -> Self {
        Self {
            session_id: session_id.to_owned(),
            dedupe_key: format!("entity:{session_id}:{child}:{terminal_seq}"),
            category,
            destination: format!("subagent:{child}"),
            marker: terminal_seq.to_string(),
        }
    }

    #[must_use]
    pub fn call(parent: &str, call_id: &str, category: Category, finished_at_ms: i64) -> Self {
        Self {
            session_id: parent.to_owned(),
            dedupe_key: format!("exec:{call_id}:terminal"),
            category,
            destination: format!("call:{call_id}"),
            marker: finished_at_ms.to_string(),
        }
    }
}

pub const APPROVAL_PREFIX: &str = "req:approval:";
pub const PERMISSION_PREFIX: &str = "req:perm:";

/// What a terminal subagent snapshot says happened. `None` for an outcome that
/// never notifies: stopped on request, or ended without one.
#[must_use]
pub fn subagent_outcome(update: &Value) -> Option<Category> {
    let errored = update
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| !error.is_empty());
    match update.get("stopReason").and_then(Value::as_str) {
        Some("end_turn") if errored => Some(Category::Failure),
        Some("end_turn") => Some(Category::Completion),
        Some("error" | "refusal" | "max_tokens" | "max_turn_requests") => Some(Category::Failure),
        None if errored => Some(Category::Failure),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn outcomes_match_the_presented_status() {
        let outcome = |update: Value| subagent_outcome(&update);
        assert_eq!(
            outcome(json!({"stopReason":"end_turn"})),
            Some(Category::Completion)
        );
        assert_eq!(
            outcome(json!({"stopReason":"end_turn","error":"boom"})),
            Some(Category::Failure)
        );
        assert_eq!(
            outcome(json!({"stopReason":"max_tokens"})),
            Some(Category::Failure)
        );
        assert_eq!(outcome(json!({"stopReason":"cancelled"})), None);
        assert_eq!(
            outcome(json!({"stopReason":null})),
            None,
            "Ended is not an outcome"
        );
    }

    #[test]
    fn occurrences_reuse_nothing_across_worker_restarts() {
        assert_ne!(
            Occurrence::permission("s", 10).dedupe_key,
            Occurrence::permission("s", 90).dedupe_key,
        );
        assert_eq!(
            Occurrence::approval("p", "op").dedupe_key,
            Occurrence::approval("p", "op").dedupe_key,
        );
    }
}

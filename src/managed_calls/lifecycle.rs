//! Effect ownership and terminal-state rules shared by storage and adapters.
//! Connection health never resets the execution state or grants redispatch.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Queued,
    Starting,
    Running,
    WaitingInput,
    Stopping,
    Completed,
    Failed,
    Cancelled,
}

impl State {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::WaitingInput => "waiting_input",
            Self::Stopping => "stopping",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Only proven, persisted observations may drive these transitions.
    /// `Starting` never transitions back to `Queued` after a timeout/restart.
    pub fn permits(self, next: Self) -> bool {
        if self.terminal() {
            return false;
        }
        matches!(
            (self, next),
            (Self::Queued, Self::Starting | Self::Cancelled | Self::Failed)
            | (Self::Starting, Self::Running | Self::Stopping | Self::Failed)
            | (Self::Running, Self::WaitingInput | Self::Stopping | Self::Completed | Self::Failed)
            | (Self::WaitingInput, Self::Running | Self::Stopping | Self::Completed | Self::Failed)
            // Completion can win a cancellation race. Preserve actual success;
            // never turn it into Cancelled just because a stop was requested.
            | (Self::Stopping, Self::Completed | Self::Failed | Self::Cancelled)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub service_id: String,
    pub parent_session_id: String,
    pub parent_revision: String,
    pub machine_id: String,
    pub workspace_id: String,
    pub cwd: String,
}

impl Placement {
    /// Same parent, Service and execution location. The parent revision is
    /// deliberately excluded: an owner or Provider-generation change revokes
    /// new admissions, but an existing child conversation stays where it is.
    pub fn same_location(&self, other: &Self) -> bool {
        self.service_id == other.service_id
            && self.parent_session_id == other.parent_session_id
            && self.machine_id == other.machine_id
            && self.workspace_id == other.workspace_id
            && self.cwd == other.cwd
    }

    pub fn validate(&self) -> bool {
        [
            &self.service_id,
            &self.parent_session_id,
            &self.machine_id,
            &self.workspace_id,
        ]
        .into_iter()
        .all(|id| super::valid_id(id))
            && !self.parent_revision.is_empty()
            && self.parent_revision.len() <= 256
            && self.cwd.starts_with('/')
            && self.cwd.len() <= 4096
            && !self.cwd.contains('\0')
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema: u16,
    pub call_id: String,
    pub request_id: String,
    pub request_digest: String,
    pub provider: String,
    pub purpose: super::Purpose,
    pub access: super::Access,
    pub labels: std::collections::BTreeMap<String, String>,
    pub placement: Placement,
    pub child_session_id: String,
    pub state: State,
    pub revision: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// An immutable snapshot lease from the target, never the live cwd alone.
    pub input_revision: Option<String>,
    /// Retained structured result, not a shell exit code or review verdict.
    pub result: Option<serde_json::Value>,
    /// The parent's runtime Machine at admission; placement is the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_machine_id: Option<String>,
    /// Exact child Provider generation pinned when the launch was claimed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_generation_digest: Option<String>,
    /// Child event sequence before this call's prompt; its turn follows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_cursor: Option<u64>,
    /// A durable stop request. It is not evidence that the child stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_requested_at_ms: Option<i64>,
    /// Classified terminal cause; the original output stays in `result`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CallError>,
    /// The child agent's configuration preset chosen by the parent's policy;
    /// absent keeps the agent's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// How the agent was chosen: `explicit` or `auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallError {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CallError {
    pub fn new(code: &str, detail: Option<String>) -> Self {
        Self {
            code: code.to_owned(),
            detail: detail.map(|detail| detail.chars().take(512).collect()),
        }
    }

    fn validate(&self) -> bool {
        !self.code.is_empty()
            && self.code.len() <= 64
            && self
                .code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_')
            && self
                .detail
                .as_ref()
                .is_none_or(|detail| detail.chars().count() <= 512)
    }
}

/// Values that may be set once and then never change.
fn once<T: PartialEq>(previous: &Option<T>, next: &Option<T>) -> bool {
    previous.is_none() || previous == next
}

/// Bounded list projection. Full results are fetched only when opening a call;
/// a page of one hundred one-megabyte reviews must not block a mobile overview.
#[derive(Serialize)]
pub struct Summary<'a> {
    pub schema: u16,
    pub call_id: &'a str,
    pub request_id: &'a str,
    pub provider: &'a str,
    pub purpose: super::Purpose,
    pub access: super::Access,
    pub labels: &'a std::collections::BTreeMap<String, String>,
    pub placement: &'a Placement,
    pub child_session_id: &'a str,
    pub state: State,
    pub revision: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub input_revision: &'a Option<String>,
    pub has_result: bool,
    pub runtime_machine_id: &'a Option<String>,
    pub provider_version: &'a Option<String>,
    pub cancel_requested: bool,
    pub error: &'a Option<CallError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<&'a str>,
    /// Optional projection of a structured review result. Execution success
    /// and review findings are distinct; the caller owns their disposition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding_count: Option<usize>,
}

impl Record {
    pub fn summary(&self) -> Summary<'_> {
        Summary {
            schema: self.schema,
            call_id: &self.call_id,
            request_id: &self.request_id,
            provider: &self.provider,
            purpose: self.purpose,
            access: self.access,
            labels: &self.labels,
            placement: &self.placement,
            child_session_id: &self.child_session_id,
            state: self.state,
            revision: self.revision,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
            input_revision: &self.input_revision,
            has_result: self.result.is_some(),
            runtime_machine_id: &self.runtime_machine_id,
            provider_version: &self.provider_version,
            cancel_requested: self.cancel_requested_at_ms.is_some(),
            error: &self.error,
            preset: self.preset.as_deref(),
            selection: self.selection.as_deref(),
            verdict: self
                .result
                .as_ref()
                .and_then(|result| result.pointer("/structured/verdict"))
                .and_then(serde_json::Value::as_str)
                .filter(|verdict| verdict.len() <= 64),
            finding_count: self
                .result
                .as_ref()
                .and_then(|result| result.pointer("/structured/findings"))
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
        }
    }

    pub fn validate(&self) -> bool {
        self.schema == 1
            && super::valid_id(&self.call_id)
            && super::valid_id(&self.request_id)
            && super::valid_id(&self.child_session_id)
            && self.placement.validate()
            && matches!(self.provider.as_str(), "codex" | "claude-code")
            && super::valid_labels(&self.labels)
            && valid_digest(&self.request_digest)
            && self.revision > 0
            && self.revision <= i64::MAX as u64
            && self.created_at_ms >= 0
            && self.updated_at_ms >= self.created_at_ms
            && self
                .input_revision
                .as_ref()
                .is_none_or(|digest| valid_digest(digest))
            && (self.state != State::Completed
                || (self.result.is_some() && self.input_revision.is_some()))
            && self.result.as_ref().is_none_or(|result| {
                serde_json::to_vec(result).is_ok_and(|bytes| bytes.len() <= 1024 * 1024)
            })
            && self
                .runtime_machine_id
                .as_ref()
                .is_none_or(|id| super::valid_id(id))
            && self
                .provider_version
                .as_ref()
                .is_none_or(|version| !version.is_empty() && version.len() <= 64)
            && self
                .provider_generation_digest
                .as_ref()
                .is_none_or(|digest| !digest.is_empty() && digest.len() <= 128)
            && self.error.as_ref().is_none_or(CallError::validate)
            && (self.error.is_none() || matches!(self.state, State::Failed | State::Cancelled))
            && self
                .preset
                .as_ref()
                .is_none_or(|preset| !preset.is_empty() && preset.len() <= 128)
            && self
                .selection
                .as_deref()
                .is_none_or(|selection| matches!(selection, "explicit" | "auto"))
    }

    /// Check a CAS replacement independently of backend SQL. Placement, native
    /// child identity and request meaning cannot change during recovery.
    pub fn accepts(&self, next: &Self) -> bool {
        self.validate()
            && next.validate()
            && self.state.permits(next.state)
            && self.call_id == next.call_id
            && self.request_id == next.request_id
            && self.request_digest == next.request_digest
            && self.provider == next.provider
            && self.purpose == next.purpose
            && self.access == next.access
            && self.labels == next.labels
            && self.placement == next.placement
            && self.child_session_id == next.child_session_id
            && self.created_at_ms == next.created_at_ms
            && next.updated_at_ms >= self.updated_at_ms
            && self.revision.checked_add(1) == Some(next.revision)
            && once(&self.input_revision, &next.input_revision)
            && once(&self.runtime_machine_id, &next.runtime_machine_id)
            && self.preset == next.preset
            && self.selection == next.selection
            && once(&self.provider_version, &next.provider_version)
            && once(
                &self.provider_generation_digest,
                &next.provider_generation_digest,
            )
            && once(&self.child_cursor, &next.child_cursor)
            && once(&self.cancel_requested_at_ms, &next.cancel_requested_at_ms)
            && once(&self.result, &next.result)
    }

    /// A same-state revision that only records a durable stop request.
    pub fn accepts_cancel_request(&self, next: &Self) -> bool {
        let mut expected = self.clone();
        expected.revision = self.revision.saturating_add(1);
        expected.updated_at_ms = next.updated_at_ms;
        expected.cancel_requested_at_ms = next.cancel_requested_at_ms;
        !self.state.terminal()
            && self.cancel_requested_at_ms.is_none()
            && next.cancel_requested_at_ms.is_some()
            && next.updated_at_ms >= self.updated_at_ms
            && expected == *next
    }
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn record() -> Record {
        Record {
            schema: 1,
            call_id: "call-1".into(),
            request_id: "review-1".into(),
            request_digest: format!("sha256:{}", "a".repeat(64)),
            provider: "codex".into(),
            purpose: super::super::Purpose::Review,
            access: super::super::Access::ReadOnly,
            labels: Default::default(),
            placement: Placement {
                service_id: "svc-1".into(),
                parent_session_id: "parent-1".into(),
                parent_revision: "binding-1".into(),
                machine_id: "hawk".into(),
                workspace_id: "project".into(),
                cwd: "/workspace/task".into(),
            },
            child_session_id: "child-1".into(),
            state: State::Queued,
            revision: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
            input_revision: None,
            result: None,
            runtime_machine_id: Some("ovh".into()),
            provider_version: None,
            provider_generation_digest: None,
            child_cursor: None,
            cancel_requested_at_ms: None,
            error: None,
            preset: None,
            selection: None,
        }
    }

    #[test]
    fn cannot_requeue_an_ambiguous_launch_or_rewrite_a_terminal_result() {
        assert!(!State::Starting.permits(State::Queued));
        assert!(!State::Running.permits(State::Starting));
        for state in [State::Completed, State::Failed, State::Cancelled] {
            for next in [
                State::Queued,
                State::Starting,
                State::Running,
                State::Stopping,
                State::Completed,
                State::Cancelled,
                State::Failed,
            ] {
                assert!(!state.permits(next));
            }
        }
        assert!(State::Stopping.permits(State::Completed));
        assert!(!State::Running.permits(State::Cancelled));
        assert!(State::Queued.permits(State::Cancelled));
    }

    #[test]
    fn recovery_cannot_rebind_parent_machine_or_native_conversation() {
        let current = record();
        let mut next = current.clone();
        next.state = State::Starting;
        next.revision = 2;
        assert!(current.accepts(&next));
        next.placement.machine_id = "ovh".into();
        assert!(!current.accepts(&next));
        next.placement = current.placement.clone();
        next.child_session_id = "another-child".into();
        assert!(!current.accepts(&next));
        // The child's runtime is chosen once, when the call is claimed.
        let mut queued = current.clone();
        queued.runtime_machine_id = None;
        let mut claimed = queued.clone();
        claimed.state = State::Starting;
        claimed.revision = 2;
        claimed.runtime_machine_id = Some("ovh".into());
        assert!(queued.accepts(&claimed));
        let mut moved = claimed.clone();
        moved.state = State::Running;
        moved.revision = 3;
        moved.runtime_machine_id = Some("hawk".into());
        assert!(!claimed.accepts(&moved));
    }

    #[test]
    fn completion_requires_captured_result_and_snapshot() {
        let mut value = record();
        value.state = State::Completed;
        assert!(!value.validate());
        value.result = Some(serde_json::json!({"findings":[]}));
        assert!(!value.validate());
        value.input_revision = Some(format!("sha256:{}", "b".repeat(64)));
        assert!(value.validate());
    }

    #[test]
    fn overview_does_not_serialize_or_clone_review_results() {
        let mut value = record();
        value.result = Some(serde_json::json!({"report":"private-report".repeat(60_000)}));
        let summary = serde_json::to_value(value.summary()).unwrap();
        assert_eq!(summary["has_result"], true);
        assert_eq!(summary["call_id"], value.call_id);
        assert_eq!(summary["revision"], value.revision);
        assert!(summary.get("result").is_none());
        assert!(serde_json::to_vec(&summary).unwrap().len() < 2048);
        // Opening the call still returns the original complete result.
        assert!(
            value.result.as_ref().unwrap()["report"]
                .as_str()
                .unwrap()
                .len()
                > 500_000
        );
        value.result = None;
        assert!(!value.summary().has_result);
    }
}

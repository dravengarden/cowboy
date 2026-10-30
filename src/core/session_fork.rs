//! Durable lineage and one-shot native fork intent of a forked session.
//!
//! `session_fork:<destination>` names the source and, until the destination's
//! native thread exists, the source thread to fork. It is both the receipt of
//! the creating operation and the `forked_from` mark of the session list.
use super::{Hub, settings_keys};
use crate::agent_model::Event;
use serde::{Deserialize, Serialize};

/// `sessionUpdate` of the first event of a forked session. It counts as
/// context for the resume rule: the forked native thread holds history.
pub const FORK_ORIGIN_UPDATE: &str = "cowboy_fork_origin";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkState {
    /// The destination's first worker must still run `session/fork`.
    Pending,
    /// The forked thread exists; later launches resume it like any other.
    Forked,
    /// The fork's context was cleared; it is an ordinary session now.
    Discarded,
    /// Creation was admitted but its result could not carry the fork. The
    /// destination never forks, and its operation reports the refusal.
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkRecord {
    pub source_session_id: String,
    pub source_agent_session_id: String,
    /// The message the caller asked to fork from; `None` asked for all of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// The answer the native fork ends at. A whole fork pins the source's last
    /// answer at admission, so later source turns never leak into it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<String>,
    /// Caller operation that created this destination; a retry with the same
    /// operation and request observes this destination instead of forking twice.
    pub operation: String,
    pub state: ForkState,
}

fn key(destination: &str) -> String {
    format!("{}{destination}", settings_keys::SESSION_FORK_PREFIX)
}

pub(super) fn is_fork_origin(update: &serde_json::Value) -> bool {
    update
        .get("sessionUpdate")
        .and_then(serde_json::Value::as_str)
        == Some(FORK_ORIGIN_UPDATE)
}

/// Where a requested fork point stands in the source's loaded transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkPoint {
    /// The last assistant message of a turn that has ended.
    Completed,
    /// The message ends a turn that is still running.
    Running,
    /// Another assistant message follows it in the same turn.
    NotTurnFinal,
    /// Older than the loaded tail; the adapter validates it.
    NotLoaded,
    Missing,
}

fn update_kind(envelope: &crate::core::Envelope) -> Option<(&str, Option<&str>)> {
    let Event::Update { update } = &envelope.event else {
        return None;
    };
    let kind = update.get("sessionUpdate")?.as_str()?;
    // A message steered into the running turn does not end that turn.
    if kind == "user_message_chunk"
        && update
            .pointer("/_meta/cowboyDelivery")
            .and_then(serde_json::Value::as_str)
            == Some("current_turn")
    {
        return Some(("steering", None));
    }
    Some((
        kind,
        update.get("messageId").and_then(serde_json::Value::as_str),
    ))
}

impl Hub {
    /// Locate a Provider message ID in the source's loaded transcript.
    #[must_use]
    pub fn fork_point(&self, source: &str, message_id: &str) -> ForkPoint {
        let sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get(source) else {
            return ForkPoint::Missing;
        };
        let Some(index) = session.log.iter().rposition(|envelope| {
            update_kind(envelope) == Some(("agent_message_chunk", Some(message_id)))
        }) else {
            return if session.reached_start {
                ForkPoint::Missing
            } else {
                ForkPoint::NotLoaded
            };
        };
        for envelope in &session.log[index + 1..] {
            match update_kind(envelope) {
                Some(("user_message_chunk", _)) => return ForkPoint::Completed,
                Some(("agent_message_chunk", Some(other))) if other != message_id => {
                    return ForkPoint::NotTurnFinal;
                }
                _ => {}
            }
        }
        if session.meta.status == crate::agent_model::Status::Busy {
            ForkPoint::Running
        } else {
            ForkPoint::Completed
        }
    }

    /// The source's newest answer in its current context, if the agent named it.
    #[must_use]
    pub fn last_answer_id(&self, source: &str) -> Option<String> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(source)?;
        for envelope in session.log.iter().rev() {
            match update_kind(envelope) {
                Some(("agent_message_chunk", Some(id))) => return Some(id.to_owned()),
                Some(("context_cleared", _)) => return None,
                _ => {}
            }
        }
        None
    }

    #[must_use]
    pub fn session_fork(&self, destination: &str) -> Option<ForkRecord> {
        self.setting(&key(destination))
            .and_then(|value| serde_json::from_value(value).ok())
    }

    /// The destination `source` already created for `operation`.
    #[must_use]
    pub fn fork_by_operation(&self, source: &str, operation: &str) -> Option<(String, ForkRecord)> {
        self.session_list().into_iter().find_map(|meta| {
            let record = self.session_fork(&meta.id)?;
            (record.source_session_id == source && record.operation == operation)
                .then_some((meta.id, record))
        })
    }

    /// Record a reserved destination before it is created, so its workspace
    /// preparation already sees the fork base.
    pub fn stage_fork(&self, destination: &str, record: &ForkRecord) {
        self.set_setting(
            key(destination),
            serde_json::to_value(record).expect("fork record serializes"),
        );
    }

    /// Discard a staged fork whose destination was never created.
    pub fn unstage_fork(&self, destination: &str) {
        if self.session_info(destination).is_none() && self.setting(&key(destination)).is_some() {
            self.set_setting(key(destination), serde_json::Value::Null);
        }
    }

    /// Open the created destination's transcript with the fork marker before
    /// its first worker launch.
    pub fn open_fork(&self, destination: &str, record: &ForkRecord, source_title: &str) {
        self.set_forked_from(destination, Some(record.source_session_id.clone()));
        self.push(
            destination,
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": FORK_ORIGIN_UPDATE,
                    "sourceSessionId": record.source_session_id,
                    "sourceTitle": source_title,
                    "messageId": record.message_id,
                }),
            },
        );
    }

    pub(super) fn native_fork_pending(&self, destination: &str) -> bool {
        self.session_fork(destination)
            .is_some_and(|record| record.state == ForkState::Pending)
    }

    /// The fork's own thread exists and has not been cleared since.
    pub(super) fn native_fork_settled(&self, destination: &str) -> bool {
        self.session_fork(destination)
            .is_some_and(|record| record.state == ForkState::Forked)
    }

    pub(super) fn discard_native_fork(&self, destination: &str) {
        self.set_fork_state(destination, ForkState::Discarded);
    }

    /// Keep the lineage but never fork this destination.
    pub fn refuse_native_fork(&self, destination: &str) {
        self.set_fork_state(destination, ForkState::Refused);
    }

    fn set_fork_state(&self, destination: &str, state: ForkState) {
        let Some(mut record) = self.session_fork(destination) else {
            return;
        };
        if record.state == state {
            return;
        }
        record.state = state;
        self.set_setting(
            key(destination),
            serde_json::to_value(record).expect("fork record serializes"),
        );
    }

    /// The fork the destination's next worker must perform, if it has no
    /// native thread yet.
    #[must_use]
    pub fn pending_native_fork(
        &self,
        destination: &str,
    ) -> Option<crate::runtime_wire::NativeFork> {
        let record = self.session_fork(destination)?;
        if record.state != ForkState::Pending {
            return None;
        }
        let has_thread = self
            .inner
            .sessions
            .lock()
            .get(destination)
            .is_some_and(|session| session.meta.agent_session_id.is_some());
        (!has_thread).then_some(crate::runtime_wire::NativeFork {
            agent_session_id: record.source_agent_session_id,
            message_id: record.point.or(record.message_id),
        })
    }

    /// The destination now has its own thread: later launches resume it.
    pub(super) fn settle_native_fork(&self, destination: &str) {
        if self.native_fork_pending(destination) {
            self.set_fork_state(destination, ForkState::Forked);
        }
    }

    /// Mark `session_id` as a fork of `source` (or restore that mark).
    pub fn set_forked_from(&self, session_id: &str, source: Option<String>) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            if s.meta.forked_from == source {
                return;
            }
            s.meta.forked_from = source;
        }
        self.broadcast_sessions();
    }

    /// Project persisted fork marks after settings and sessions load.
    pub fn restore_forks(&self) {
        for meta in self.session_list() {
            if let Some(record) = self.session_fork(&meta.id) {
                self.set_forked_from(&meta.id, Some(record.source_session_id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{SessionOrigin, SessionRegistration};

    fn session(hub: &Hub, id: &str) {
        hub.create_session(SessionRegistration {
            id: id.to_owned(),
            provider: "claude-code".to_owned(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".to_owned(),
            title: id.to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
    }

    fn record(message_id: Option<&str>) -> ForkRecord {
        ForkRecord {
            source_session_id: "source".to_owned(),
            source_agent_session_id: "native-source".to_owned(),
            message_id: message_id.map(str::to_owned),
            point: None,
            operation: "op-1".to_owned(),
            state: ForkState::Pending,
        }
    }

    #[test]
    fn a_pending_fork_launches_once_and_its_thread_resumes_before_a_prompt() {
        let hub = Hub::new();
        session(&hub, "source");
        session(&hub, "fork");
        hub.stage_fork("fork", &record(Some("msg-1")));
        hub.open_fork("fork", &record(Some("msg-1")), "Source");

        assert_eq!(
            hub.pending_native_fork("fork"),
            Some(crate::runtime_wire::NativeFork {
                agent_session_id: "native-source".to_owned(),
                message_id: Some("msg-1".to_owned()),
            })
        );
        assert_eq!(hub.agent_session_id_for_resume("fork"), None);
        assert_eq!(
            hub.session_info("fork")
                .unwrap()
                .meta
                .forked_from
                .as_deref(),
            Some("source")
        );

        hub.set_agent_session_id("fork", "native-fork".to_owned());
        assert_eq!(hub.pending_native_fork("fork"), None);
        assert_eq!(hub.session_fork("fork").unwrap().state, ForkState::Forked);
        // The marker is context: the forked thread resumes with no prompt yet.
        assert_eq!(
            hub.agent_session_id_for_resume("fork").as_deref(),
            Some("native-fork")
        );
    }

    fn chunk(hub: &Hub, kind: &str, message_id: Option<&str>) {
        hub.push(
            "source",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": kind,
                    "messageId": message_id,
                    "content": {"type": "text", "text": "x"},
                }),
            },
        );
    }

    #[test]
    fn only_the_last_answer_of_an_ended_turn_is_a_fork_point() {
        let hub = Hub::new();
        session(&hub, "source");
        hub.set_status("source", crate::agent_model::Status::Running, None);
        chunk(&hub, "user_message_chunk", None);
        chunk(&hub, "agent_message_chunk", Some("a1"));
        chunk(&hub, "tool_call", None);
        chunk(&hub, "agent_message_chunk", Some("a2"));
        chunk(&hub, "agent_message_chunk", Some("a2"));
        chunk(&hub, "user_message_chunk", None);
        chunk(&hub, "agent_message_chunk", Some("b1"));
        hub.push(
            "source",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "_meta": {"cowboyDelivery": "current_turn"},
                    "content": {"type": "text", "text": "steer"},
                }),
            },
        );
        chunk(&hub, "agent_message_chunk", Some("b2"));
        assert_eq!(hub.fork_point("source", "b1"), ForkPoint::NotTurnFinal);
        assert_eq!(hub.fork_point("source", "a1"), ForkPoint::NotTurnFinal);
        assert_eq!(hub.fork_point("source", "a2"), ForkPoint::Completed);
        assert_eq!(hub.fork_point("source", "zz"), ForkPoint::Missing);
        assert_eq!(hub.fork_point("source", "b2"), ForkPoint::Completed);
        hub.set_status("source", crate::agent_model::Status::Busy, None);
        assert_eq!(hub.fork_point("source", "b2"), ForkPoint::Running);
        assert_eq!(hub.fork_point("source", "a2"), ForkPoint::Completed);
    }

    #[test]
    fn clearing_a_fork_discards_its_source_before_and_after_the_first_launch() {
        let hub = Hub::new();
        session(&hub, "source");
        session(&hub, "fork");
        hub.stage_fork("fork", &record(None));
        hub.prepare_context_reset("fork");
        hub.mark_context_cleared("fork");
        assert_eq!(hub.pending_native_fork("fork"), None);
        assert_eq!(
            hub.session_fork("fork").unwrap().state,
            ForkState::Discarded
        );
        // A fresh thread after the clear is not resumable before a prompt.
        hub.set_agent_session_id("fork", "fresh".to_owned());
        assert_eq!(hub.agent_session_id_for_resume("fork"), None);

        session(&hub, "fork-2");
        hub.stage_fork("fork-2", &record(None));
        hub.set_agent_session_id("fork-2", "native-fork".to_owned());
        assert_eq!(
            hub.agent_session_id_for_resume("fork-2").as_deref(),
            Some("native-fork")
        );
        hub.prepare_context_reset("fork-2");
        hub.mark_context_cleared("fork-2");
        hub.set_agent_session_id("fork-2", "fresh".to_owned());
        assert_eq!(hub.agent_session_id_for_resume("fork-2"), None);
    }

    #[test]
    fn a_fork_without_its_marker_still_resumes_its_thread() {
        let hub = Hub::new();
        session(&hub, "source");
        session(&hub, "fork");
        hub.stage_fork("fork", &record(None));
        hub.set_agent_session_id("fork", "native-fork".to_owned());
        assert_eq!(
            hub.agent_session_id_for_resume("fork").as_deref(),
            Some("native-fork")
        );
    }

    #[test]
    fn a_refused_fork_never_launches_its_source() {
        let hub = Hub::new();
        session(&hub, "source");
        session(&hub, "fork");
        hub.stage_fork("fork", &record(None));
        hub.refuse_native_fork("fork");
        assert_eq!(hub.pending_native_fork("fork"), None);
    }

    #[test]
    fn an_operation_names_its_destination() {
        let hub = Hub::new();
        session(&hub, "source");
        session(&hub, "fork");
        hub.stage_fork("fork", &record(None));
        assert_eq!(
            hub.fork_by_operation("source", "op-1").map(|(id, _)| id),
            Some("fork".to_owned())
        );
        assert_eq!(hub.fork_by_operation("source", "op-2"), None);
        assert_eq!(hub.fork_by_operation("other", "op-1"), None);
    }
}

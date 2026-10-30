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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkRecord {
    pub source_session_id: String,
    pub source_agent_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
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

impl Hub {
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

    /// Record a newly admitted destination before its first worker launch, and
    /// open its transcript with the fork marker.
    pub fn record_fork(&self, destination: &str, record: &ForkRecord, source_title: &str) {
        self.set_setting(
            key(destination),
            serde_json::to_value(record).expect("fork record serializes"),
        );
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
        (!has_thread).then(|| crate::runtime_wire::NativeFork {
            agent_session_id: record.source_agent_session_id,
            message_id: record.message_id,
        })
    }

    /// The destination now has its own thread (or discarded the fork): never
    /// fork it again, even after its context is cleared.
    pub(super) fn settle_native_fork(&self, destination: &str) {
        let Some(mut record) = self.session_fork(destination) else {
            return;
        };
        if record.state == ForkState::Forked {
            return;
        }
        record.state = ForkState::Forked;
        self.set_setting(
            key(destination),
            serde_json::to_value(record).expect("fork record serializes"),
        );
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
            operation: "op-1".to_owned(),
            state: ForkState::Pending,
        }
    }

    #[test]
    fn a_pending_fork_launches_once_and_its_thread_resumes_before_a_prompt() {
        let hub = Hub::new();
        session(&hub, "source");
        session(&hub, "fork");
        hub.record_fork("fork", &record(Some("msg-1")), "Source");

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

    #[test]
    fn an_operation_names_its_destination() {
        let hub = Hub::new();
        session(&hub, "source");
        session(&hub, "fork");
        hub.record_fork("fork", &record(None), "Source");
        assert_eq!(
            hub.fork_by_operation("source", "op-1").map(|(id, _)| id),
            Some("fork".to_owned())
        );
        assert_eq!(hub.fork_by_operation("source", "op-2"), None);
        assert_eq!(hub.fork_by_operation("other", "op-1"), None);
    }
}

//! Live requests are independent of the bounded transcript and read/unread state.
use super::{Event, Hub, Status};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAttention {
    pub approvals: usize,
    pub questions: usize,
    /// Native recovery knows the request is live but its description left the hot tail.
    #[serde(default)]
    pub other: usize,
}

impl SessionAttention {
    pub fn is_empty(&self) -> bool {
        self.approvals == 0 && self.questions == 0 && self.other == 0
    }
}

#[derive(Default)]
pub(super) struct PendingActions(BTreeMap<String, Request>);

struct Request {
    question: Option<bool>,
    child: bool,
    tool: Option<String>,
}

impl PendingActions {
    pub(super) fn restored(
        log: &[super::Envelope],
        runtime: Option<&super::WorkerSnapshot>,
        meta: &mut super::SessionMeta,
    ) -> Self {
        let mut pending = Self::default();
        if let Some(worker) = runtime {
            for envelope in log {
                pending.observe(&envelope.event);
            }
            if !matches!(
                worker.state,
                super::WorkerState::Exited | super::WorkerState::Crashed
            ) {
                pending.reconcile(&worker.pending_permissions);
            } else {
                pending.0.clear();
            }
        }
        meta.attention = pending.summary();
        pending
    }
    pub(super) fn observe(&mut self, event: &Event) {
        match event {
            Event::PermissionRequest {
                request_id,
                tool_call,
                ..
            } => {
                let input = &tool_call["rawInput"];
                let question = tool_call.pointer("/_cowboy/elicitation").is_some()
                    || (input["question"].is_string() && input["options"].is_array());
                self.0.insert(
                    request_id.clone(),
                    Request {
                        question: Some(question),
                        child: tool_call
                            .pointer("/_cowboy/subagent")
                            .is_some_and(serde_json::Value::is_object),
                        tool: tool_call["toolCallId"].as_str().map(str::to_owned),
                    },
                );
            }
            Event::PermissionResolved { request_id, .. } => {
                self.0.remove(request_id);
            }
            Event::TurnEnd { .. } => self.0.retain(|_, request| request.child),
            Event::Lifecycle {
                status: Status::Exited | Status::Crashed | Status::Interrupted,
                ..
            } => self.0.clear(),
            Event::Update { update } => {
                if update["sessionUpdate"] == "tool_call_update"
                    && matches!(update["status"].as_str(), Some("completed" | "failed"))
                    && let Some(tool) = update["toolCallId"].as_str()
                {
                    self.0
                        .retain(|_, request| request.tool.as_deref() != Some(tool));
                }
                if update["sessionUpdate"] == "context_cleared" {
                    self.0.clear();
                }
            }
            _ => {}
        }
    }

    pub(super) fn reconcile(&mut self, ids: &[String]) {
        self.0.retain(|id, _| ids.contains(id));
        for id in ids {
            self.0.entry(id.clone()).or_insert(Request {
                question: None,
                child: true,
                tool: None,
            });
        }
    }

    pub(super) fn summary(&self) -> SessionAttention {
        let questions = self
            .0
            .values()
            .filter(|request| request.question == Some(true))
            .count();
        let other = self
            .0
            .values()
            .filter(|request| request.question.is_none())
            .count();
        SessionAttention {
            approvals: self.0.len() - questions - other,
            questions,
            other,
        }
    }
}

impl Hub {
    /// Accepted native snapshots restore requests even when their transcript row
    /// has left the hot tail. This is presentation only, never approval authority.
    pub fn reconcile_session_attention(&self, session_id: &str, ids: &[String]) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(session_id) else {
                return;
            };
            // Recover question/tool descriptions from the retained tail after
            // a Controller restart; native ids decide which are still live.
            for envelope in session.log.iter().rev() {
                if let Event::PermissionRequest { request_id, .. } = &envelope.event
                    && ids.contains(request_id)
                    && !session.pending_actions.0.contains_key(request_id)
                {
                    session.pending_actions.observe(&envelope.event);
                }
            }
            session.pending_actions.reconcile(ids);
            let summary = session.pending_actions.summary();
            let changed = session.meta.attention != summary;
            session.meta.attention = summary;
            changed
        };
        if changed {
            self.broadcast_sessions();
        }
    }
}

pub(super) fn prepare_restored_log(
    log: &mut Vec<super::Envelope>,
    status: Status,
    persisted: bool,
    reached_start: &mut bool,
) -> usize {
    for envelope in log.iter_mut() {
        crate::persistence::compact_canonical_tool_output(envelope);
    }
    let mut bytes = log.iter().fold(0usize, |size, envelope| {
        size.saturating_add(super::estimated_envelope_bytes(envelope))
    });
    if persisted
        && super::trim_hot_log(log, &mut bytes, false, super::hot_tail_budget_bytes(status))
    {
        *reached_start = false;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(id: &str, tool: serde_json::Value) -> Event {
        Event::PermissionRequest {
            request_id: id.into(),
            tool_call: tool,
            options: json!([]),
        }
    }

    #[test]
    fn session_attention_resolves_once_and_allows_reused_ids() {
        let mut pending = PendingActions::default();
        let ask = request("1", json!({"title":"Shell"}));
        pending.observe(&ask);
        pending.observe(&ask);
        assert_eq!(pending.summary().approvals, 1);
        let answer = Event::PermissionResolved {
            request_id: "1".into(),
            option_id: None,
        };
        pending.observe(&answer);
        pending.observe(&answer);
        assert!(pending.summary().is_empty());
        pending.observe(&ask);
        assert_eq!(pending.summary().approvals, 1);
    }

    #[test]
    fn session_attention_retains_child_questions_until_answer_or_worker_exit() {
        let mut pending = PendingActions::default();
        pending.observe(&request("parent", json!({})));
        pending.observe(&request(
            "child",
            json!({"_cowboy":{"subagent":{},"elicitation":{"mode":"form"}}}),
        ));
        pending.observe(
            &serde_json::from_value(json!({"kind":"turn_end","stop_reason":"end_turn"})).unwrap(),
        );
        assert_eq!(
            pending.summary(),
            SessionAttention {
                approvals: 0,
                questions: 1,
                other: 0,
            }
        );
        pending.observe(&Event::Lifecycle {
            status: Status::Exited,
            detail: None,
        });
        assert!(pending.summary().is_empty());
    }

    #[test]
    fn session_attention_tool_completion_and_native_reconciliation() {
        let mut pending = PendingActions::default();
        pending.observe(&request(
            "q",
            json!({"toolCallId":"t", "rawInput":{"question":"Which?", "options":[]}}),
        ));
        pending.reconcile(&["q".into(), "outside-tail".into()]);
        assert_eq!(
            pending.summary(),
            SessionAttention {
                approvals: 0,
                questions: 1,
                other: 1,
            }
        );
        pending.observe(&Event::Update { update: json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed"}) });
        assert_eq!(pending.summary().questions, 0);
        pending.reconcile(&[]);
        assert!(pending.summary().is_empty());
    }

    #[test]
    fn session_attention_is_in_connect_snapshot_and_not_cleared_by_reading() {
        let hub = Hub::new();
        let meta: super::super::SessionMeta = serde_json::from_value(
            json!({"id":"s", "provider":"codex", "cwd":"/tmp", "title":"Test", "status":"running"}),
        )
        .unwrap();
        assert!(hub.project_persisted_session(meta));
        hub.push("s", request("1", json!({"title":"Shell"})));
        assert_eq!(hub.session_list()[0].attention.approvals, 1);
        let _ = hub.snapshot("s");
        assert_eq!(hub.session_list()[0].attention.approvals, 1);
        hub.push(
            "s",
            Event::PermissionResolved {
                request_id: "1".into(),
                option_id: None,
            },
        );
        assert!(hub.session_list()[0].attention.is_empty());
    }

    #[test]
    fn session_attention_restores_only_requests_still_owned_by_native_worker() {
        use super::super::runtime_reconciliation_tests::{restored_busy, worker_snapshot};
        let hub = Hub::new();
        let mut restored = restored_busy("recover");
        restored.log.push(super::super::Envelope {
            session_id: "recover".into(),
            seq: 0,
            cmid: None,
            event: request(
                "question",
                json!({"rawInput":{"question":"Where?","options":[]}}),
            ),
        });
        let mut worker = worker_snapshot("recover", "epoch");
        worker.pending_permissions = vec!["question".into(), "trimmed".into()];
        hub.restore_with_workers(vec![restored], &[worker]);
        let attention = hub.session_list()[0].attention.clone();
        assert_eq!(attention.questions, 1);
        assert_eq!(attention.other, 1);
        hub.reconcile_session_attention("recover", &[]);
        assert!(hub.session_list()[0].attention.is_empty());
    }
}

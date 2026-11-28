//! Product-neutral external Trigger session lifecycle projection.
//!
//! Triggered runs persist compact metadata under `session_trigger:<session_id>`
//! so they stay isolated from ordinary human folders, support correlation-key
//! routing for active incidents, auto-hibernate after settling, auto-move to
//! Trash after their outcome-aware TTL, and cleanly promote into ordinary human
//! sessions when a person replies or files them into a workspace folder.

use super::{Hub, Status, settings_keys};
use crate::model::TriggerSessionMeta;

fn key(session_id: &str) -> String {
    format!("{}{session_id}", settings_keys::SESSION_TRIGGER_PREFIX)
}

fn is_settled_outcome(outcome: &str) -> bool {
    matches!(outcome, "resolved" | "suppressed")
}

impl Hub {
    /// Set or clear a session's external Trigger metadata and persist it.
    /// Passing `None` promotes the session into an ordinary interactive session.
    pub fn set_session_trigger(&self, session_id: &str, trigger: Option<TriggerSessionMeta>) {
        let changed = {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(session_id) else {
                return;
            };
            if session.meta.trigger == trigger {
                false
            } else {
                session.meta.trigger = trigger.clone();
                true
            }
        };
        let setting_key = key(session_id);
        match trigger {
            Some(meta) => {
                self.set_setting(
                    setting_key,
                    serde_json::to_value(meta).expect("trigger metadata serializes"),
                );
            }
            None if self.setting(&setting_key).is_some() => {
                self.set_setting(setting_key, serde_json::Value::Null);
            }
            None => {}
        }
        if changed {
            self.broadcast_session(session_id);
        }
    }

    /// Promote a triggered run to an ordinary interactive session when a human
    /// takes over (for example by sending a Web prompt).
    pub fn promote_trigger_if_person(
        &self,
        session_id: &str,
        caller: &crate::prompt_origin::RequestCaller,
    ) {
        if matches!(caller, crate::prompt_origin::RequestCaller::Person)
            && self
                .session_info(session_id)
                .is_some_and(|info| info.meta.trigger.is_some())
        {
            self.set_session_trigger(session_id, None);
        }
    }

    /// Restore persisted `session_trigger:<id>` metadata onto loaded sessions.
    pub fn restore_triggers(&self) {
        let entries: Vec<(String, TriggerSessionMeta)> = self
            .settings_snapshot()
            .into_iter()
            .filter_map(|(k, v)| {
                let id = k.strip_prefix(settings_keys::SESSION_TRIGGER_PREFIX)?;
                if v.is_null() {
                    return None;
                }
                let meta: TriggerSessionMeta = serde_json::from_value(v).ok()?;
                Some((id.to_owned(), meta))
            })
            .collect();
        if entries.is_empty() {
            return;
        }
        let mut sessions = self.inner.sessions.lock();
        for (id, meta) in entries {
            if let Some(session) = sessions.get_mut(&id) {
                session.meta.trigger = Some(meta);
            }
        }
    }

    /// Find the newest live, non-trashed trigger session matching
    /// `correlation_key` whose outcome is still active (not resolved/suppressed).
    #[must_use]
    pub fn find_active_trigger_session(&self, correlation_key: &str) -> Option<String> {
        let trimmed = correlation_key.trim();
        if trimmed.is_empty() {
            return None;
        }
        let order = self.inner.order.lock().clone();
        let sessions = self.inner.sessions.lock();
        for id in order.iter().rev() {
            let Some(session) = sessions.get(id) else {
                continue;
            };
            let meta = &session.meta;
            if meta.closing || meta.trashed_at_ms.is_some() || meta.system {
                continue;
            }
            let Some(trigger) = meta.trigger.as_ref() else {
                continue;
            };
            if trigger.correlation_key.as_deref() == Some(trimmed)
                && !is_settled_outcome(&trigger.outcome)
            {
                return Some(id.clone());
            }
        }
        None
    }

    /// Live, idle, unpromoted trigger sessions whose `auto_trash_at_ms` has elapsed.
    #[must_use]
    pub fn expired_trigger_sessions(&self, now_ms: i64) -> Vec<String> {
        let sessions = self.inner.sessions.lock();
        sessions
            .values()
            .filter_map(|session| {
                let meta = &session.meta;
                if meta.closing
                    || meta.trashed_at_ms.is_some()
                    || meta.system
                    || session.in_flight
                    || meta.status == Status::Busy
                    || meta.background_tasks > 0
                    || !session.queue.is_empty()
                {
                    return None;
                }
                let due = meta.trigger.as_ref()?.auto_trash_at_ms?;
                (due <= now_ms).then(|| meta.id.clone())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{SessionOrigin, SessionRegistration};

    fn add_session(hub: &Hub, id: &str) {
        hub.create_session(SessionRegistration {
            id: id.to_owned(),
            title: id.to_owned(),
            provider: "codex".to_owned(),
            cwd: "/tmp".to_owned(),
            origin: SessionOrigin::Api,
            system: false,
            machine_id: "local".to_owned(),
            provider_account_id: Default::default(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_auth_generation: None,
            provider_behavior: None,
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            owner_user_id: None,
            owner_username: None,
        });
    }

    fn trigger(
        corr: Option<&str>,
        outcome: &str,
        auto_trash_at_ms: Option<i64>,
    ) -> TriggerSessionMeta {
        TriggerSessionMeta {
            kind: "alert.triage.v1".to_owned(),
            source: "monitor".to_owned(),
            correlation_key: corr.map(str::to_owned),
            outcome: outcome.to_owned(),
            settled_at_ms: auto_trash_at_ms.map(|t| t - 1000),
            auto_trash_at_ms,
        }
    }

    #[test]
    fn trigger_metadata_persists_restores_correlates_expires_and_promotes() {
        let hub = Hub::new();
        add_session(&hub, "s1");
        add_session(&hub, "s2");
        hub.set_session_trigger("s1", Some(trigger(Some("ep-disk"), "escalated", Some(500))));
        hub.set_session_trigger("s2", Some(trigger(Some("ep-done"), "resolved", Some(500))));

        assert_eq!(
            hub.find_active_trigger_session("ep-disk").as_deref(),
            Some("s1")
        );
        assert_eq!(hub.find_active_trigger_session("ep-done"), None);

        let mut expired = hub.expired_trigger_sessions(500);
        expired.sort();
        assert_eq!(expired, vec!["s1".to_owned(), "s2".to_owned()]);
        assert!(hub.expired_trigger_sessions(499).is_empty());

        let restored = Hub::new();
        add_session(&restored, "s1");
        restored.load_settings(hub.settings_snapshot().into_iter().collect());
        restored.restore_triggers();
        assert_eq!(
            restored.find_active_trigger_session("ep-disk").as_deref(),
            Some("s1")
        );

        hub.promote_trigger_if_person("s1", &crate::prompt_origin::RequestCaller::Person);
        assert!(hub.session_info("s1").unwrap().meta.trigger.is_none());
        assert_eq!(hub.find_active_trigger_session("ep-disk"), None);
    }
}

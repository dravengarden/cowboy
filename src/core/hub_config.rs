//! Agent configuration options per session: the advertised options, persisted preferences, in-flight overlays and their broadcast.

use super::*;

impl Hub {
    /// Snapshot the captured config options for one session — used by the
    /// WS connect handler to replay the agent's last-seen `configOptions`
    /// to a freshly-connected client (so its composer dropdowns hydrate
    /// without waiting for the next `config_option_update`).
    #[must_use]
    pub fn config_options(&self, session_id: &str) -> Option<serde_json::Value> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).and_then(|session| {
            projected_config_options(
                &session.meta.provider,
                session.meta.provider_behavior.as_ref(),
                &session.config_preferences,
                display_config_options(session),
            )
        })
    }

    /// Supply a preview without allocating a worker or claiming an ACP reply.
    pub fn set_initial_config_options(&self, expected: &SessionMeta, options: serde_json::Value) {
        let mut sessions = self.inner.sessions.lock();
        if let Some(session) = sessions.get_mut(&expected.id)
            && session.meta.provider == expected.provider
            && session.meta.provider_account_id == expected.provider_account_id
            && session.meta.provider_generation_digest == expected.provider_generation_digest
            && session.meta.provider_version == expected.provider_version
            && session.meta.provider_auth_generation == expected.provider_auth_generation
            && session.meta.machine_id == expected.machine_id
            && session.meta.owner_user_id == expected.owner_user_id
            && session.config_options.is_none()
        {
            session.initial_config_options = Some(options);
        }
    }

    /// Return only the last agent-backed options snapshot. Unlike
    /// [`Self::config_options`], this does not synthesize host-owned controls
    /// when the agent has not advertised any options yet.
    pub fn persisted_config_options(&self, session_id: &str) -> Option<serde_json::Value> {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|session| session.config_options.clone())
    }

    /// Return the durable values selected for a session. The returned object is
    /// safe to pass across the Machine boundary because it contains only ACP
    /// option ids and scalar values, never provider credentials.
    #[must_use]
    pub fn config_preferences(&self, session_id: &str) -> Option<serde_json::Value> {
        let sessions = self.inner.sessions.lock();
        sessions
            .get(session_id)
            .map(|session| session.config_preferences.clone())
    }

    /// The value one config option holds now: the durable selection, else the
    /// agent's last advertised current value.
    #[must_use]
    pub fn current_config_value(
        &self,
        session_id: &str,
        config_id: &str,
    ) -> Option<serde_json::Value> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        if let Some(selected) = session.config_preferences.get(config_id) {
            return Some(selected.clone());
        }
        session
            .config_options
            .as_ref()?
            .as_array()?
            .iter()
            .find(|option| option.get("id").and_then(serde_json::Value::as_str) == Some(config_id))
            .and_then(config_current_value)
            .cloned()
    }

    /// Record one user-selected config value and immediately fan out the
    /// optimistic selection. The agent's later authoritative option snapshot
    /// will correct it if the provider normalizes or rejects the value.
    pub fn set_config_preference(
        &self,
        session_id: &str,
        config_id: String,
        value: serde_json::Value,
    ) -> Result<(), String> {
        if config_id.is_empty() || config_id.len() > 128 {
            return Err("configuration id is invalid".to_owned());
        }
        if !matches!(
            &value,
            serde_json::Value::String(_) | serde_json::Value::Bool(_)
        ) {
            return Err("configuration values must be a string id or boolean".to_owned());
        }
        let (preferences, options, persist_options) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(session_id) else {
                return Err(format!("unknown session {session_id:?}"));
            };
            if !session.config_preferences.is_object() {
                session.config_preferences = serde_json::json!({});
            }
            session
                .config_preferences
                .as_object_mut()
                .expect("config preferences are an object")
                .insert(config_id.clone(), value.clone());
            self.inner
                .config_in_flight
                .lock()
                .entry(session_id.to_owned())
                .or_default()
                .insert(
                    config_id.clone(),
                    (value.clone(), std::time::Instant::now()),
                );
            let mut options = projected_config_options(
                &session.meta.provider,
                session.meta.provider_behavior.as_ref(),
                &session.config_preferences,
                display_config_options(session),
            );
            let options = options.as_mut().and_then(|options| {
                set_config_option_current_value(options, &config_id, &value)
                    .then(|| options.clone())
            });
            if let Some(options) = &options
                && session.config_options.is_some()
            {
                session.config_options = Some(options.clone());
            }
            (
                session.config_preferences.clone(),
                options,
                session.config_options.is_some(),
            )
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateConfigPreferences {
                session_id: session_id.to_owned(),
                preferences,
            });
        }
        if let Some(options) = options {
            if let Some(tx) = self.inner.store_tx.as_ref()
                && persist_options
            {
                let _ = tx.send(StoreWrite::UpdateConfigOptions {
                    session_id: session_id.to_owned(),
                    options: options.clone(),
                });
            }
            self.fanout(Outbound::ConfigOptions {
                session_id: session_id.to_owned(),
                options,
            });
        }
        Ok(())
    }

    /// Keep client-set values the agent has not answered yet in an agent
    /// snapshot. Pending values settle together, only when one snapshot
    /// reports every one of them: during rapid preset taps a stale answer can
    /// match one latest value by coincidence while the rest are still queued,
    /// and settling that one alone let the next stale answer flip it back.
    /// Values the agent no longer offers are dropped (unless a model change is
    /// pending, whose old-model snapshots cannot judge the new model's
    /// options), and everything expires so a lost command cannot pin a value
    /// the agent never applied.
    pub(super) fn overlay_config_in_flight(
        &self,
        session_id: &str,
        options: &mut serde_json::Value,
    ) {
        const SETTLE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
        let mut in_flight = self.inner.config_in_flight.lock();
        let Some(pending) = in_flight.get_mut(session_id) else {
            return;
        };
        let find = |options: &serde_json::Value, config_id: &str| {
            options.as_array().and_then(|options| {
                options
                    .iter()
                    .find(|option| {
                        option.get("id").and_then(serde_json::Value::as_str) == Some(config_id)
                    })
                    .cloned()
            })
        };
        let model_pending = pending.iter().any(|(config_id, (value, _))| {
            find(options, config_id).is_some_and(|option| {
                option.get("category").and_then(serde_json::Value::as_str) == Some("model")
                    && config_option_accepts(&option, value)
            })
        });
        pending.retain(|config_id, (value, set_at)| {
            find(options, config_id).is_some_and(|option| {
                set_at.elapsed() <= SETTLE_DEADLINE
                    && (model_pending || config_option_accepts(&option, value))
            })
        });
        let settled = pending.iter().all(|(config_id, (value, _))| {
            find(options, config_id)
                .is_some_and(|option| config_current_value(&option) == Some(value))
        });
        if settled {
            in_flight.remove(session_id);
            return;
        }
        for (config_id, (value, _)) in pending.iter() {
            set_config_option_current_value(options, config_id, value);
        }
    }

    /// Store the latest agent-advertised config options for a session and
    /// fan them out to every client. Called from acp.rs when the upstream
    /// emits a `config_option_update` notification, and from the
    /// `SetConfigOption` reply path (the agent's authoritative response
    /// refreshes the same array).
    pub fn set_config_options(&self, session_id: &str, options: serde_json::Value) {
        let (options, corrected_preferences) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let raw_options = options;
            let mut options = projected_config_options(
                &s.meta.provider,
                s.meta.provider_behavior.as_ref(),
                &s.config_preferences,
                Some(raw_options.clone()),
            )
            .expect("agent config options remain present after projection");
            let corrected = reconcile_config_preferences(Some(&options), &mut s.config_preferences);
            if corrected {
                options = projected_config_options(
                    &s.meta.provider,
                    s.meta.provider_behavior.as_ref(),
                    &s.config_preferences,
                    Some(raw_options),
                )
                .expect("agent config options remain present after projection");
            }
            self.overlay_config_in_flight(session_id, &mut options);
            s.config_options = Some(options.clone());
            (options, corrected.then(|| s.config_preferences.clone()))
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateConfigOptions {
                session_id: session_id.to_owned(),
                options: options.clone(),
            });
            if let Some(preferences) = corrected_preferences {
                let _ = tx.send(StoreWrite::UpdateConfigPreferences {
                    session_id: session_id.to_owned(),
                    preferences,
                });
            }
        }
        self.fanout(Outbound::ConfigOptions {
            session_id: session_id.to_owned(),
            options,
        });
    }
}

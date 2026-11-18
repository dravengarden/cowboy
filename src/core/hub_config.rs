//! Agent configuration options per session: the advertised options, persisted preferences, in-flight overlays and their broadcast.

use super::*;

impl Hub {
    /// Automatic supervision must wait for native preset acknowledgement.
    pub fn config_changes_pending(&self, session_id: &str) -> bool {
        self.inner.config_in_flight.lock().contains_key(session_id)
    }
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
            session.config_previews.initial = Some(options);
        }
    }

    /// Offer an exited session choices its pinned Provider generation has
    /// but its snapshot lacks: a session moved to another generation while
    /// exited keeps the snapshot its previous generation reported until a
    /// worker starts. The choices are display-only: never persisted, never
    /// part of [`Self::persisted_config_options`] (and so never replayed as a
    /// capability), shown only while the session stays exited on the same
    /// generation and login, and dropped by the next agent snapshot. A user's
    /// selection among them is an ordinary preference, which the next worker
    /// validates against its own report. Returns whether the display changed.
    pub fn set_dormant_config_choices(
        &self,
        expected: &SessionMeta,
        snapshot: &serde_json::Value,
        choices: std::collections::BTreeMap<String, Vec<serde_json::Value>>,
    ) -> bool {
        let mut sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get_mut(&expected.id) else {
            return false;
        };
        // The choices were derived from `snapshot`; a worker report that
        // replaced it in the meantime is newer than them.
        if session.meta.status != Status::Exited
            || session.in_flight
            || session.config_options.as_ref() != Some(snapshot)
            || session.meta.provider != expected.provider
            || session.meta.provider_account_id != expected.provider_account_id
            || session.meta.provider_version != expected.provider_version
            || session.meta.provider_generation_digest != expected.provider_generation_digest
            || session.meta.provider_auth_generation != expected.provider_auth_generation
            || session.meta.machine_id != expected.machine_id
            || session.meta.owner_user_id != expected.owner_user_id
        {
            return false;
        }
        let before = display_config_options(session);
        // Every completed lookup replaces the previous one, an empty one too.
        session.config_previews.dormant = (!choices.is_empty()
            && !session.config_previews.reported_by_current(&session.meta))
        .then(|| DormantConfigChoices {
            generation_digest: session.meta.provider_generation_digest.clone(),
            auth_generation: session.meta.provider_auth_generation,
            choices,
        });
        let after = display_config_options(session);
        if before == after {
            return false;
        }
        // Publish under the session lock so a worker snapshot that replaces
        // this one is always broadcast after it.
        if let Some(options) = projected_config_options(
            &session.meta.provider,
            session.meta.provider_behavior.as_ref(),
            &session.config_preferences,
            after,
        ) {
            self.fanout(Outbound::ConfigOptions {
                session_id: expected.id.clone(),
                options,
            });
        }
        true
    }

    /// Whether this Controller saw `meta`'s exact generation and login report
    /// the session's current snapshot; such a snapshot needs no dormant
    /// choices. `None` after a restart, when the producer is unknown.
    #[must_use]
    pub fn config_reported_by_current(&self, meta: &SessionMeta) -> Option<bool> {
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(&meta.id)?;
        session
            .config_previews
            .reported_by
            .as_ref()
            .map(|_| session.config_previews.reported_by_current(&session.meta))
    }

    /// Model choices reported by live workers (Running or Busy) of sessions on
    /// `meta`'s exact, nonempty Provider release, Machine, owner, account and
    /// login, excluding managed children. Read under one lock with each
    /// session's identity, so a reload cannot pair one release's identity with
    /// another's options. `None` when no such worker has reported options.
    #[must_use]
    pub fn live_config_peers(&self, meta: &SessionMeta) -> Option<Vec<serde_json::Value>> {
        if meta.provider_version.is_empty() || meta.provider_generation_digest.is_empty() {
            return None;
        }
        let sessions = self.inner.sessions.lock();
        let peers = sessions
            .values()
            .filter(|peer| {
                let p = &peer.meta;
                p.id != meta.id
                    && matches!(p.status, Status::Running | Status::Busy)
                    && p.execution_binding
                        .as_ref()
                        .is_none_or(|binding| binding.managed().is_none())
                    && p.provider == meta.provider
                    && p.provider_account_id == meta.provider_account_id
                    && p.provider_auth_generation == meta.provider_auth_generation
                    && p.provider_version == meta.provider_version
                    && p.provider_generation_digest == meta.provider_generation_digest
                    && p.machine_id == meta.machine_id
                    && p.owner_user_id == meta.owner_user_id
                    && peer.config_previews.reported_by_current(p)
            })
            .filter_map(|peer| peer.config_options.clone())
            .collect::<Vec<_>>();
        (!peers.is_empty()).then_some(peers)
    }

    /// The options as the agent reported them, without dormant display
    /// choices: for views not scoped to the session's owner.
    #[must_use]
    pub fn reported_config_options(&self, session_id: &str) -> Option<serde_json::Value> {
        let sessions = self.inner.sessions.lock();
        sessions.get(session_id).and_then(|session| {
            projected_config_options(
                &session.meta.provider,
                session.meta.provider_behavior.as_ref(),
                &session.config_preferences,
                stored_config_options(session),
            )
        })
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
            let project = |options| {
                let mut options = projected_config_options(
                    &session.meta.provider,
                    session.meta.provider_behavior.as_ref(),
                    &session.config_preferences,
                    options,
                );
                options.as_mut().and_then(|options| {
                    set_config_option_current_value(options, &config_id, &value)
                        .then(|| options.clone())
                })
            };
            // Persist what the agent reported, never dormant display choices.
            let stored = project(stored_config_options(session));
            let options = project(display_config_options(session));
            if let Some(stored) = stored
                && session.config_options.is_some()
            {
                session.config_options = Some(stored);
            }
            // Publish under the session lock, like a dormant overlay, so a
            // worker snapshot replacing either is always broadcast after it.
            if let Some(options) = &options {
                self.fanout(Outbound::ConfigOptions {
                    session_id: session_id.to_owned(),
                    options: options.clone(),
                });
            }
            (
                session.config_preferences.clone(),
                options.is_some(),
                session.config_options.clone(),
            )
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateConfigPreferences {
                session_id: session_id.to_owned(),
                preferences,
            });
        }
        if options
            && let Some(tx) = self.inner.store_tx.as_ref()
            && let Some(stored) = persist_options
        {
            let _ = tx.send(StoreWrite::UpdateConfigOptions {
                session_id: session_id.to_owned(),
                options: stored,
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
            s.config_previews.dormant = None;
            s.config_previews.reported_by = Some((
                s.meta.provider_generation_digest.clone(),
                s.meta.provider_auth_generation,
            ));
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

/// Append each addition `{value, name}` to the flat choice list of the option
/// with the same id when no choice has its value. Grouped choice lists are left
/// alone: an ungrouped entry would change how the agent's groups render.
pub(super) fn add_config_choices(
    snapshot: &mut serde_json::Value,
    additions: &std::collections::BTreeMap<String, Vec<serde_json::Value>>,
) -> bool {
    let Some(options) = snapshot.as_array_mut() else {
        return false;
    };
    let mut changed = false;
    for option in options {
        let Some(id) = option.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(additions) = additions.get(id) else {
            continue;
        };
        let Some(choices) = option
            .get_mut("options")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        if choices
            .iter()
            .any(|choice| choice.get("options").is_some() || choice.get("value").is_none())
        {
            continue;
        }
        for addition in additions {
            let Some(value) = addition.get("value") else {
                continue;
            };
            if !choices
                .iter()
                .any(|choice| choice.get("value") == Some(value))
            {
                choices.push(addition.clone());
                changed = true;
            }
        }
    }
    changed
}

/// Option previews shown before, or instead of, a current agent snapshot.
#[derive(Default)]
pub(super) struct ConfigPreviews {
    /// Signed defaults/presets before the first ACP snapshot.
    pub(super) initial: Option<serde_json::Value>,
    /// Choices of an exited session's pinned generation that its snapshot,
    /// reported by an earlier generation, lacks; see
    /// `Hub::set_dormant_config_choices`.
    pub(super) dormant: Option<DormantConfigChoices>,
    /// The generation digest and login of the worker that reported the
    /// current snapshot, when this Controller saw it.
    pub(super) reported_by: Option<(String, Option<u64>)>,
}

impl ConfigPreviews {
    fn reported_by_current(&self, meta: &SessionMeta) -> bool {
        self.reported_by.as_ref().is_some_and(|(digest, login)| {
            *digest == meta.provider_generation_digest && *login == meta.provider_auth_generation
        })
    }
}

/// Display-only choices of an exited session's pinned generation.
pub(super) struct DormantConfigChoices {
    generation_digest: String,
    auth_generation: Option<u64>,
    pub(super) choices: std::collections::BTreeMap<String, Vec<serde_json::Value>>,
}

impl DormantConfigChoices {
    /// Only while the session stays exited on the generation and login the
    /// choices were gathered for.
    pub(super) fn applies_to(&self, session: &Session) -> bool {
        session.meta.status == Status::Exited
            && !session.in_flight
            && session.meta.provider_generation_digest == self.generation_digest
            && session.meta.provider_auth_generation == self.auth_generation
    }
}

//! Session metadata projection and ordered live session-list fan-out.
//!
//! Single-session mutations emit compact `Outbound::SessionUpdated` /
//! `Outbound::SessionRemoved` deltas stamped with a strictly monotonic
//! `session_fanout_seq` while holding `sessions`. Connect bootstraps read the
//! filtered client list and current sequence under the same lock hierarchy
//! (`sessions` -> `provider_update_offers` -> `order`), so live fan-out can
//! skip frames already reflected in the bootstrap snapshot without missing or
//! reordering concurrent changes.

use super::*;

/// Maximum number of affected sessions that fan out as individual deltas
/// before coalescing into a single full `Outbound::Sessions` frame.
pub(super) const SESSION_DELTA_BATCH_LIMIT: usize = 8;

fn derive_session_meta(
    session: &Session,
    offers: &HashMap<String, ProviderUpdateOffer>,
    now: i64,
) -> SessionMeta {
    let mut meta = session.meta.clone();
    // Surface the soonest scheduled-draft fire so the session-row
    // clock badge can show it, without shipping the drafts here.
    meta.next_schedule_ms = session
        .drafts
        .iter()
        .filter_map(|m| m.schedule.as_ref().map(|sc| sc.fire_at_ms))
        .min();
    meta.provider_update_available = offers
        .get(&session.meta.id)
        .filter(|offer| {
            meta.provider_update.is_none() && offer.digest != meta.provider_generation_digest
        })
        .map(|offer| ProviderUpdateAvailable {
            version: offer.version.clone(),
            digest: offer.digest.clone(),
            when_idle: offer.when_idle,
            on_open: offer.on_open,
            automatic_at_ms: offer.automatic_after.map(|after| {
                let remaining = after.saturating_sub(session.last_activity.elapsed());
                now.saturating_add(i64::try_from(remaining.as_millis()).unwrap_or(i64::MAX))
            }),
        });
    meta
}

fn build_session_list(
    sessions: &HashMap<String, Session>,
    order: &[String],
    offers: &HashMap<String, ProviderUpdateOffer>,
    now: i64,
    keep: impl Fn(Option<&str>) -> bool,
    for_client: bool,
) -> Vec<SessionMeta> {
    order
        .iter()
        .filter_map(|id| {
            sessions.get(id).and_then(|s| {
                keep(s.meta.owner_user_id.as_deref()).then(|| {
                    let meta = derive_session_meta(s, offers, now);
                    if for_client { meta.for_client() } else { meta }
                })
            })
        })
        .collect()
}

impl Hub {
    /// Current session list (insertion order), retaining internal Provider
    /// behavior contracts for Controller-side callers.
    #[must_use]
    pub fn session_list(&self) -> Vec<SessionMeta> {
        self.session_list_filtered(|_| true)
    }

    /// Session list after applying `keep` to each row's `owner_user_id`,
    /// retaining internal Provider behavior contracts for Controller callers.
    #[must_use]
    pub fn session_list_filtered(&self, keep: impl Fn(Option<&str>) -> bool) -> Vec<SessionMeta> {
        let now = now_ms();
        let sessions = self.inner.sessions.lock();
        let offers = self.inner.provider_update_offers.lock();
        let order = self.inner.order.lock();
        build_session_list(&sessions, &order, &offers, now, keep, false)
    }

    /// Client-projected session list (`provider_behavior` stripped) after
    /// applying `keep` to each row's `owner_user_id`.
    #[must_use]
    pub fn session_list_for_client(&self, keep: impl Fn(Option<&str>) -> bool) -> Vec<SessionMeta> {
        self.session_list_for_client_with_seq(keep).0
    }

    /// Atomically capture a client-projected session list and the session
    /// fan-out sequence watermark it already reflects.
    #[must_use]
    pub fn session_list_for_client_with_seq(
        &self,
        keep: impl Fn(Option<&str>) -> bool,
    ) -> (Vec<SessionMeta>, u64) {
        let now = now_ms();
        let sessions = self.inner.sessions.lock();
        let offers = self.inner.provider_update_offers.lock();
        let order = self.inner.order.lock();
        let list = build_session_list(&sessions, &order, &offers, now, keep, true);
        let seq = self.inner.session_fanout_seq.load(Ordering::Acquire);
        (list, seq)
    }

    /// Atomically capture the client-projected session list, fan-out sequence
    /// watermark, and global `SyncPatch` resync frames under one consistent
    /// lock boundary (`sync` -> `sessions` -> `provider_update_offers` -> `order`).
    /// This guarantees `"title"` / `"order"` sync versions and values match the
    /// exact session list and `sessions_seq` returned to both delta-aware and
    /// legacy WebSocket clients.
    #[must_use]
    pub fn global_bootstrap_snapshot_for_client(
        &self,
        keep: impl Fn(Option<&str>) -> bool,
    ) -> (Vec<SessionMeta>, u64, Vec<Outbound>) {
        let now = now_ms();
        let (list, seq, title_patch, order_patch, other_states) = {
            let reg = self.inner.sync.lock();
            let sessions = self.inner.sessions.lock();
            let offers = self.inner.provider_update_offers.lock();
            let order = self.inner.order.lock();

            let list = build_session_list(&sessions, &order, &offers, now, keep, true);
            let seq = self.inner.session_fanout_seq.load(Ordering::Acquire);

            let title_value = serde_json::Value::Object(
                sessions
                    .values()
                    .map(|s| {
                        (
                            s.meta.id.clone(),
                            serde_json::Value::String(s.meta.title.clone()),
                        )
                    })
                    .collect(),
            );
            let (title_version, title_confirmed) = reg.get("title").map_or_else(
                || (0, Vec::new()),
                |entry| (entry.version, entry.seen.iter().cloned().collect()),
            );
            let title_patch = Outbound::SyncPatch {
                state: "title".to_owned(),
                version: title_version,
                value: title_value,
                confirmed: title_confirmed,
                resync: true,
            };

            let order_value = serde_json::Value::Array(
                order
                    .iter()
                    .map(|id| serde_json::Value::String(id.clone()))
                    .collect(),
            );
            let (order_version, order_confirmed) = reg.get("order").map_or_else(
                || (0, Vec::new()),
                |entry| (entry.version, entry.seen.iter().cloned().collect()),
            );
            let order_patch = Outbound::SyncPatch {
                state: "order".to_owned(),
                version: order_version,
                value: order_value,
                confirmed: order_confirmed,
                resync: true,
            };

            let mut other_states: Vec<(String, u64, Vec<String>)> = reg
                .iter()
                .filter(|(s, _)| {
                    *s != "title"
                        && *s != "order"
                        && !s.starts_with("queue:")
                        && !s.starts_with("mobile-review:")
                })
                .map(|(s, e)| (s.clone(), e.version, e.seen.iter().cloned().collect()))
                .collect();
            for state in ["folders", "workspace-order"] {
                if !other_states.iter().any(|(s, _, _)| s == state) {
                    let version = reg.get(state).map_or(0, |e| e.version);
                    other_states.push((state.to_owned(), version, Vec::new()));
                }
            }
            for session_id in sessions.keys() {
                let state = format!("mobile-review:{session_id}");
                if !other_states
                    .iter()
                    .any(|(existing, _, _)| existing == &state)
                {
                    let version = reg.get(&state).map_or(0, |entry| entry.version);
                    other_states.push((state, version, Vec::new()));
                }
            }
            (list, seq, title_patch, order_patch, other_states)
        };

        let mut resyncs = Vec::with_capacity(other_states.len() + 2);
        resyncs.push(title_patch);
        resyncs.push(order_patch);
        for (state, version, confirmed) in other_states {
            resyncs.push(Outbound::SyncPatch {
                value: self.sync_value(&state),
                state,
                version,
                confirmed,
                resync: true,
            });
        }
        (list, seq, resyncs)
    }

    /// Replace every session's Provider update offer. Emits per-session deltas
    /// when only a small batch changed, or a full session list otherwise.
    pub fn publish_provider_update_offers(&self, offers: HashMap<String, ProviderUpdateOffer>) {
        let sessions = self.inner.sessions.lock();
        let mut current = self.inner.provider_update_offers.lock();
        if *current == offers {
            return;
        }
        let mut affected: Vec<String> = Vec::new();
        for (id, session) in sessions.iter() {
            let old = current.get(id).filter(|offer| {
                session.meta.provider_update.is_none()
                    && offer.digest != session.meta.provider_generation_digest
            });
            let new = offers.get(id).filter(|offer| {
                session.meta.provider_update.is_none()
                    && offer.digest != session.meta.provider_generation_digest
            });
            if old != new {
                affected.push(id.clone());
            }
        }
        *current = offers;
        if affected.is_empty() {
            return;
        }
        let now = now_ms();
        if affected.len() <= SESSION_DELTA_BATCH_LIMIT {
            for id in affected {
                if let Some(session) = sessions.get(&id) {
                    let meta = derive_session_meta(session, &current, now).for_client();
                    let owner = meta.owner_user_id.clone();
                    let seq = self.next_session_fanout_seq();
                    self.publish_session_frame(
                        Outbound::SessionUpdated {
                            session: Box::new(meta),
                        },
                        seq,
                        Some(owner),
                    );
                }
            }
        } else {
            let order = self.inner.order.lock();
            let list = build_session_list(&sessions, &order, &current, now, |_| true, true);
            drop(order);
            let seq = self.next_session_fanout_seq();
            self.publish_session_frame(Outbound::Sessions { sessions: list }, seq, None);
        }
    }

    fn next_session_fanout_seq(&self) -> u64 {
        self.inner.session_fanout_seq.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(super) fn broadcast_sessions(&self) {
        let now = now_ms();
        let sessions = self.inner.sessions.lock();
        let offers = self.inner.provider_update_offers.lock();
        let order = self.inner.order.lock();
        let list = build_session_list(&sessions, &order, &offers, now, |_| true, true);
        drop(order);
        drop(offers);
        let seq = self.next_session_fanout_seq();
        self.publish_session_frame(Outbound::Sessions { sessions: list }, seq, None);
    }

    pub(super) fn broadcast_session(&self, session_id: &str) {
        let sessions = self.inner.sessions.lock();
        self.broadcast_session_locked(&sessions, session_id);
    }

    pub(super) fn broadcast_session_locked(
        &self,
        sessions: &HashMap<String, Session>,
        session_id: &str,
    ) {
        let Some(session) = sessions.get(session_id) else {
            return;
        };
        let now = now_ms();
        let offers = self.inner.provider_update_offers.lock();
        let meta = derive_session_meta(session, &offers, now).for_client();
        drop(offers);
        let owner = meta.owner_user_id.clone();
        let seq = self.next_session_fanout_seq();
        self.publish_session_frame(
            Outbound::SessionUpdated {
                session: Box::new(meta),
            },
            seq,
            Some(owner),
        );
    }

    pub(super) fn broadcast_session_removed(
        &self,
        session_id: &str,
        owner_user_id: Option<String>,
    ) {
        let sessions = self.inner.sessions.lock();
        if sessions.contains_key(session_id) {
            return;
        }
        let seq = self.next_session_fanout_seq();
        self.publish_session_frame(
            Outbound::SessionRemoved {
                session_id: session_id.to_owned(),
            },
            seq,
            Some(owner_user_id),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register_owned(hub: &Hub, id: &str, owner: Option<&str>) {
        hub.create_session(SessionRegistration {
            id: id.to_owned(),
            provider: "codex".to_owned(),
            provider_version: "1.0.0".to_owned(),
            provider_generation_digest: "sha256:old".to_owned(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: Some(crate::provider::legacy_behavior("codex")),
            machine_id: "hawk".to_owned(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".to_owned(),
            title: id.to_owned(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: owner.map(str::to_owned),
            owner_username: owner.map(str::to_owned),
        });
    }

    #[test]
    fn single_session_mutations_emit_deltas_without_provider_behavior() {
        let hub = Hub::new();
        let mut rx = hub.subscribe();
        register_owned(&hub, "s1", Some("u1"));
        let created = rx.try_recv().expect("create frame");
        assert_eq!(created.sessions_seq(), Some(1));
        assert_eq!(created.session_owner(), Some(Some("u1")));
        let Outbound::SessionUpdated { session } = created.outbound() else {
            panic!("expected SessionUpdated on create");
        };
        assert_eq!(session.id, "s1");
        assert!(session.provider_behavior.is_none());
        // Internal callers retain provider_behavior.
        assert!(hub.session_list()[0].provider_behavior.is_some());
        assert!(
            hub.session_list_for_client(|_| true)[0]
                .provider_behavior
                .is_none()
        );

        hub.set_background_tasks("s1", 2);
        let updated = rx.try_recv().expect("update frame");
        assert_eq!(updated.sessions_seq(), Some(2));
        let Outbound::SessionUpdated { session } = updated.outbound() else {
            panic!("expected SessionUpdated on background_tasks");
        };
        assert_eq!(session.background_tasks, 2);
        assert!(session.provider_behavior.is_none());

        hub.set_agent_session_id("s1", "native-1".to_owned());
        let native_set = rx.try_recv().expect("agent_session_id set frame");
        assert!(matches!(
            native_set.outbound(),
            Outbound::SessionUpdated { session } if session.agent_session_id.as_deref() == Some("native-1")
        ));
        // Duplicate set does not emit another frame.
        hub.set_agent_session_id("s1", "native-1".to_owned());
        assert!(rx.try_recv().is_err());

        hub.prepare_context_reset("s1");
        let cleared = rx.try_recv().expect("context reset clears native id");
        assert!(matches!(
            cleared.outbound(),
            Outbound::SessionUpdated { session } if session.agent_session_id.is_none()
        ));

        assert!(hub.delete_session("s1"));
        // Deleting a session also clears its per-session settings (such as the
        // cleared-native fence recorded by `prepare_context_reset`), which may
        // emit non-session preference frames before `SessionRemoved`.
        let removed = std::iter::from_fn(|| rx.try_recv().ok())
            .find(|frame| frame.sessions_seq().is_some())
            .expect("remove frame");
        assert_eq!(removed.sessions_seq(), Some(5));
        assert_eq!(removed.session_owner(), Some(Some("u1")));
        assert!(matches!(
            removed.outbound(),
            Outbound::SessionRemoved { session_id } if session_id == "s1"
        ));

        let (list, seq) = hub.session_list_for_client_with_seq(|_| true);
        assert!(list.is_empty());
        assert_eq!(seq, 5);
    }

    #[test]
    fn provider_update_offers_emit_deltas_for_small_batches_and_full_list_for_large() {
        let hub = Hub::new();
        for index in 0..10 {
            register_owned(&hub, &format!("s{index}"), Some("u1"));
        }
        let mut rx = hub.subscribe();
        let offer = |digest: &str| ProviderUpdateOffer {
            version: "2.0.0".to_owned(),
            digest: digest.to_owned(),
            when_idle: false,
            on_open: false,
            automatic_after: None,
        };
        hub.publish_provider_update_offers(HashMap::from([
            ("s0".to_owned(), offer("sha256:new")),
            ("s1".to_owned(), offer("sha256:new")),
        ]));
        let mut updated_ids = Vec::new();
        for _ in 0..2 {
            let frame = rx.try_recv().expect("delta offer frame");
            let Outbound::SessionUpdated { session } = frame.outbound() else {
                panic!("expected SessionUpdated for small batch");
            };
            assert!(session.provider_update_available.is_some());
            updated_ids.push(session.id.clone());
        }
        updated_ids.sort();
        assert_eq!(updated_ids, vec!["s0", "s1"]);
        assert!(rx.try_recv().is_err());

        // Changing all 10 exceeds SESSION_DELTA_BATCH_LIMIT and coalesces.
        let bulk: HashMap<_, _> = (0..10)
            .map(|index| (format!("s{index}"), offer("sha256:newer")))
            .collect();
        hub.publish_provider_update_offers(bulk);
        let frame = rx.try_recv().expect("bulk offer frame");
        assert!(matches!(
            frame.outbound(),
            Outbound::Sessions { sessions } if sessions.len() == 10
        ));
        assert!(hub.memory_stats().broadcast_last_bytes > 2_000);
        assert!(rx.try_recv().is_err());
    }
}

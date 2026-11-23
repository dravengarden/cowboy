use super::*;
use std::sync::OnceLock;

/// One immutable live frame shared by every WebSocket and the Web Push observer.
/// Structured `Outbound` stays available for observers; JSON is serialized at
/// most once and reused for every socket write.
#[derive(Debug)]
pub struct FanoutFrame {
    outbound: Outbound,
    native_wait_started: bool,
    json: OnceLock<String>,
}

impl FanoutFrame {
    fn new(outbound: Outbound, native_wait_started: bool) -> Arc<Self> {
        Arc::new(Self {
            outbound,
            native_wait_started,
            json: OnceLock::new(),
        })
    }

    #[must_use]
    pub fn outbound(&self) -> &Outbound {
        &self.outbound
    }

    /// The Hub saw a start transition in the canonical native snapshots.
    /// This hint is local to live subscribers.
    #[must_use]
    pub fn native_wait_started(&self) -> bool {
        self.native_wait_started
    }

    /// Cached JSON for this frame. Concurrent first writers may serialize twice;
    /// `OnceLock` keeps one buffer for the ring's lifetime.
    pub fn json(&self) -> Result<&str, serde_json::Error> {
        if let Some(json) = self.json.get() {
            return Ok(json);
        }
        let encoded = serde_json::to_string(&self.outbound)?;
        Ok(self.json.get_or_init(|| encoded))
    }
}

impl std::ops::Deref for FanoutFrame {
    type Target = Outbound;

    fn deref(&self) -> &Self::Target {
        &self.outbound
    }
}

impl Hub {
    pub(super) fn fanout(&self, outbound: Outbound) {
        self.publish_frame(outbound, false);
    }

    fn publish_frame(&self, outbound: Outbound, native_wait_started: bool) {
        // A send error just means no clients or internal observers are
        // connected. The canonical hot tail remains authoritative.
        let bytes = match &outbound {
            Outbound::Event { envelope } => estimated_envelope_bytes(envelope),
            _ => 256,
        };
        self.inner
            .broadcast_last_bytes
            .store(bytes, Ordering::Relaxed);
        let _ = self
            .inner
            .tx
            .send(FanoutFrame::new(outbound, native_wait_started));
    }

    fn prepare_attention_event(&self, session_id: &str, event: Event) -> Event {
        let mut event = event;
        crate::persistence::compact_inbound_event(&mut event);
        if let Some(artifacts) = self.inner.artifacts.lock().clone()
            && let Event::Update { update } = &mut event
            && let Err(error) = artifacts.externalize_images(update)
        {
            tracing::warn!(%error, session_id, "live image externalize failed");
        }
        event
    }
    /// Append an event to a session's log under the next `seq` and fan it out.
    /// Unknown sessions are ignored (a race with teardown).
    pub fn push(&self, session_id: &str, event: Event) {
        self.push_tagged(session_id, event, None);
    }

    /// Like [`Self::push`] but stamps a live `cmid` on the broadcast envelope —
    /// used to tag a dispatched prompt's user-message echo so the originating
    /// client reconciles its optimistic bubble (see Envelope::cmid).
    pub fn push_tagged(&self, session_id: &str, event: Event, cmid: Option<String>) {
        let mut event = self.prepare_attention_event(session_id, event);
        // A subagent snapshot is fenced by its producer identity (I4) before
        // it receives a seq: an exact duplicate or an older revision of the
        // same worker incarnation never becomes a newer observation.
        let subagent_child = live_subagent_child(&event).map(str::to_owned);
        let mut subagent_anchor = None;
        let (envelope, durable_agent_session_id, attention_changed, native_wait_started) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let seq = s.next_seq;
            let mut subagent_ending = None;
            if let (Some(child), Event::Update { update }) = (&subagent_child, &event) {
                match self.accept_subagent_snapshot(session_id, child, update, seq) {
                    Some(ending) => subagent_ending = Some(ending),
                    None => return,
                }
            }
            Self::annotate_dispatched_prompt(s, &mut event, cmid.as_deref());
            // A permission answered on two devices at once resolves once. The
            // second answer is the same fact: the first row stays and no
            // duplicate is appended (docs/offline-first-sync.md, conflict 7).
            //
            // "Same" means the same request, not the same id: the id is the
            // agent's JSON-RPC request id, which starts over with every agent
            // process. Only a resolution newer than the id's latest request
            // answers it; an older one belongs to an earlier request that
            // happened to reuse the number, and dropping this answer for it
            // would leave the request pending in every client forever.
            if let Event::PermissionResolved { request_id, .. } = &event
                && permission_already_resolved(s.log.iter().map(|entry| &entry.event), request_id)
            {
                return;
            }
            s.pending_actions.observe(&event);
            let attention = s.pending_actions.summary();
            let attention_changed = s.meta.attention != attention;
            s.meta.attention = attention;
            s.next_seq += 1;
            if let Some(boundary) = context_rollover::HistoryBoundary::from_event(&event) {
                s.history_boundary = boundary;
            }
            let native_wait_started = subagent_child
                .as_deref()
                .is_some_and(|child| native_wait_start(s, &event, child, subagent_ending));
            if counts_as_activity(&event) {
                s.last_activity = std::time::Instant::now();
            }
            if subagent_child.is_some() {
                stamp_subagent_snapshot(&mut event, seq, subagent_ending);
            }
            let envelope = Envelope {
                session_id: session_id.to_owned(),
                seq,
                event,
                cmid,
            };
            // A send begins one turn at its first chunk, however many blocks follow.
            if crate::prompt_origin::is_owner_chunk(&envelope)
                && !s
                    .log
                    .last()
                    .is_some_and(crate::prompt_origin::is_owner_chunk)
            {
                s.owner_turn_seq = Some(seq);
            }
            if is_user_message_chunk(&envelope)
                && let Some(cmid) = envelope.cmid.as_deref()
                && let Some(dispatched) =
                    s.dispatched_cmids.iter_mut().find(|seen| seen.cmid == cmid)
            {
                // The bounded hot transcript can evict this echo before a
                // disconnected sender retries. Keep delivery evidence for as
                // long as its dispatch record still prevents re-execution.
                dispatched.echoed = true;
            }
            if let Some(canonical) = self.inner.history_reducer.lock().reduce(envelope.clone()) {
                if subagent_child.is_some() {
                    subagent_anchor = Some(canonical.seq);
                }
                match s
                    .log
                    .binary_search_by_key(&canonical.seq, |entry| entry.seq)
                {
                    Ok(index) => {
                        s.log_bytes = s
                            .log_bytes
                            .saturating_sub(estimated_envelope_bytes(&s.log[index]))
                            .saturating_add(estimated_envelope_bytes(&canonical));
                        s.log[index] = canonical;
                    }
                    Err(_) if canonical.seq == seq => {
                        s.log_bytes = s
                            .log_bytes
                            .saturating_add(estimated_envelope_bytes(&canonical));
                        s.log.push(canonical);
                        s.event_count = s.event_count.saturating_add(1);
                    }
                    // The canonical row was already trimmed from the hot tail.
                    // Its durable UPSERT still lands below, but re-inserting an
                    // old seq here would break the tail's sorted cursor contract.
                    Err(_) => {}
                }
            }
            if self.inner.store_tx.is_some()
                && trim_hot_log(
                    &mut s.log,
                    &mut s.log_bytes,
                    true,
                    hot_tail_budget_bytes(s.meta.status),
                )
            {
                s.reached_start = false;
            }
            let durable_agent_session_id = is_user_message_chunk(&envelope)
                .then(|| s.meta.agent_session_id.clone())
                .flatten();
            if let (Some(child), Some(anchor), Event::Update { update }) =
                (subagent_child, subagent_anchor, &envelope.event)
            {
                self.record_subagent_observation(session_id, child, anchor, update.clone());
            }
            (
                envelope,
                durable_agent_session_id,
                attention_changed,
                native_wait_started,
            )
        };
        self.finish_pushed_event(
            session_id,
            envelope,
            durable_agent_session_id,
            attention_changed,
            native_wait_started,
        );
    }

    fn finish_pushed_event(
        &self,
        session_id: &str,
        envelope: Envelope,
        durable_agent_session_id: Option<String>,
        attention_changed: bool,
        native_wait_started: bool,
    ) {
        self.observe_compaction(session_id, &envelope.event);
        if is_user_message_chunk(&envelope)
            && let Some(cmid) = envelope
                .cmid
                .as_deref()
                .filter(|id| id.starts_with(crate::agent_model::CALL_NOTICE_PREFIX))
        {
            self.clear_notice_claim(session_id, cmid);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::AppendEvent(envelope.clone()));
            if let Some(agent_session_id) = durable_agent_session_id {
                let _ = tx.send(StoreWrite::SetAgentSessionId {
                    session_id: session_id.to_owned(),
                    agent_session_id: Some(agent_session_id),
                });
            }
        }
        self.publish_frame(Outbound::Event { envelope }, native_wait_started);
        if attention_changed {
            self.broadcast_sessions();
        }
    }
}

fn stamp_subagent_snapshot(event: &mut Event, seq: u64, ending: Option<SubagentEnding>) {
    let Event::Update { update } = event else {
        return;
    };
    let Some(object) = update.as_object_mut() else {
        return;
    };
    // I2: order snapshots by Controller receipt seq.
    object.insert("observedSeq".to_owned(), serde_json::Value::from(seq));
    // One terminal occurrence keeps its seq through later metadata updates.
    let terminal = match ending {
        Some(SubagentEnding::Ended(terminal)) => Some(terminal),
        Some(SubagentEnding::AlreadyEnded(terminal)) => terminal,
        _ => None,
    };
    if let Some(terminal) = terminal {
        object.insert("terminalSeq".to_owned(), serde_json::Value::from(terminal));
    } else {
        object.remove("terminalSeq");
    }
}

fn native_wait_start(
    session: &Session,
    event: &Event,
    child: &str,
    transition: Option<SubagentEnding>,
) -> bool {
    let Some(SubagentEnding::Working {
        first_snapshot,
        after_idle,
    }) = transition
    else {
        return false;
    };
    if !matches!(event, Event::Update { update } if update["state"] == "running") {
        return false;
    }
    // Restarts may only warm a suffix of the durable transcript. An absent
    // mark can prove a first occurrence only with complete known history.
    after_idle
        || (first_snapshot
            && session.reached_start
            && session
                .log
                .iter()
                .all(|row| live_subagent_child(&row.event) != Some(child)))
}

fn live_subagent_child(event: &Event) -> Option<&str> {
    match event {
        Event::Update { update }
            if update
                .get("sessionUpdate")
                .and_then(serde_json::Value::as_str)
                == Some(crate::transcript_anchors::SUBAGENT_UPDATE) =>
        {
            update
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .filter(|child| crate::transcript_anchors::valid_id(child))
        }
        _ => None,
    }
}

/// An idle sandboxed agent whose account-sandbox connection closed (the
/// daemon restarted for maintenance) ended without losing any work: record a
/// sleeping session, which the next prompt starts again, instead of a crash.
/// A loss during a turn or startup stays a crash.
pub(super) fn idle_sandbox_loss(
    was: Status,
    in_flight: bool,
    status: Status,
    detail: Option<&str>,
) -> bool {
    was == Status::Running
        && !in_flight
        && status == Status::Crashed
        && detail.is_some_and(|detail| {
            detail.starts_with("agent subprocess exited mid-session (account sandbox ")
        })
}

#[cfg(test)]
mod idle_sandbox_loss_tests {
    use super::*;

    #[test]
    fn only_an_idle_sandbox_connection_loss_becomes_a_sleep() {
        let lost = Some(
            "agent subprocess exited mid-session (account sandbox connection lost: writing to the account sandbox daemon: Broken pipe (os error 32))",
        );
        assert!(idle_sandbox_loss(
            Status::Running,
            false,
            Status::Crashed,
            lost
        ));
        assert!(!idle_sandbox_loss(
            Status::Busy,
            false,
            Status::Crashed,
            lost
        ));
        assert!(!idle_sandbox_loss(
            Status::Running,
            true,
            Status::Crashed,
            lost
        ));
        assert!(!idle_sandbox_loss(
            Status::Starting,
            false,
            Status::Crashed,
            lost
        ));
        assert!(!idle_sandbox_loss(
            Status::Running,
            false,
            Status::Crashed,
            Some("acp connection: x")
        ));
        assert!(!idle_sandbox_loss(
            Status::Running,
            false,
            Status::Exited,
            lost
        ));
    }

    #[test]
    fn a_hub_records_the_idle_loss_as_exited() {
        let hub = Hub::new();
        hub.create_local_session(
            "agy".to_owned(),
            "antigravity".to_owned(),
            "/tmp".to_owned(),
            "t".to_owned(),
            SessionOrigin::Web,
            false,
        );
        hub.set_status("agy", Status::Running, None);
        hub.set_status(
            "agy",
            Status::Crashed,
            Some(
                "agent subprocess exited mid-session (account sandbox unit cga-x ended (failed))"
                    .into(),
            ),
        );
        assert_eq!(hub.status("agy"), Some(Status::Exited));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::runtime_reconciliation_tests::restored_busy;

    #[test]
    fn live_frame_only_marks_proven_subagent_starts() {
        for complete_history in [true, false] {
            let hub = Hub::new();
            let mut session = restored_busy("parent");
            session.reached_start = complete_history;
            hub.restore_reconciling_runtime(vec![session]);
            let mut stream = hub.subscribe();
            for (revision, (state, incarnation, expected)) in [
                ("running", "worker", complete_history),
                ("running", "worker", false),
                ("idle", "worker", false),
                ("running", "worker", true),
                ("running", "reconnected-worker", false),
            ]
            .into_iter()
            .enumerate()
            {
                hub.push(
                    "parent",
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": crate::transcript_anchors::SUBAGENT_UPDATE,
                            "sessionId": "child", "incarnation": incarnation,
                            "state": state, "revision": revision + 1,
                        }),
                    },
                );
                let frame = stream.try_recv().unwrap();
                assert_eq!(frame.native_wait_started(), expected);
                assert!(!frame.json().unwrap().contains("native_wait_started"));
            }
        }
    }

    #[test]
    fn terminal_metadata_updates_retain_canonical_terminal_seq_without_fabricating_one() {
        let hub = Hub::new();
        let mut session = restored_busy("parent");
        session.reached_start = true;
        hub.restore_reconciling_runtime(vec![session]);
        let mut stream = hub.subscribe();
        let mut push_snapshot = |child: &str, state: &str, rev: u64, summary: &str| {
            hub.push(
                "parent",
                Event::Update {
                    update: serde_json::json!({
                        "sessionUpdate": crate::transcript_anchors::SUBAGENT_UPDATE,
                        "sessionId": child, "incarnation": "worker",
                        "state": state, "revision": rev, "summary": summary,
                    }),
                },
            );
            let frame = stream.try_recv().unwrap();
            let Outbound::Event { envelope } = frame.outbound() else {
                panic!("expected event frame");
            };
            let Event::Update { update } = &envelope.event else {
                panic!("expected update event");
            };
            (
                envelope.seq,
                update["observedSeq"].as_u64(),
                update["terminalSeq"].as_u64(),
            )
        };
        let (_, _, running_terminal) = push_snapshot("child-live", "running", 1, "working");
        assert_eq!(running_terminal, None);
        let (ended_seq, obs1, term1) = push_snapshot("child-live", "idle", 2, "done");
        assert_eq!((obs1, term1), (Some(ended_seq), Some(ended_seq)));
        // Later terminal metadata updates retain the original terminalSeq atomically.
        let (meta_seq, obs2, term2) = push_snapshot("child-live", "idle", 3, "done with summary");
        assert_eq!((obs2, term2), (Some(meta_seq), Some(ended_seq)));
        assert_ne!(meta_seq, ended_seq);

        // Warmed legacy terminal mark without any sequence stays unstamped instead of
        // fabricating a new terminalSeq on every metadata update.
        hub.warm_subagent_marks(vec![(
            "parent".into(),
            crate::store::transcript_anchors::AnchoredRow {
                key: format!("{}child-legacy", crate::transcript_anchors::SUBAGENT_PREFIX),
                anchor_seq: 1,
                update: serde_json::json!({
                    "sessionUpdate": crate::transcript_anchors::SUBAGENT_UPDATE,
                    "sessionId": "child-legacy", "incarnation": "worker",
                    "state": "idle", "revision": 1, "summary": "legacy",
                }),
            },
        )]);
        // Even if an incoming legacy metadata update supplies its own
        // terminalSeq, Hub strips it when deriving None.
        hub.push(
            "parent",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": crate::transcript_anchors::SUBAGENT_UPDATE,
                    "sessionId": "child-legacy", "incarnation": "worker",
                    "state": "idle", "revision": 2, "summary": "legacy metadata update",
                    "terminalSeq": 1,
                }),
            },
        );
        let frame = stream.try_recv().unwrap();
        let Outbound::Event { envelope } = frame.outbound() else {
            panic!("expected event frame");
        };
        let Event::Update { update } = &envelope.event else {
            panic!("expected update event");
        };
        assert_eq!(update["observedSeq"].as_u64(), Some(envelope.seq));
        assert_eq!(update.get("terminalSeq"), None);
    }
}

use super::*;

impl Hub {
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
        let subagent_child = live_subagent_child(&event);
        let mut subagent_ending = None;
        if let (Some(child), Event::Update { update }) = (&subagent_child, &event) {
            match self.accept_subagent_snapshot(session_id, child, update) {
                Some(ending) => subagent_ending = Some(ending),
                None => return,
            }
        }
        let mut subagent_anchor = None;
        let (envelope, durable_agent_session_id, attention_changed) = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
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
            let seq = s.next_seq;
            s.next_seq += 1;
            if counts_as_activity(&event) {
                s.last_activity = std::time::Instant::now();
            }
            // I2: clients order subagent snapshots by the Controller's receipt
            // seq, never by the producer's wall-clock revision alone.
            if subagent_child.is_some()
                && let Event::Update { update } = &mut event
                && let Some(object) = update.as_object_mut()
            {
                object.insert("observedSeq".to_owned(), serde_json::Value::from(seq));
                // One terminal occurrence keeps one identity through later
                // metadata updates; a new one starts at this seq.
                match subagent_ending {
                    Some(SubagentEnding::Ended) => {
                        object.insert("terminalSeq".to_owned(), serde_json::Value::from(seq));
                    }
                    Some(SubagentEnding::AlreadyEnded(Some(terminal))) => {
                        object.insert("terminalSeq".to_owned(), serde_json::Value::from(terminal));
                    }
                    _ => {}
                }
            }
            let envelope = Envelope {
                session_id: session_id.to_owned(),
                seq,
                event,
                cmid,
            };
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
            (envelope, durable_agent_session_id, attention_changed)
        };
        if let (Some(child), Event::Update { update }) = (&subagent_child, &envelope.event) {
            self.settle_subagent_mark(session_id, child, update);
        }
        if let (Some(child), Some(anchor), Event::Update { update }) =
            (subagent_child, subagent_anchor, &envelope.event)
        {
            self.record_subagent_observation(session_id, child, anchor, update.clone());
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
        self.fanout(Outbound::Event { envelope });
        if attention_changed {
            self.broadcast_sessions();
        }
    }
}

fn live_subagent_child(event: &Event) -> Option<String> {
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
                .map(str::to_owned)
        }
        _ => None,
    }
}

//! ACP `AgentSink` adapter emitting worker `RuntimeEvent`s.

use super::*;

#[derive(Clone)]
pub struct RemoteSink {
    pub shared: Arc<Shared>,
}

impl RemoteSink {
    fn status_state(status: Status) -> WorkerState {
        match status {
            Status::Starting => WorkerState::Starting,
            Status::Running => WorkerState::Running,
            Status::Busy => WorkerState::Busy,
            Status::Exited | Status::Interrupted => WorkerState::Exited,
            Status::Crashed => WorkerState::Crashed,
        }
    }

    fn emit_event(&self, event: Event, cmid: Option<String>) {
        let runtime = match event {
            Event::Update { mut update } => {
                if update["sessionUpdate"] == "user_message_chunk"
                    && let Some(cid) = cmid.as_deref()
                {
                    let _ = self.shared.take_prompt_command(Some(cid));
                }
                if update["sessionUpdate"] == "compaction_update"
                    && update["status"] == "completed"
                    && let Some(observer) = &self.shared.request_observer
                {
                    observer.compacted(crate::logs::now_ms());
                }
                let mut stored = self.shared.echo_artifacts.lock();
                crate::artifacts::reference_echoed_image(&mut update, stored.make_contiguous());
                drop(stored);
                RuntimeEvent::Update { update, cmid }
            }
            Event::PermissionRequest {
                request_id,
                tool_call,
                options,
            } => RuntimeEvent::PermissionRequest {
                request_id,
                tool_call,
                options,
            },
            Event::PermissionResolved {
                request_id,
                option_id,
            } => RuntimeEvent::PermissionResolved {
                request_id,
                option_id,
            },
            Event::Lifecycle { status, detail } => RuntimeEvent::Status {
                state: Self::status_state(status),
                detail,
            },
            Event::TurnEnd { stop_reason, usage } => {
                let turn_id = self
                    .shared
                    .snapshot
                    .lock()
                    .current_turn_id
                    .clone()
                    .unwrap_or_else(|| "unknown-turn".to_owned());
                let ended = RuntimeEvent::TurnEnded {
                    usage,
                    turn_id: turn_id.clone(),
                    stop_reason,
                };
                // A sandboxed turn completes only once the target proved every
                // process it started gone (design R3-5, R3-6).
                if let Some(execution) = &self.shared.execution
                    && let Some(settled) = execution.finish_turn(&turn_id)
                {
                    let shared = Arc::clone(&self.shared);
                    let publisher_guard = shared.crashed_drain.enter_event_publisher(&shared);
                    execution.spawn_settled(async move {
                        let _guard = publisher_guard;
                        settled.await;
                        shared.emit(ended);
                    });
                    return;
                }
                ended
            }
        };
        self.shared.emit(runtime);
    }
}

impl AgentSink for RemoteSink {
    fn delivery_target_is_current(
        &self,
        session_id: &str,
        target: &crate::send_delivery::DeliveryTarget,
    ) -> bool {
        let snapshot = self.shared.snapshot.lock();
        session_id == self.shared.session_id
            && snapshot.worker_epoch == target.worker_epoch
            && snapshot.delivery_turn_id() == Some(target.turn_id.as_str())
            && !snapshot.drain_requested
    }
    fn wrap_up_hint_slot(
        &self,
        session_id: &str,
    ) -> Option<Arc<crate::acp_wrap_up::WrapUpHintSlot>> {
        (session_id == self.shared.session_id).then(|| Arc::clone(&self.shared.wrap_up_hint))
    }
    fn raw_wrap_up_launch_eligible(&self, session_id: &str) -> bool {
        session_id == self.shared.session_id && self.shared.raw_wrap_up_launch_eligible()
    }
    fn wrap_up_target_matches(
        &self,
        session_id: &str,
        target: &crate::send_delivery::DeliveryTarget,
    ) -> bool {
        let snapshot = self.shared.snapshot.lock();
        session_id == self.shared.session_id
            && snapshot.wrap_up_steering
            && snapshot.worker_epoch == target.worker_epoch
            && snapshot.current_turn_id.as_deref() == Some(target.turn_id.as_str())
            && !snapshot.drain_requested
            && !super::snapshot_is_managed_read_only(&snapshot)
    }
    fn set_active_turn_nonce(&self, session_id: &str, turn: Option<(String, String)>) {
        if session_id == self.shared.session_id
            && let Some(execution) = &self.shared.execution
        {
            execution.set_active_turn_nonce(turn);
        }
    }
    fn record_proxy_uncertain_effect(&self, session_id: &str, method: &str) {
        if session_id != self.shared.session_id {
            return;
        }
        if let Some(ledger) = self
            .shared
            .execution
            .as_ref()
            .and_then(|execution| execution.ledger())
        {
            ledger.mark_uncertain(method);
        } else {
            let prev = self
                .shared
                .snapshot
                .lock()
                .uncertain_effect
                .as_ref()
                .map_or(0, |effect| effect.sequence);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
                });
            self.shared
                .mark_uncertain_effect(method, prev.saturating_add(1).max(now));
        }
    }
    fn record_unconfirmed_exit_if_sandboxed(&self, session_id: &str) {
        if session_id != self.shared.session_id {
            return;
        }
        if let Some(ledger) = self
            .shared
            .execution
            .as_ref()
            .and_then(|execution| execution.ledger())
        {
            ledger.mark_uncertain("process/start");
        }
    }
    fn session_is_managed_read_only(&self, session_id: &str) -> bool {
        session_id == self.shared.session_id && self.shared.managed_read_only()
    }
    fn managed_config_option_allowed(&self, session_id: &str, config_id: &str) -> bool {
        session_id == self.shared.session_id && self.shared.preset_options.contains(config_id)
    }
    fn managed_prompt_meta(
        &self,
        session_id: &str,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
        if !self.session_is_managed_read_only(session_id) {
            return Ok(None);
        }
        if let Some(remote) = &self.shared.managed_round {
            return remote
                .marker
                .lock()
                .as_ref()
                .map(|round| Some(round.prompt_meta()))
                .ok_or_else(|| "managed call round is unavailable".to_owned());
        }
        crate::managed_calls::round::read_round(&self.shared.workspace_path)
            .map(|round| Some(round.prompt_meta()))
            .ok_or_else(|| "managed call round is unavailable".to_owned())
    }
    fn managed_round_fence(
        &self,
        session_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<Option<Result<(), String>>>> {
        (session_id == self.shared.session_id)
            .then(|| {
                self.shared
                    .managed_round
                    .as_ref()
                    .map(|round| round.ready.clone())
            })
            .flatten()
    }
    fn prompt_started(&self, _session_id: &str, cmid: Option<&str>) {
        if let Some(observer) = &self.shared.request_observer {
            observer.prompt_started(crate::logs::now_ms());
        }
        self.shared.telemetry.lock().started(cmid);
    }

    fn prompt_completed(&self, _session_id: &str, cmid: Option<&str>, outcome: &str) {
        if let Some(cmid) = cmid {
            crate::logs::wait_cache::record_notice_receipt(&self.shared.session_id, cmid, outcome);
        }
        self.shared.telemetry.lock().completed(cmid, outcome);
    }

    fn record_prompt_observation(
        &self,
        _session_id: &str,
        stop_reason: &str,
        usage: Option<crate::agent_model::TurnUsage>,
        meta: Option<&serde_json::Map<String, serde_json::Value>>,
    ) {
        if let Some(observer) = &self.shared.request_observer {
            observer.record_prompt_observation(stop_reason, usage, meta);
        }
    }

    fn set_status(&self, _session_id: &str, status: Status, detail: Option<String>) {
        let mut state = Self::status_state(status);
        if state == WorkerState::Running && self.shared.snapshot.lock().drain_requested {
            state = WorkerState::Draining;
        }
        self.shared.emit(RuntimeEvent::Status { state, detail });
    }

    fn push(&self, _session_id: &str, event: Event) {
        self.emit_event(event, None);
    }

    fn push_tagged(&self, _session_id: &str, event: Event, cmid: Option<String>) {
        self.emit_event(event, cmid);
    }

    fn set_config_options(&self, _session_id: &str, mut options: serde_json::Value) {
        if self.shared.managed_read_only() {
            if let Some(options) = options.as_array_mut() {
                options.retain(|option| {
                    option["id"]
                        .as_str()
                        .is_some_and(|id| self.shared.preset_options.contains(id))
                });
            } else {
                options = serde_json::json!([]);
            }
        }
        if let Some(observer) = &self.shared.request_observer {
            observer.observe_config_options(&options);
        }
        self.shared.emit(RuntimeEvent::ConfigOptions { options });
    }

    fn set_agent_session_id(&self, _session_id: &str, agent_session_id: String) {
        if let Some(observer) = &self.shared.request_observer {
            observer.native_id(&agent_session_id);
        }
        self.shared.emit(RuntimeEvent::AgentSessionId {
            agent_session_id,
            materialized: false,
        });
    }

    fn set_forked_agent_session_id(&self, _session_id: &str, agent_session_id: String) {
        if let Some(observer) = &self.shared.request_observer {
            observer.native_id(&agent_session_id);
        }
        self.shared.emit(RuntimeEvent::AgentSessionId {
            agent_session_id,
            materialized: true,
        });
    }

    /// Never waits: a busy or full outbox drops the update and counts it for
    /// that child, whose next emitted update reports the gap.
    fn observe_subagent(
        &self,
        _session_id: &str,
        child_session_id: &str,
        update: serde_json::Value,
    ) {
        let shared = &self.shared;
        if !shared.observation.enabled.load(Ordering::Relaxed) {
            return;
        }
        let update = crate::runtime_wire::bound_observed_update(update, OBSERVE_MAX_BYTES);
        let mut dropped = shared.observation.dropped.lock();
        let outbox = shared
            .outbox
            .try_lock()
            .filter(|outbox| outbox.len() < OBSERVE_OUTBOX_LIMIT);
        let Some(mut outbox) = outbox else {
            if dropped.len() < OBSERVE_DROP_CHILDREN || dropped.contains_key(child_session_id) {
                *dropped.entry(child_session_id.to_owned()).or_default() += 1;
            }
            return;
        };
        let mut observed = serde_json::json!({
            "sessionUpdate": crate::runtime_wire::SUBAGENT_EVENT_UPDATE,
            "childSessionId": child_session_id,
            "update": update,
        });
        if let Some(count) = dropped.remove(child_session_id) {
            observed["dropped"] = count.into();
        }
        drop(dropped);
        shared.emit_locked(
            &mut outbox,
            RuntimeEvent::Update {
                update: observed,
                cmid: None,
            },
        );
    }

    fn set_session_usage(&self, _session_id: &str, usage: crate::agent_model::SessionUsage) {
        let configured_size = self
            .shared
            .snapshot
            .lock()
            .launch
            .as_ref()
            .and_then(|launch| launch.context_window);
        self.shared.emit(RuntimeEvent::ContextUsage {
            used: usage.used,
            size: configured_size.map_or(usage.size, |size| size.min(usage.size)),
            raw: usage.raw,
            observed_at_ms: usage.observed_at_ms,
        });
    }

    fn set_background_tasks(&self, _session_id: &str, count: u32) {
        self.shared.emit(RuntimeEvent::Update {
            update: crate::runtime_wire::background_tasks_update(count),
            cmid: None,
        });
    }

    fn set_native_cycle(&self, _session_id: &str, cycle_id: Option<&str>) {
        self.shared.emit(RuntimeEvent::Update {
            update: crate::runtime_wire::native_cycle_update(cycle_id),
            cmid: None,
        });
    }

    fn set_account_unit(
        &self,
        _session_id: &str,
        unit: Option<crate::runtime_wire::AccountUnitHandle>,
    ) {
        self.shared.snapshot.lock().account_unit = unit;
        let _ = self.shared.notify.send(());
    }

    fn schedule_wakeup(&self, _session_id: &str, delay_seconds: i64, prompt: String) {
        self.shared.emit(RuntimeEvent::ScheduleWakeup {
            delay_seconds,
            prompt,
        });
    }

    fn session_is_system(&self, _session_id: &str) -> bool {
        self.shared.system
    }

    fn broadcast_error(&self, _session_id: Option<String>, message: String) {
        self.shared.emit(RuntimeEvent::Error { message });
    }

    fn requeue_prompt(
        &self,
        _session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        let content = if content.is_empty() && !text.is_empty() {
            vec![serde_json::json!({"type": "text", "text": text})]
        } else {
            content
        };
        let command_id = self
            .shared
            .take_prompt_command(cmid.as_deref())
            .unwrap_or_else(|| "unknown-command".to_owned());
        self.shared.emit(RuntimeEvent::UndeliveredPrompt {
            command_id,
            cmid,
            content,
            error: "agent failed before accepting prompt".to_owned(),
        });
    }
}

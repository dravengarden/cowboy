//! Antigravity wrap-up steering Controller-side state: raw capability cache,
//! protocol/config sanitization, per-turn hint deduplication, and direct
//! bounded WebSocket hint dispatch bypassing the shared runtime `UnixStream` FIFO.

use std::collections::HashMap;
#[cfg(test)]
use std::sync::Arc;
use std::sync::atomic::Ordering;

use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::{Shared, WorkerSnapshot};
use crate::core::Hub;
use crate::machine_protocol::{MachineFrame, WRAP_UP_STEERING_PROTOCOL_VERSION};
use crate::runtime_wire::{
    CoreCommand, DeliveryTarget, Frame, WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
};
use crate::wrap_up::{
    clamp_delivery_capabilities_for_provider, effective_wrap_up_steering,
    wrap_up_hint_within_size_limit,
};

#[derive(Default)]
pub(super) struct WrapUpState {
    /// Direct per-connection bounded WS hint channel (cap 32, dropped on disconnect).
    hint_ws_tx: Mutex<Option<mpsc::Sender<String>>>,
    /// Raw broker-reported `wrap_up_steering` keyed by `(session_id, worker_epoch)`
    /// so reconnects, epoch switches, and live config reloads cannot cross-contaminate epochs.
    raw_wrap_up: Mutex<HashMap<(String, String), bool>>,
    /// Last hinted `(worker_epoch, turn_id)` per session so at most ONE hint
    /// frame is emitted per active turn on the Controller->Machine hop.
    last_hinted: Mutex<HashMap<String, DeliveryTarget>>,
    /// Live Service configuration handle (`None` in unit tests defaults to `true`).
    service_config: Mutex<Option<crate::config::Handle>>,
}

impl WrapUpState {
    pub(super) fn attach_hint_tx(&self, tx: mpsc::Sender<String>) {
        *self.hint_ws_tx.lock() = Some(tx);
    }

    pub(super) fn attach_service_config(&self, config: crate::config::Handle) {
        *self.service_config.lock() = Some(config);
    }

    pub(super) fn clear_hint_tx(&self) {
        *self.hint_ws_tx.lock() = None;
    }

    pub(super) fn config_enabled(&self) -> bool {
        self.service_config.lock().as_ref().is_none_or(|cfg| {
            cfg.get(&crate::config::schema::PROVIDER_ANTIGRAVITY_WRAP_UP_STEERING)
        })
    }

    pub(super) fn sanitize_snapshot(
        &self,
        worker: &mut WorkerSnapshot,
        raw: bool,
        machine_protocol: u16,
        runtime_protocol: u16,
        hub: &Hub,
    ) {
        let provider = resolved_provider(worker, hub);
        clamp_delivery_capabilities_for_provider(
            provider.as_deref(),
            &mut worker.supports_current_turn,
            &mut worker.delivery_controls.next_checkpoint,
        );
        worker.wrap_up_steering = effective_wrap_up_steering(
            provider.as_deref(),
            raw,
            Some(machine_protocol),
            runtime_protocol,
            self.config_enabled(),
        );
    }

    /// Record raw capability while caller holds `shared.workers.lock()` so a
    /// concurrent `recompute_all` can never prune an incoming replacement epoch.
    pub(super) fn record_raw_locked(&self, session_id: &str, worker_epoch: &str, raw: bool) {
        self.raw_wrap_up
            .lock()
            .insert((session_id.to_owned(), worker_epoch.to_owned()), raw);
    }

    pub(super) fn record_raw_event(
        &self,
        session_id: &str,
        raw: bool,
        worker: &mut WorkerSnapshot,
        machine_protocol: u16,
        runtime_protocol: u16,
        hub: &Hub,
    ) -> bool {
        self.raw_wrap_up
            .lock()
            .insert((session_id.to_owned(), worker.worker_epoch.clone()), raw);
        let provider = resolved_provider(worker, hub)
            .or_else(|| hub.session_info(session_id).map(|info| info.meta.provider));
        let effective = effective_wrap_up_steering(
            provider.as_deref(),
            raw,
            Some(machine_protocol),
            runtime_protocol,
            self.config_enabled(),
        );
        clamp_delivery_capabilities_for_provider(
            provider.as_deref(),
            &mut worker.supports_current_turn,
            &mut worker.delivery_controls.next_checkpoint,
        );
        worker.wrap_up_steering = effective;
        effective
    }

    pub(super) fn recompute_all(&self, shared: &Shared) {
        // Hold `projection_fence` across the entire recompute + Hub publication
        // so `disconnect()` serializes against it and a superseded runtime can
        // never publish stale capabilities after a replacement runtime connects.
        let fenced = shared.projection_fence.lock();
        if *fenced
            || !shared.connected.load(Ordering::Acquire)
            || shared.shutdown.load(Ordering::Acquire)
        {
            return;
        }
        let _projection = shared.hub.runtime_projection_guard();
        let runtime_protocol = shared.protocol.load(Ordering::Acquire);
        let enabled = self.config_enabled();
        let updated: Vec<WorkerSnapshot> = {
            let mut workers = shared.workers.lock();
            let mut raw_map = self.raw_wrap_up.lock();
            raw_map.retain(|(sid, epoch), _| {
                workers
                    .get(sid)
                    .is_some_and(|worker| worker.worker_epoch == *epoch)
            });
            workers
                .iter_mut()
                .map(|(session_id, worker)| {
                    let raw = raw_map
                        .get(&(session_id.clone(), worker.worker_epoch.clone()))
                        .copied()
                        .unwrap_or(false);
                    let provider = resolved_provider(worker, &shared.hub);
                    clamp_delivery_capabilities_for_provider(
                        provider.as_deref(),
                        &mut worker.supports_current_turn,
                        &mut worker.delivery_controls.next_checkpoint,
                    );
                    let effective = effective_wrap_up_steering(
                        provider.as_deref(),
                        raw,
                        Some(shared.machine_protocol),
                        runtime_protocol,
                        enabled,
                    );
                    worker.wrap_up_steering = effective;
                    worker.clone()
                })
                .collect()
        };
        for worker in &updated {
            let still_current = shared
                .workers
                .lock()
                .get(&worker.session_id)
                .is_some_and(|current| current.worker_epoch == worker.worker_epoch);
            if still_current
                && !super::foreign_session(shared, &worker.session_id)
                && super::snapshot::admits_worker(shared, worker)
            {
                shared
                    .hub
                    .set_current_turn_support(&worker.session_id, worker.supports_current_turn);
                shared
                    .hub
                    .set_wrap_up_steering(&worker.session_id, worker.wrap_up_steering);
            }
        }
    }

    pub(super) fn send_hint(
        &self,
        shared: &Shared,
        session_id: &str,
        target: &DeliveryTarget,
    ) -> bool {
        let runtime_protocol = shared.protocol.load(Ordering::Acquire);
        if !shared.connected.load(Ordering::Acquire)
            || shared.shutdown.load(Ordering::Acquire)
            || shared.machine_protocol < WRAP_UP_STEERING_PROTOCOL_VERSION
            || runtime_protocol < WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION
            || !self.config_enabled()
        {
            return false;
        }
        let eligible = {
            let workers = shared.workers.lock();
            workers.get(session_id).is_some_and(|worker| {
                worker.wrap_up_steering
                    && worker.worker_epoch == target.worker_epoch
                    && worker.current_turn_id.as_deref() == Some(target.turn_id.as_str())
                    && !worker.drain_requested
            })
        };
        if !eligible {
            return false;
        }
        let frame = Frame::CoreCommand {
            command: CoreCommand::WrapUpHint {
                session_id: session_id.to_owned(),
                target: target.clone(),
            },
        };
        if !wrap_up_hint_within_size_limit(&frame) {
            return false;
        }
        let Ok(text) = serde_json::to_string(&MachineFrame::Runtime { frame }) else {
            return false;
        };
        let Some(tx) = self.hint_ws_tx.lock().clone() else {
            return false;
        };
        let mut last_hinted = self.last_hinted.lock();
        if last_hinted.get(session_id) == Some(target) {
            return false;
        }
        match tx.try_send(text) {
            Ok(()) => {
                last_hinted.insert(session_id.to_owned(), target.clone());
                true
            }
            Err(_) => false,
        }
    }
}

fn resolved_provider(worker: &WorkerSnapshot, hub: &Hub) -> Option<String> {
    worker
        .launch
        .as_ref()
        .map(|launch| launch.provider.clone())
        .or_else(|| {
            hub.session_info(&worker.session_id)
                .map(|info| info.meta.provider)
        })
}

impl super::RemoteRuntime {
    pub fn attach_wrap_up_hint_tx(&self, tx: mpsc::Sender<String>) {
        self.shared.wrap_up.attach_hint_tx(tx);
    }

    pub fn attach_service_config(&self, config: crate::config::Handle) {
        self.shared.wrap_up.attach_service_config(config);
        self.refresh_wrap_up_steering();
    }

    pub fn refresh_wrap_up_steering(&self) {
        self.shared.wrap_up.recompute_all(&self.shared);
    }

    pub fn send_wrap_up_hint(
        &self,
        session_id: &str,
        target: &crate::send_delivery::DeliveryTarget,
    ) -> bool {
        self.shared
            .wrap_up
            .send_hint(&self.shared, session_id, target)
    }
}

impl super::Shared {
    pub(super) fn install_worker_with_raw_wrap_up(&self, worker: WorkerSnapshot, raw_wrap: bool) {
        let mut workers = self.workers.lock();
        self.wrap_up
            .record_raw_locked(&worker.session_id, &worker.worker_epoch, raw_wrap);
        workers.insert(worker.session_id.clone(), worker);
    }

    pub(super) fn apply_wrap_up_capability_update(
        &self,
        session_id: &str,
        update: &serde_json::Value,
        worker: &mut WorkerSnapshot,
    ) {
        let raw_wrap = crate::runtime_wire::wrap_up_steering_support(update).unwrap_or(false);
        let effective = self.wrap_up.record_raw_event(
            session_id,
            raw_wrap,
            worker,
            self.machine_protocol,
            self.protocol.load(Ordering::Acquire),
            &self.hub,
        );
        if !super::foreign_session(self, session_id)
            && self.hub.accepts_runtime_projection(session_id)
            && self.hub.classify_runtime_epoch(
                session_id,
                &worker.worker_epoch,
                worker.agent_session_id.as_deref(),
            ) == crate::core::RuntimeEpoch::Current
        {
            self.hub
                .set_current_turn_support(session_id, worker.supports_current_turn);
            self.hub.set_wrap_up_steering(session_id, effective);
        }
    }
}

pub(super) fn apply_current_turn_capability_event(
    hub: &Hub,
    session_id: &str,
    update: &serde_json::Value,
) {
    let mut supported = crate::runtime_wire::current_turn_support(update).unwrap_or(false);
    let mut next_cp = false;
    let provider = hub.session_info(session_id).map(|info| info.meta.provider);
    clamp_delivery_capabilities_for_provider(provider.as_deref(), &mut supported, &mut next_cp);
    hub.set_current_turn_support(session_id, supported);
}

#[cfg(test)]
pub(super) async fn merge_worker_snapshots(
    shared: &Arc<super::Shared>,
    workers: Vec<WorkerSnapshot>,
) {
    let proto = shared.protocol.load(Ordering::Acquire);
    super::merge_worker_snapshots_with_protocol(
        shared,
        workers,
        if proto == 0 {
            crate::runtime_wire::PROTOCOL_VERSION
        } else {
            proto
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SessionOrigin;
    use crate::runtime_wire::{DeliveryControls, WorkerState};

    fn busy_snapshot(session_id: &str, epoch: &str, wrap_up: bool) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: session_id.into(),
            worker_epoch: epoch.into(),
            generation: "g1".into(),
            executable: None,
            launch: None,
            state: WorkerState::Busy,
            agent_session_id: None,
            native_thread_materialized: None,
            current_turn_id: Some("turn-1".into()),
            supports_current_turn: false,
            delivery_controls: DeliveryControls::default(),
            wrap_up_steering: wrap_up,
            supports_subagent_cancel: false,
            last_runtime_seq: 1,
            pending_permissions: vec![],
            config_options: None,
            context_used: None,
            context_size: None,
            pending_prompt_count: 0,
            drain_requested: false,
            exit_detail: None,
            background_tasks: None,
            native_cycle_id: None,
            incarnation: None,
            account_unit: None,
            uncertain_effect: None,
        }
    }

    #[tokio::test]
    async fn duplicate_hint_returns_false_and_sends_single_frame() {
        let hub = Hub::new();
        hub.create_local_session(
            "s1".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s1".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub, Vec::new());
        runtime
            .shared
            .protocol
            .store(WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION, Ordering::Release);
        runtime.shared.connected.store(true, Ordering::Release);
        let (tx, mut rx) = mpsc::channel(8);
        runtime.attach_wrap_up_hint_tx(tx);
        runtime
            .shared
            .workers
            .lock()
            .insert("s1".into(), busy_snapshot("s1", "e1", true));
        let target = DeliveryTarget {
            worker_epoch: "e1".into(),
            turn_id: "turn-1".into(),
        };
        assert!(runtime.send_wrap_up_hint("s1", &target));
        assert!(
            !runtime.send_wrap_up_hint("s1", &target),
            "suppressed duplicate must return false"
        );
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn superseded_or_disconnected_runtime_refresh_never_overwrites_hub() {
        let hub = Hub::new();
        hub.create_session(crate::core::SessionRegistration {
            id: "s1".into(),
            provider: "antigravity".into(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_version: "0.2.35".into(),
            provider_generation_digest: "sha256:abc".into(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "m-new".into(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tmp".into(),
            title: "s1".into(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        hub.set_wrap_up_steering("s1", true);

        // Superseded runtime (`shutdown = true`) must never overwrite Hub.
        let old_runtime = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        old_runtime
            .shared
            .protocol
            .store(WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION, Ordering::Release);
        old_runtime.shared.connected.store(true, Ordering::Release);
        old_runtime.shared.shutdown.store(true, Ordering::Release);
        let mut snap = busy_snapshot("s1", "e-old", false);
        old_runtime.shared.wrap_up.sanitize_snapshot(
            &mut snap,
            false,
            WRAP_UP_STEERING_PROTOCOL_VERSION,
            WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION,
            &hub,
        );
        {
            let mut workers = old_runtime.shared.workers.lock();
            old_runtime
                .shared
                .wrap_up
                .record_raw_locked("s1", "e-old", false);
            workers.insert("s1".into(), snap);
        }
        old_runtime.refresh_wrap_up_steering();
        assert!(hub.session_info("s1").unwrap().meta.wrap_up_steering);

        // Runtime whose machine_id ("m-old") does NOT own s1 ("m-new"):
        // refresh must not overwrite Hub's live wrap_up_steering.
        let auth_dir = tempfile::tempdir().unwrap();
        let foreign_runtime = super::super::RemoteRuntime::connecting(
            hub.clone(),
            "m-old".into(),
            std::path::PathBuf::from("/tmp/unused-foreign.sock"),
            "g1".into(),
            std::sync::Arc::new(
                crate::provider_service::ProviderAuthService::open(auth_dir.path()).unwrap(),
            ),
            WRAP_UP_STEERING_PROTOCOL_VERSION,
            None,
        );
        foreign_runtime
            .shared
            .protocol
            .store(WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION, Ordering::Release);
        foreign_runtime
            .shared
            .connected
            .store(true, Ordering::Release);
        foreign_runtime
            .shared
            .workers
            .lock()
            .insert("s1".into(), busy_snapshot("s1", "e-foreign", false));
        foreign_runtime.refresh_wrap_up_steering();
        assert!(hub.session_info("s1").unwrap().meta.wrap_up_steering);

        // Disconnected runtime for an owned session must also never overwrite Hub.
        hub.create_local_session(
            "s2".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s2".into(),
            SessionOrigin::Web,
            false,
        );
        hub.set_wrap_up_steering("s2", true);
        let disconnected = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        disconnected
            .shared
            .protocol
            .store(WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION, Ordering::Release);
        disconnected
            .shared
            .connected
            .store(false, Ordering::Release);
        disconnected
            .shared
            .workers
            .lock()
            .insert("s2".into(), busy_snapshot("s2", "e-new", false));
        disconnected.refresh_wrap_up_steering();
        assert!(hub.session_info("s2").unwrap().meta.wrap_up_steering);

        // Projection-fenced runtime (as set by disconnect()) must never publish.
        let fenced_rt = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        fenced_rt
            .shared
            .protocol
            .store(WRAP_UP_STEERING_RUNTIME_PROTOCOL_VERSION, Ordering::Release);
        fenced_rt.shared.connected.store(true, Ordering::Release);
        *fenced_rt.shared.projection_fence.lock() = true;
        fenced_rt
            .shared
            .workers
            .lock()
            .insert("s2".into(), busy_snapshot("s2", "e-fenced", false));
        fenced_rt.refresh_wrap_up_steering();
        assert!(hub.session_info("s2").unwrap().meta.wrap_up_steering);
    }

    #[tokio::test]
    async fn replacement_worker_with_unacknowledged_uncertain_effect_requeues_admitted_prompt() {
        let hub = Hub::new();
        hub.create_local_session(
            "s-unc".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-unc".into(),
            SessionOrigin::Web,
            false,
        );
        hub.set_setting(
            format!(
                "{}s-unc",
                crate::core::settings_keys::SESSION_UNCERTAIN_EFFECT_ACK_PREFIX
            ),
            serde_json::json!({"observed": 1, "acknowledged": 1, "method": "process/terminate"}),
        );
        let rt = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        rt.shared.config_startups.lock().insert("s-unc".into());
        rt.shared.pending.lock().insert(
            "prompt-1".into(),
            CoreCommand::Prompt {
                session_id: "s-unc".into(),
                command_id: "prompt-1".into(),
                turn_id: "turn-new".into(),
                content: vec![serde_json::json!({"type": "text", "text": "next step"})],
                cmid: Some("cmid-1".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: None,
                acknowledged_uncertain_sequence: None,
            },
        );
        let mut replacement = busy_snapshot("s-unc", "epoch-2", true);
        replacement.state = WorkerState::Running;
        replacement.current_turn_id = None;
        replacement.config_options = Some(serde_json::json!([]));
        replacement.uncertain_effect = Some(crate::runtime_wire::UncertainEffect {
            method: "process/terminate".into(),
            sequence: 2,
        });
        super::super::config_sync::sync_config_for_worker(&rt.shared, &replacement);
        assert!(
            rt.shared.pending.lock().is_empty(),
            "admitted prompt must be withdrawn from pending instead of woken"
        );
        let (queue, _) = hub.queue_and_drafts("s-unc", |_| true).unwrap();
        assert_eq!(queue.len(), 1, "withdrawn prompt must return to Hub queue");
    }

    #[tokio::test]
    async fn attempted_prompt_is_not_blindly_requeued_on_replacement_uncertainty() {
        let hub = Hub::new();
        hub.create_local_session(
            "s-att".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-att".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        runtime.shared.pending.lock().insert(
            "cmd-attempted".into(),
            CoreCommand::Prompt {
                session_id: "s-att".into(),
                command_id: "cmd-attempted".into(),
                turn_id: "turn-1".into(),
                content: vec![serde_json::json!({"type": "text", "text": "already sent"})],
                cmid: Some("cmid-att".into()),
                trace: None,
                echo_artifacts: false,
                observe_subagents: false,
                expected_worker_epoch: Some("e-old".into()),
                acknowledged_uncertain_sequence: Some(0),
            },
        );
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-attempted".into());

        let mut replacement = busy_snapshot("s-att", "e-new", true);
        replacement.state = WorkerState::Running;
        replacement.current_turn_id = None;
        replacement.config_options = Some(serde_json::json!([]));
        replacement.uncertain_effect = Some(crate::runtime_wire::UncertainEffect {
            method: "process/terminate".into(),
            sequence: 99,
        });
        super::super::config_sync::sync_config_for_worker(&runtime.shared, &replacement);

        // Already-attempted send MUST stay out of Hub queue until explicit CommandAck refusal.
        assert_eq!(hub.session_info("s-att").unwrap().queue_count, 0);
        assert!(runtime.shared.pending.lock().contains_key("cmd-attempted"));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn prompt_binding_persists_in_pending_and_blocks_cross_epoch_rebind() {
        let hub = Hub::new();
        hub.create_local_session(
            "s-bind".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-bind".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub, Vec::new());
        let mut w1 = busy_snapshot("s-bind", "epoch-1", true);
        w1.state = WorkerState::Running;
        w1.current_turn_id = None;
        runtime.shared.workers.lock().insert("s-bind".into(), w1);
        let cmd = CoreCommand::Prompt {
            session_id: "s-bind".into(),
            command_id: "cmd-bind".into(),
            turn_id: "turn-1".into(),
            content: vec![serde_json::json!({"type": "text", "text": "once"})],
            cmid: Some("cmid-bind".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: None,
            acknowledged_uncertain_sequence: None,
        };
        let version = runtime
            .shared
            .pending
            .lock()
            .insert("cmd-bind".into(), cmd.clone());
        let mut outbound = cmd;
        assert!(matches!(
            super::super::config_sync::prepare_prompt_for_send(
                &runtime.shared,
                "cmd-bind",
                version,
                &mut outbound,
            ),
            super::super::config_sync::PromptPrep::Ready
        ));
        // Binding MUST be persisted into `shared.pending`, not only on the clone.
        let stored = runtime
            .shared
            .pending
            .lock()
            .get("cmd-bind")
            .unwrap()
            .clone();
        assert!(matches!(
            stored,
            CoreCommand::Prompt {
                expected_worker_epoch: Some(ref ep),
                acknowledged_uncertain_sequence: Some(0),
                ..
            } if ep == "epoch-1"
        ));

        // Simulate lost ACK + worker change to epoch-2 after send attempt.
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-bind".into());
        let mut w2 = busy_snapshot("s-bind", "epoch-2", true);
        w2.state = WorkerState::Running;
        w2.current_turn_id = None;
        runtime.shared.workers.lock().insert("s-bind".into(), w2);
        let mut retry = runtime
            .shared
            .pending
            .lock()
            .get("cmd-bind")
            .unwrap()
            .clone();
        match super::super::config_sync::prepare_prompt_for_send(
            &runtime.shared,
            "cmd-bind",
            version,
            &mut retry,
        ) {
            super::super::config_sync::PromptPrep::ParkAmbiguous {
                session_id,
                command,
                replacement_worker,
            } => {
                assert!(
                    replacement_worker.is_some(),
                    "connected replacement epoch superseded bound epoch"
                );
                super::super::notice_recovery::park_and_retire_ambiguous_pending(
                    &runtime.shared,
                    &session_id,
                    "cmd-bind",
                    version,
                    *command,
                    "prompt delivery across worker change is unresolved; retained as draft",
                    replacement_worker.as_deref(),
                    false,
                );
            }
            _ => panic!("cross-epoch retry must stage ambiguous park"),
        }
        assert!(!runtime.shared.pending.lock().contains_key("cmd-bind"));
        let info = runtime.shared.hub.session_info("s-bind").unwrap();
        assert_eq!(info.queue_count, 0);
        assert_eq!(info.drafts_count, 1);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn unbound_attempted_prompt_and_delivery_ambiguous_ack_park_draft_without_requeue() {
        let hub = Hub::new();
        hub.create_local_session(
            "s-unbound".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-unbound".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub, Vec::new());

        // First send occurred while no worker snapshot existed -> expected_worker_epoch = None.
        let unbound_cmd = CoreCommand::Prompt {
            session_id: "s-unbound".into(),
            command_id: "cmd-unbound".into(),
            turn_id: "turn-u".into(),
            content: vec![serde_json::json!({"type": "text", "text": "do not replay"})],
            cmid: Some("cmid-u".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: None,
            acknowledged_uncertain_sequence: Some(0),
        };
        let version = runtime
            .shared
            .pending
            .lock()
            .insert("cmd-unbound".into(), unbound_cmd.clone());
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-unbound".into());
        let mut w_new = busy_snapshot("s-unbound", "epoch-replacement", true);
        w_new.state = WorkerState::Running;
        w_new.current_turn_id = None;
        runtime
            .shared
            .workers
            .lock()
            .insert("s-unbound".into(), w_new);

        let mut retry = unbound_cmd;
        match super::super::config_sync::prepare_prompt_for_send(
            &runtime.shared,
            "cmd-unbound",
            version,
            &mut retry,
        ) {
            super::super::config_sync::PromptPrep::ParkAmbiguous {
                session_id,
                command,
                replacement_worker,
            } => {
                assert!(
                    replacement_worker.is_none(),
                    "unbound prompt has no replacement epoch"
                );
                super::super::notice_recovery::park_and_retire_ambiguous_pending(
                    &runtime.shared,
                    &session_id,
                    "cmd-unbound",
                    version,
                    *command,
                    "prompt delivery across worker change is unresolved; retained as draft",
                    replacement_worker.as_deref(),
                    false,
                );
            }
            _ => panic!("unbound already-attempted prompt must stage ambiguous park"),
        }
        assert!(!runtime.shared.pending.lock().contains_key("cmd-unbound"));
        let info = runtime.shared.hub.session_info("s-unbound").unwrap();
        assert_eq!(info.queue_count, 0);
        assert_eq!(info.drafts_count, 1);

        // `delivery_ambiguous:` CommandAck parks a draft without requeuing.
        let amb_cmd = CoreCommand::Prompt {
            session_id: "s-unbound".into(),
            command_id: "cmd-amb".into(),
            turn_id: "turn-a".into(),
            content: vec![serde_json::json!({"type": "text", "text": "ambiguous prompt"})],
            cmid: Some("cmid-a".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("epoch-old".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        super::super::notice_recovery::handle_rejected_ack(
            &runtime.shared,
            "s-unbound",
            "cmd-amb",
            Some(amb_cmd),
            false,
            Some("delivery_ambiguous: original worker changed after prompt handoff".into()),
        );
        let info2 = runtime.shared.hub.session_info("s-unbound").unwrap();
        assert_eq!(info2.queue_count, 0);
        assert_eq!(info2.drafts_count, 2);

        // `unhanded:` CommandAck safely requeues back into Hub queue.
        let unhanded_cmd = CoreCommand::Prompt {
            session_id: "s-unbound".into(),
            command_id: "cmd-unhanded".into(),
            turn_id: "turn-uh".into(),
            content: vec![serde_json::json!({"type": "text", "text": "safe requeue"})],
            cmid: Some("cmid-uh".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("epoch-old".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        super::super::notice_recovery::handle_rejected_ack(
            &runtime.shared,
            "s-unbound",
            "cmd-unhanded",
            Some(unhanded_cmd),
            false,
            Some("unhanded: worker epoch changed before prompt admission".into()),
        );
        let info3 = runtime.shared.hub.session_info("s-unbound").unwrap();
        assert_eq!(info3.queue_count, 1);
        assert_eq!(info3.drafts_count, 2);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn disconnect_preserves_pending_prompts_and_retransmitted_refusal_parks_draft() {
        let hub = Hub::new();
        hub.create_local_session(
            "s-disc".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-disc".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        let unsent = CoreCommand::Prompt {
            session_id: "s-disc".into(),
            command_id: "cmd-unsent".into(),
            turn_id: "t-u".into(),
            content: vec![serde_json::json!({"type": "text", "text": "unsent payload"})],
            cmid: Some("cmid-unsent".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: None,
            acknowledged_uncertain_sequence: Some(0),
        };
        let attempted = CoreCommand::Prompt {
            session_id: "s-disc".into(),
            command_id: "cmd-att".into(),
            turn_id: "t-a".into(),
            content: vec![serde_json::json!({"type": "text", "text": "attempted payload"})],
            cmid: Some("cmid-att".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("e1".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        runtime
            .shared
            .pending
            .lock()
            .insert("cmd-unsent".into(), unsent);
        runtime
            .shared
            .pending
            .lock()
            .insert("cmd-att".into(), attempted.clone());
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-att".into());

        // Retransmitted generic refusal MUST park as draft (not requeue) and
        // retire via take_acked_command without reconciling stale idle guard.
        runtime
            .shared
            .retransmitted_sends
            .lock()
            .insert("cmd-att".into());
        let (_, parked) = super::super::notice_recovery::take_acked_command(
            &runtime.shared,
            "s-disc",
            "cmd-att",
            false,
            Some("Machine is busy; retry shortly"),
        )
        .expect("matching session CommandAck");
        assert!(parked);
        assert_eq!(hub.session_info("s-disc").unwrap().drafts_count, 1);

        // Optimistic revival with unattempted prompt rolls back Starting -> Exited on disconnect.
        hub.set_status("s-disc", super::super::Status::Starting, None);
        runtime
            .shared
            .optimistic_revivals
            .lock()
            .insert("s-disc".into());

        // Cached terminal Exited worker must NOT block optimistic revival rollback (Finding 15).
        let mut exited_worker = busy_snapshot("s-disc", "epoch-exited", true);
        exited_worker.state = WorkerState::Exited;
        exited_worker.current_turn_id = None;
        runtime
            .shared
            .workers
            .lock()
            .insert("s-disc".into(), exited_worker);

        // Broker placeholder must NEVER be mistaken for a live replacement worker (Finding 13).
        let mut placeholder = busy_snapshot("s-disc", "broker-s-disc", true);
        placeholder.state = WorkerState::Running;
        placeholder.current_turn_id = None;
        let check_cmd = CoreCommand::Prompt {
            session_id: "s-disc".into(),
            command_id: "cmd-ph".into(),
            turn_id: "t-ph".into(),
            content: vec![],
            cmid: None,
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("epoch-old".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        let saved = runtime
            .shared
            .workers
            .lock()
            .insert("s-disc".into(), placeholder);
        assert!(
            super::super::notice_recovery::replacement_worker_superseded_epoch(
                &runtime.shared,
                "s-disc",
                &check_cmd,
            )
            .is_none(),
            "broker placeholder must not authorize idle guard release"
        );
        if let Some(exited) = saved {
            runtime
                .shared
                .workers
                .lock()
                .insert("s-disc".into(), exited);
        }

        // Disconnect recovers remaining unattempted prompt to queue and rolls back Starting -> Exited.
        runtime.disconnect();
        let info = hub.session_info("s-disc").unwrap();
        assert_eq!(info.queue_count, 1);
        assert_eq!(info.drafts_count, 1);
        assert_eq!(hub.status("s-disc"), Some(super::super::Status::Exited));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn durable_disconnect_park_live_replacement_retire_and_draft_dedupe() {
        let health = Arc::new(crate::core::PersistenceHealth::default());
        let (sink, mut store_rx) = crate::core::StoreSink::channel(16, health);
        let hub = Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        hub.create_local_session(
            "s-dur".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-dur".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());

        // Finding 3 + Finding 1: live replacement snapshot retires a still-`sent` prompt
        // bound to an earlier epoch ONLY AFTER SQL confirms the recovery draft commit.
        let sent_cmd = CoreCommand::Prompt {
            session_id: "s-dur".into(),
            command_id: "cmd-sent".into(),
            turn_id: "t-1".into(),
            content: vec![serde_json::json!({"type": "text", "text": "hello world"})],
            cmid: Some("cmid-dur".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("epoch-old".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        runtime
            .shared
            .pending
            .lock()
            .insert("cmd-sent".into(), sent_cmd);
        runtime.shared.sent.lock().insert("cmd-sent".into());
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-sent".into());
        let caller = crate::prompt_origin::RequestCaller::Person;
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(&caller);
        hub.record_prompt_dispatch("s-dur", "cmid-dur".into(), origin.clone(), origin, 1_000);

        let mut replacement = busy_snapshot("s-dur", "epoch-new", true);
        replacement.state = WorkerState::Running;
        replacement.current_turn_id = None;
        // Exercise the actual live Frame::Snapshot handler (Finding 1).
        runtime
            .project_trace_fixture(Frame::Snapshot {
                worker: Box::new(replacement),
            })
            .await;

        // Pending command stays fenced until SQL ACK confirms the draft.
        assert!(runtime.shared.pending.lock().contains_key("cmd-sent"));
        assert!(runtime.shared.fenced_commands.lock().contains("cmd-sent"));
        assert_eq!(hub.session_info("s-dur").unwrap().drafts_count, 0);

        // Live replacement park uses 2-phase SQL (provisional-recovery-* then finalized recovery-*).
        for expected_prefix in ["provisional-recovery-", "recovery-"] {
            let (ack_id, drafts) = loop {
                if let crate::core::StoreWrite::UpdatePending {
                    ack_id: Some(id),
                    drafts,
                    ..
                } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                    .await
                    .unwrap()
                    .unwrap()
                {
                    break (id, drafts);
                }
            };
            assert!(
                drafts.iter().any(|d| d.id.starts_with(expected_prefix)),
                "expected {expected_prefix} draft in SQL write, got {drafts:?}"
            );
            acks.complete(ack_id, Ok(true));
        }
        hub.drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;

        assert!(!runtime.shared.pending.lock().contains_key("cmd-sent"));
        assert_eq!(hub.session_info("s-dur").unwrap().drafts_count, 1);
        assert!(
            hub.take_pending_echo_for_undelivered("s-dur", "cmid-dur")
                .is_none()
        );
        let blocks = vec![serde_json::json!({"type": "text", "text": "hello world"})];
        assert_eq!(
            hub.front_submit_with_outcome(
                "s-dur",
                "hello world".into(),
                blocks,
                Some("cmid-dur".into()),
                &caller
            ),
            crate::core::FrontSubmitOutcome::Duplicate
        );
        let draft_id = hub.queue_and_drafts("s-dur", |_| true).unwrap().1[0]
            .id
            .clone();
        assert!(draft_id.starts_with("recovery-"));
        hub.activate_all_drafts("s-dur", &caller);
        assert_eq!(hub.session_info("s-dur").unwrap().drafts_count, 1);
        hub.activate_draft("s-dur", &draft_id, Some("cmid-dur".into()), &caller);
        hub.drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;
        assert_eq!(hub.session_info("s-dur").unwrap().drafts_count, 1);
        hub.submit(
            "s-dur",
            "other".into(),
            vec![],
            Some("cmid-other".into()),
            &caller,
        );
        hub.activate_draft("s-dur", &draft_id, Some("cmid-other".into()), &caller);
        hub.drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;
        assert_eq!(hub.session_info("s-dur").unwrap().drafts_count, 1);

        // Finding 7: recovery activation commits draft->queue in SQL before leaving drafts.
        hub.activate_draft("s-dur", &draft_id, Some("cmid-fresh".into()), &caller);
        let act_ack = loop {
            if let crate::core::StoreWrite::UpdatePending {
                ack_id: Some(id),
                queue,
                drafts,
                ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                assert!(drafts.is_empty());
                assert!(
                    queue
                        .iter()
                        .any(|q| q.cmid.as_deref() == Some("cmid-fresh"))
                );
                break id;
            }
        };
        assert_eq!(
            hub.session_info("s-dur").unwrap().drafts_count,
            1,
            "draft stays visible until SQL commits draft->queue transition"
        );
        acks.complete(act_ack, Ok(true));
        hub.drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;
        assert_eq!(hub.session_info("s-dur").unwrap().drafts_count, 0);
        assert_eq!(hub.session_info("s-dur").unwrap().queue_count, 2);
        assert!(
            hub.take_pending_echo_for_undelivered("s-dur", "cmid-dur")
                .is_none()
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn late_positive_ack_rolls_back_in_flight_recovery_park_and_failed_park_retries() {
        // Finding 2: late positive ACK while SQL is in flight rolls back committed draft.
        let health = Arc::new(crate::core::PersistenceHealth::default());
        let (sink, mut store_rx) = crate::core::StoreSink::channel(16, health);
        let hub = Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        hub.create_local_session(
            "s-race".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-race".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        let cmd = CoreCommand::Prompt {
            session_id: "s-race".into(),
            command_id: "cmd-race".into(),
            turn_id: "t-race".into(),
            content: vec![serde_json::json!({"type": "text", "text": "race"})],
            cmid: Some("cmid-race".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("e-old".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        runtime.shared.pending.lock().insert("cmd-race".into(), cmd);
        runtime.shared.sent.lock().insert("cmd-race".into());
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-race".into());
        let mut repl = busy_snapshot("s-race", "e-new", true);
        repl.state = WorkerState::Running;
        repl.current_turn_id = None;
        runtime
            .project_trace_fixture(Frame::Snapshot {
                worker: Box::new(repl),
            })
            .await;
        let ack_id = loop {
            if let crate::core::StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };
        // Finding 3: verified UndeliveredPrompt arrives while fenced, then late positive ACK cancels park.
        let caller = crate::prompt_origin::RequestCaller::Person;
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(&caller);
        hub.record_prompt_dispatch("s-race", "cmid-race".into(), origin.clone(), origin, 2_000);
        runtime.shared.in_flight_prompts.lock().record(
            "cmd-race".into(),
            super::super::prompt_tracking::prepare_sent_prompt(
                &hub,
                "s-race",
                "t-race",
                Some("cmid-race"),
                &[serde_json::json!({"type": "text", "text": "race"})],
            ),
        );
        super::super::prompt_tracking::handle_undelivered_prompt(
            &runtime.shared,
            "s-race",
            "cmd-race",
            Some("cmid-race"),
            vec![serde_json::json!({"type": "text", "text": "race"})],
            "acp startup failed".into(),
        );
        let _ = super::super::notice_recovery::take_acked_command(
            &runtime.shared,
            "s-race",
            "cmd-race",
            true,
            None,
        );
        // Phase 1 provisional SQL completes, then acknowledged cancellation SQL removes provisional draft (Finding 4).
        acks.complete(ack_id, Ok(true));
        let cancel_ack = loop {
            if let crate::core::StoreWrite::UpdatePending {
                ack_id: Some(id),
                drafts,
                ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                assert!(
                    drafts.is_empty(),
                    "cancellation write must delete provisional draft"
                );
                break id;
            }
        };
        acks.complete(cancel_ack, Ok(true));
        hub.drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;
        assert_eq!(hub.session_info("s-race").unwrap().drafts_count, 0);
        assert_eq!(
            hub.session_info("s-race").unwrap().queue_count,
            1,
            "verified UndeliveredPrompt + positive ACK must requeue exact 1 copy instead of losing it"
        );
        assert!(!runtime.shared.pending.lock().contains_key("cmd-race"));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn failed_recovery_park_retries_across_runtime_drop_and_restore_seeds_prefix() {
        let health = Arc::new(crate::core::PersistenceHealth::default());
        let (sink, mut store_rx) = crate::core::StoreSink::channel(16, health);
        let hub = Hub::with_store(Some(sink));
        let acks = hub.store_ack_sink();
        hub.create_local_session(
            "s-race".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-race".into(),
            SessionOrigin::Web,
            false,
        );
        // Finding 3: failed SQL park stays retryable on Hub across runtime drop & retries cleanly.
        let cmd2 = CoreCommand::Prompt {
            session_id: "s-race".into(),
            command_id: "cmd-fail".into(),
            turn_id: "t-fail".into(),
            content: vec![serde_json::json!({"type": "text", "text": "retry me"})],
            cmid: Some("cmid-fail".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("e-old".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        let rt2 = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        rt2.shared.pending.lock().insert("cmd-fail".into(), cmd2);
        rt2.shared.attempted_sends.lock().insert("cmd-fail".into());
        super::super::notice_recovery::recover_pending_prompts_on_disconnect(&rt2.shared);
        drop(rt2);
        for _ in 0..4 {
            let aid = loop {
                if let crate::core::StoreWrite::UpdatePending {
                    ack_id: Some(id), ..
                } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                    .await
                    .unwrap()
                    .unwrap()
                {
                    break id;
                }
            };
            acks.complete(aid, Err("db busy".into()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        assert_eq!(hub.session_info("s-race").unwrap().drafts_count, 0);
        // Finding 5: reconnect handshake bootstrap retries failed park without direct call.
        let rt3 = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());
        super::merge_worker_snapshots(&rt3.shared, Vec::new()).await;
        let retry_ack = loop {
            if let crate::core::StoreWrite::UpdatePending {
                ack_id: Some(id), ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break id;
            }
        };
        acks.complete(retry_ack, Ok(true));
        hub.drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;
        assert_eq!(hub.session_info("s-race").unwrap().drafts_count, 1);

        // Finding 7: restore_sessions seeds past recovery-q<N> and preserves prefix on dedupe.
        let hub_restore = Hub::new();
        let meta = hub.session_info("s-race").unwrap().meta;
        let dup_draft = crate::core::QueuedMessage {
            id: "recovery-q5".into(),
            text: "dup".into(),
            content: vec![],
            cmid: Some("c-dup".into()),
            schedule: None,
            origin: None,
            initiator: None,
        };
        hub_restore.restore_reconciling_runtime(vec![crate::core::RestoredSession {
            meta,
            log: vec![],
            event_count: 0,
            reached_start: true,
            next_seq: 1,
            queue: vec![],
            drafts: vec![dup_draft.clone(), dup_draft],
            config_options: None,
            config_preferences: serde_json::json!({}),
            mobile_review_state: serde_json::json!({}),
            folder_id: None,
            last_event_at_ms: None,
        }]);
        let restored_drafts = hub_restore.queue_and_drafts("s-race", |_| true).unwrap().1;
        assert_eq!(restored_drafts[0].id, "recovery-q5");
        assert_eq!(restored_drafts[1].id, "recovery-q6");

        // Finding 4: provisional-recovery-* drafts are ineligible for restore.
        // Finding 6: persisted session_retired_undelivered prevents hydration rearm after draft deletion.
        let hub_f6 = Hub::new();
        hub_f6.set_setting(
            format!(
                "{}s-race",
                crate::model::settings_keys::SESSION_RETIRED_UNDELIVERED_PREFIX
            ),
            serde_json::json!(["cmid-deleted"]),
        );
        let provisional = crate::core::QueuedMessage {
            id: "provisional-recovery-q9".into(),
            text: "provisional".into(),
            content: vec![],
            cmid: Some("c-prov".into()),
            schedule: None,
            origin: None,
            initiator: None,
        };
        hub_f6.restore_reconciling_runtime(vec![crate::core::RestoredSession {
            meta: hub.session_info("s-race").unwrap().meta,
            log: vec![],
            event_count: 0,
            reached_start: true,
            next_seq: 1,
            queue: vec![],
            drafts: vec![provisional],
            config_options: None,
            config_preferences: serde_json::json!({}),
            mobile_review_state: serde_json::json!({}),
            folder_id: None,
            last_event_at_ms: None,
        }]);
        assert_eq!(hub_f6.session_info("s-race").unwrap().drafts_count, 0);
        let owner = crate::prompt_origin::DeliveryOrigin::from_caller(
            &crate::prompt_origin::RequestCaller::Person,
        );
        hub_f6.hydrate_prompt_dispatches(vec![crate::prompt_origin::PromptDispatchRecord {
            session_id: "s-race".into(),
            cmid: "cmid-deleted".into(),
            origin: owner.clone(),
            trigger: owner,
            dispatched_at_ms: 5_000,
        }]);
        assert!(
            hub_f6
                .take_pending_echo_for_undelivered("s-race", "cmid-deleted")
                .is_none(),
            "persisted retirement must keep deleted recovery draft's cmid unarmed after restart"
        );
    }
}

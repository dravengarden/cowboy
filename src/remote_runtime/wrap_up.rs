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
    async fn live_replacement_snapshot_retires_superseded_prompt_and_guards_recovery_draft() {
        let hub = Hub::new();
        hub.create_local_session(
            "s-sync".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-sync".into(),
            SessionOrigin::Web,
            false,
        );
        let runtime = super::super::RemoteRuntime::for_test(hub.clone(), Vec::new());

        // Live Frame::Snapshot retires a still-`sent` prompt bound to an earlier epoch,
        // parks a `recovery-*` draft with `cmid: None` (so the original op is never
        // prematurely confirmed), and never deadlocks on `shared.workers`.
        let sent_cmd = CoreCommand::Prompt {
            session_id: "s-sync".into(),
            command_id: "cmd-sent".into(),
            turn_id: "t-1".into(),
            content: vec![serde_json::json!({"type": "text", "text": "hello world"})],
            cmid: Some("cmid-orig".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("epoch-old".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        let notice_cmd = CoreCommand::Prompt {
            session_id: "s-sync".into(),
            command_id: "cmd-notice".into(),
            turn_id: "t-n".into(),
            content: vec![serde_json::json!({"type": "text", "text": "notice"})],
            cmid: Some(format!("{}c1", crate::agent_model::CALL_NOTICE_PREFIX)),
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
        runtime
            .shared
            .pending
            .lock()
            .insert("cmd-notice".into(), notice_cmd);
        runtime.shared.sent.lock().insert("cmd-sent".into());
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-sent".into());
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-notice".into());
        let caller = crate::prompt_origin::RequestCaller::Person;
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(&caller);
        hub.record_prompt_dispatch("s-sync", "cmid-orig".into(), origin.clone(), origin, 1_000);

        let mut replacement = busy_snapshot("s-sync", "epoch-new", true);
        replacement.state = WorkerState::Running;
        replacement.current_turn_id = None;
        runtime
            .project_trace_fixture(Frame::Snapshot {
                worker: Box::new(replacement),
            })
            .await;

        assert!(!runtime.shared.pending.lock().contains_key("cmd-sent"));
        assert!(!runtime.shared.pending.lock().contains_key("cmd-notice"));
        let (queue, drafts) = hub.queue_and_drafts("s-sync", |_| true).unwrap();
        assert!(queue.is_empty());
        assert_eq!(
            drafts.len(),
            1,
            "only ordinary prompt becomes a recovery draft"
        );
        assert!(drafts[0].id.starts_with("recovery-"));
        assert_eq!(
            drafts[0].cmid, None,
            "recovery draft must keep cmid=None to avoid premature delivery confirmation"
        );
        let draft_id = drafts[0].id.clone();

        // Late UndeliveredPrompt for the parked cmid must not re-queue a duplicate.
        super::super::prompt_tracking::handle_undelivered_prompt(
            &runtime.shared,
            "s-sync",
            "cmd-sent",
            Some("cmid-orig"),
            vec![serde_json::json!({"type": "text", "text": "hello world"})],
            "late undelivered".into(),
        );
        assert_eq!(hub.session_info("s-sync").unwrap().queue_count, 0);

        // Retrying the original cmid via front_submit or force_submit deduplicates.
        assert_eq!(
            hub.front_submit_with_outcome(
                "s-sync",
                "hello world".into(),
                vec![serde_json::json!({"type": "text", "text": "hello world"})],
                Some("cmid-orig".into()),
                &caller,
            ),
            crate::core::FrontSubmitOutcome::Duplicate
        );
        let _ = hub.force_submit(
            "s-sync",
            "hello world".into(),
            vec![serde_json::json!({"type": "text", "text": "hello world"})],
            Some("cmid-orig".into()),
            false,
            &caller,
        );
        assert_eq!(hub.session_info("s-sync").unwrap().queue_count, 0);

        // Bulk activation excludes `recovery-*` drafts.
        hub.activate_all_drafts("s-sync", &caller);
        assert_eq!(hub.session_info("s-sync").unwrap().drafts_count, 1);

        // Manual activation requires a fresh unused op ID and emits addressed CommandResult.
        let mut events = hub.subscribe();
        hub.activate_draft("s-sync", &draft_id, None, &caller);
        assert_eq!(hub.session_info("s-sync").unwrap().drafts_count, 1);
        let missing_err = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let frame = events.recv().await.unwrap();
                if let crate::core::Outbound::Error { message, .. } = frame.outbound() {
                    return message.clone();
                }
            }
        })
        .await
        .unwrap();
        assert!(missing_err.contains("operation id"));

        hub.activate_draft("s-sync", &draft_id, Some("cmid-orig".into()), &caller);
        assert_eq!(hub.session_info("s-sync").unwrap().drafts_count, 1);
        let (reused_cmid, reused_outcome) =
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                loop {
                    let frame = events.recv().await.unwrap();
                    if let crate::core::Outbound::CommandResult { cmid, outcome, .. } =
                        frame.outbound()
                    {
                        return (cmid.clone(), *outcome);
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(reused_cmid, "cmid-orig");
        assert_eq!(reused_outcome, crate::core::CommandOutcome::Rejected);

        // Fresh op ID activates atomically into the queue.
        hub.activate_draft("s-sync", &draft_id, Some("cmid-fresh".into()), &caller);
        assert_eq!(hub.session_info("s-sync").unwrap().drafts_count, 0);
        let (queue_after, _) = hub.queue_and_drafts("s-sync", |_| true).unwrap();
        assert_eq!(queue_after.len(), 1);
        assert_eq!(queue_after[0].cmid.as_deref(), Some("cmid-fresh"));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn rejected_binding_snapshot_preserves_cache_and_atomic_recovery_activation_persists_before_dispatch()
     {
        // 1. Finding 4: default-account session with execution_binding mismatch must
        //    leave previous cached worker, raw_wrap_up, pending EnsureSession, and
        //    pending prompts completely intact.
        let seed_hub = Hub::new();
        seed_hub.create_local_session(
            "s-bound".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-bound".into(),
            SessionOrigin::Web,
            false,
        );
        let mut bound_meta = seed_hub.session_info("s-bound").unwrap().meta;
        bound_meta.execution_binding = Some(
            crate::execution_environment::ExecutionBinding::from_record(serde_json::json!({
                "schema": 1,
                "kind": "remote",
                "target": "hawk"
            })),
        );
        let hub = Hub::new();
        hub.restore_reconciling_runtime(vec![crate::core::RestoredSession {
            meta: bound_meta.clone(),
            log: vec![],
            event_count: 0,
            reached_start: true,
            next_seq: 1,
            queue: vec![],
            drafts: vec![],
            config_options: None,
            config_preferences: serde_json::json!({}),
            mobile_review_state: serde_json::json!({}),
            folder_id: None,
            last_event_at_ms: None,
        }]);
        let mut good_worker = busy_snapshot("s-bound", "epoch-good", true);
        good_worker.launch = Some(crate::runtime_wire::StartSession {
            session_id: "s-bound".into(),
            provider: "antigravity".into(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: bound_meta.provider_account_id.clone(),
            provider_auth_generation: None,
            provider_behavior: None,
            cwd: "/tmp".into(),
            agent_session_id: Some("agent-1".into()),
            system: false,
            context_window: None,
            auto_compact_token_limit: None,
            cache_protection: None,
            generation: "g1".into(),
            fallback_for: None,
            adopt_only: true,
            execution_binding: bound_meta.execution_binding.clone(),
            extensions: None,
            mcp_plan: None,
            native_fork: None,
        });
        let runtime = super::super::RemoteRuntime::for_test(hub.clone(), vec![good_worker.clone()]);
        let cmd = CoreCommand::Prompt {
            session_id: "s-bound".into(),
            command_id: "cmd-bound".into(),
            turn_id: "t-b".into(),
            content: vec![serde_json::json!({"type": "text", "text": "stay pending"})],
            cmid: Some("cmid-b".into()),
            trace: None,
            echo_artifacts: false,
            observe_subagents: false,
            expected_worker_epoch: Some("epoch-good".into()),
            acknowledged_uncertain_sequence: Some(0),
        };
        runtime
            .shared
            .pending
            .lock()
            .insert("cmd-bound".into(), cmd);
        runtime
            .shared
            .attempted_sends
            .lock()
            .insert("cmd-bound".into());
        runtime
            .shared
            .wrap_up
            .record_raw_locked("s-bound", "epoch-good", true);
        runtime.shared.pending.lock().insert(
            "ensure:s-bound".into(),
            CoreCommand::EnsureSession {
                session: good_worker.launch.clone().unwrap(),
            },
        );
        // Incoming snapshot has mismatched execution_binding (launch = None) and wrap_up_steering = false.
        let mut stale_worker = busy_snapshot("s-bound", "epoch-stale", false);
        stale_worker.state = WorkerState::Running;
        stale_worker.current_turn_id = None;
        runtime
            .project_trace_fixture(Frame::Snapshot {
                worker: Box::new(stale_worker),
            })
            .await;
        assert_eq!(
            runtime
                .shared
                .workers
                .lock()
                .get("s-bound")
                .map(|w| w.worker_epoch.as_str()),
            Some("epoch-good"),
            "rejected snapshot must not overwrite cached worker authority"
        );
        assert!(
            !runtime
                .shared
                .wrap_up
                .raw_wrap_up
                .lock()
                .contains_key(&("s-bound".into(), "epoch-stale".into())),
            "rejected snapshot must not cache raw_wrap_up for stale epoch"
        );
        assert!(
            runtime.shared.pending.lock().contains_key("ensure:s-bound"),
            "rejected snapshot must not acknowledge pending EnsureSession"
        );
        assert!(
            runtime.shared.pending.lock().contains_key("cmd-bound"),
            "rejected snapshot must not retire pending prompts"
        );

        // 2. Findings 1, 3, 5, 6: atomic park (draft + retired_dispatches in one SQL write,
        //    addressed confirm_delivered ONLY AFTER commit and on replay) + durable recovery
        //    activation (rejects synthetic op IDs, persists draft -> queue before dispatch,
        //    and rolls back cleanly on SQL error).
        let health = Arc::new(crate::core::PersistenceHealth::default());
        let (sink, mut store_rx) = crate::core::StoreSink::channel(16, health);
        let hub_store = Hub::with_store(Some(sink));
        let acks = hub_store.store_ack_sink();
        let (disp_tx, mut disp_rx) = tokio::sync::mpsc::channel(16);
        hub_store.set_dispatch_tx(disp_tx);
        hub_store.create_local_session(
            "s-atomic".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-atomic".into(),
            SessionOrigin::Web,
            false,
        );
        hub_store.set_status("s-atomic", super::super::Status::Running, None);
        let caller = crate::prompt_origin::RequestCaller::Person;
        let origin = crate::prompt_origin::DeliveryOrigin::from_caller(&caller);
        hub_store.record_prompt_dispatch(
            "s-atomic",
            "op-orig".into(),
            origin.clone(),
            origin,
            5_000,
        );
        let mut events = hub_store.subscribe();

        hub_store.park_recovery_draft(
            "s-atomic",
            "recover me".into(),
            vec![serde_json::json!({"type": "text", "text": "recover me"})],
            Some("op-orig".into()),
        );
        let (park_ack, park_drafts, park_retired) = loop {
            if let crate::core::StoreWrite::UpdatePending {
                ack_id: Some(id),
                drafts,
                retired_dispatches,
                ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                break (id, drafts, retired_dispatches);
            }
        };
        assert_eq!(park_drafts.len(), 1);
        assert_eq!(park_retired, vec!["op-orig".to_owned()]);
        // Before SQL ACK, duplicate submit must NOT confirm `op-orig` early.
        assert_eq!(
            hub_store.front_submit_with_outcome(
                "s-atomic",
                "recover me".into(),
                vec![],
                Some("op-orig".into()),
                &caller,
            ),
            crate::core::FrontSubmitOutcome::Duplicate
        );
        while let Ok(frame) = events.try_recv() {
            if let crate::core::Outbound::SyncPatch { confirmed, .. } = frame.outbound() {
                assert!(
                    !confirmed.iter().any(|c| c == "op-orig"),
                    "op-orig must not be confirmed before atomic park commits"
                );
            }
        }
        // Complete SQL park -> browser receives addressed `confirmed: ["op-orig"]` with recovery draft.
        acks.complete(park_ack, Ok(true));
        let confirmed_after_park = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let frame = events.recv().await.unwrap();
                if let crate::core::Outbound::SyncPatch { confirmed, .. } = frame.outbound()
                    && confirmed.iter().any(|c| c == "op-orig")
                {
                    return confirmed.clone();
                }
            }
        })
        .await
        .unwrap();
        assert!(confirmed_after_park.iter().any(|c| c == "op-orig"));
        let draft_id = hub_store.queue_and_drafts("s-atomic", |_| true).unwrap().1[0]
            .id
            .clone();

        // Synthetic op IDs (`cowboy-*`, `__*`) must be refused on recovery draft activation.
        for bad_op in ["cowboy-retry:1", "__sched:1", "managed-call:1"] {
            hub_store.activate_draft("s-atomic", &draft_id, Some(bad_op.into()), &caller);
            assert_eq!(hub_store.session_info("s-atomic").unwrap().drafts_count, 1);
        }

        // Recovery activation persists draft -> queue BEFORE dispatching, and rolls back on SQL failure.
        hub_store.activate_draft("s-atomic", &draft_id, Some("op-fail".into()), &caller);
        assert!(
            disp_rx.try_recv().is_err(),
            "idle session must not dispatch recovery draft before UpdatePending commits"
        );
        let fail_ack = loop {
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
                assert_eq!(queue.len(), 1);
                assert_eq!(queue[0].cmid.as_deref(), Some("op-fail"));
                assert!(drafts.is_empty());
                break id;
            }
        };
        acks.complete(fail_ack, Err("db busy".into()));
        hub_store
            .drain_teardown_requeues(std::time::Duration::from_secs(2))
            .await;
        assert_eq!(hub_store.session_info("s-atomic").unwrap().drafts_count, 1);
        assert_eq!(hub_store.session_info("s-atomic").unwrap().queue_count, 0);
        assert!(disp_rx.try_recv().is_err());

        // Succeeding activation persists draft -> queue and dispatches ONLY AFTER SQL ACK.
        hub_store.activate_draft("s-atomic", &draft_id, Some("op-send-2".into()), &caller);
        assert!(disp_rx.try_recv().is_err());
        let ok_ack = loop {
            if let crate::core::StoreWrite::UpdatePending {
                ack_id: Some(id),
                queue,
                ..
            } = tokio::time::timeout(std::time::Duration::from_secs(2), store_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                assert_eq!(queue.len(), 1);
                assert_eq!(queue[0].cmid.as_deref(), Some("op-send-2"));
                break id;
            }
        };
        acks.complete(ok_ack, Ok(true));
        let dispatched = tokio::time::timeout(std::time::Duration::from_secs(2), disp_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dispatched.cmid.as_deref(), Some("op-send-2"));
    }

    #[tokio::test]
    async fn exact_undelivered_retirement_survives_restart_without_wall_clock_collision() {
        // Finding 7: exact per-dispatch `undelivered_retired` suppresses the parked/deleted
        // dispatch after restart while NEVER suppressing a fresh dispatch on the exact same
        // millisecond or after a backward clock correction, and restore seeds past `recovery-q5`.
        let seed_hub = Hub::new();
        seed_hub.create_local_session(
            "s-exact".into(),
            "antigravity".into(),
            "/tmp".into(),
            "s-exact".into(),
            SessionOrigin::Web,
            false,
        );
        let meta = seed_hub.session_info("s-exact").unwrap().meta;
        let dup_draft = crate::core::QueuedMessage {
            id: "recovery-q5".into(),
            text: "dup".into(),
            content: vec![],
            cmid: None,
            schedule: None,
            origin: None,
            initiator: None,
        };
        let hub_restore = Hub::new();
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
        let restored_drafts = hub_restore.queue_and_drafts("s-exact", |_| true).unwrap().1;
        assert_eq!(restored_drafts[0].id, "recovery-q5");
        assert_eq!(restored_drafts[1].id, "recovery-q6");

        let owner = crate::prompt_origin::DeliveryOrigin::from_caller(
            &crate::prompt_origin::RequestCaller::Person,
        );
        hub_restore.hydrate_prompt_dispatches(vec![
            crate::prompt_origin::PromptDispatchRecord {
                session_id: "s-exact".into(),
                cmid: "cmid-parked".into(),
                origin: owner.clone(),
                trigger: owner.clone(),
                dispatched_at_ms: 5_000,
                undelivered_retired: true,
            },
            // Same millisecond (5_000) fresh dispatch must NOT be suppressed by cmid-parked.
            crate::prompt_origin::PromptDispatchRecord {
                session_id: "s-exact".into(),
                cmid: "cmid-same-ms".into(),
                origin: owner.clone(),
                trigger: owner.clone(),
                dispatched_at_ms: 5_000,
                undelivered_retired: false,
            },
        ]);
        assert!(hub_restore.is_cmid_parked_or_fenced("s-exact", "cmid-parked"));
        assert!(!hub_restore.is_cmid_parked_or_fenced("s-exact", "cmid-same-ms"));
        assert!(
            hub_restore
                .take_pending_echo_for_undelivered("s-exact", "cmid-parked")
                .is_none()
        );
        assert!(
            hub_restore
                .take_pending_echo_for_undelivered("s-exact", "cmid-same-ms")
                .is_some()
        );
        // Even a live dispatch whose timestamp stepped backward (4_999 < 5_000) is NOT suppressed.
        hub_restore.record_prompt_dispatch(
            "s-exact",
            "cmid-clock-step".into(),
            owner.clone(),
            owner,
            4_999,
        );
        assert!(!hub_restore.is_cmid_parked_or_fenced("s-exact", "cmid-clock-step"));
        assert!(
            hub_restore
                .take_pending_echo_for_undelivered("s-exact", "cmid-clock-step")
                .is_some()
        );
    }
}

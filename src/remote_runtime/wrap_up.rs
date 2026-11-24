//! Antigravity wrap-up steering Controller-side state: raw capability cache,
//! protocol/config sanitization, per-turn hint deduplication, and direct
//! bounded WebSocket hint dispatch bypassing the shared runtime `UnixStream` FIFO.

use std::collections::HashMap;
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
    /// Raw broker-reported `wrap_up_steering` per session (before Controller
    /// protocol/config gating), so reconnects & live config reloads recompute cleanly.
    raw_wrap_up: Mutex<HashMap<String, bool>>,
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

    pub(super) fn record_raw_and_sanitize(
        &self,
        worker: &mut WorkerSnapshot,
        machine_protocol: u16,
        runtime_protocol: u16,
        hub: &Hub,
    ) {
        let raw = worker.wrap_up_steering;
        self.raw_wrap_up
            .lock()
            .insert(worker.session_id.clone(), raw);
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

    pub(super) fn record_raw_event(
        &self,
        session_id: &str,
        raw: bool,
        worker: Option<&mut WorkerSnapshot>,
        machine_protocol: u16,
        runtime_protocol: u16,
        hub: &Hub,
    ) -> bool {
        self.raw_wrap_up.lock().insert(session_id.to_owned(), raw);
        let provider = worker
            .as_deref()
            .and_then(|w| resolved_provider(w, hub))
            .or_else(|| hub.session_info(session_id).map(|info| info.meta.provider));
        let effective = effective_wrap_up_steering(
            provider.as_deref(),
            raw,
            Some(machine_protocol),
            runtime_protocol,
            self.config_enabled(),
        );
        if let Some(worker) = worker {
            clamp_delivery_capabilities_for_provider(
                provider.as_deref(),
                &mut worker.supports_current_turn,
                &mut worker.delivery_controls.next_checkpoint,
            );
            worker.wrap_up_steering = effective;
        }
        effective
    }

    pub(super) fn recompute_all(
        &self,
        workers: &mut HashMap<String, WorkerSnapshot>,
        machine_protocol: u16,
        runtime_protocol: u16,
        hub: &Hub,
    ) {
        let enabled = self.config_enabled();
        let raw_map = self.raw_wrap_up.lock().clone();
        for (session_id, worker) in workers.iter_mut() {
            let raw = raw_map.get(session_id).copied().unwrap_or(false);
            let provider = resolved_provider(worker, hub);
            clamp_delivery_capabilities_for_provider(
                provider.as_deref(),
                &mut worker.supports_current_turn,
                &mut worker.delivery_controls.next_checkpoint,
            );
            let effective = effective_wrap_up_steering(
                provider.as_deref(),
                raw,
                Some(machine_protocol),
                runtime_protocol,
                enabled,
            );
            worker.wrap_up_steering = effective;
            hub.set_current_turn_support(session_id, worker.supports_current_turn);
            hub.set_wrap_up_steering(session_id, effective);
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
        if self.last_hinted.lock().get(session_id) == Some(target) {
            return true;
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
        let tx = self.hint_ws_tx.lock().clone();
        let Some(tx) = tx else {
            return false;
        };
        match tx.try_send(text) {
            Ok(()) => {
                self.last_hinted
                    .lock()
                    .insert(session_id.to_owned(), target.clone());
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
        let runtime_protocol = self.shared.protocol.load(Ordering::Acquire);
        let mut workers = self.shared.workers.lock();
        self.shared.wrap_up.recompute_all(
            &mut workers,
            self.shared.machine_protocol,
            runtime_protocol,
            &self.shared.hub,
        );
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
            Some(worker),
            self.machine_protocol,
            self.protocol.load(Ordering::Acquire),
            &self.hub,
        );
        self.hub.set_wrap_up_steering(session_id, effective);
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
pub(super) async fn merge_worker_snapshots(shared: &super::Shared, workers: Vec<WorkerSnapshot>) {
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

//! Schema-1 pending user-MCP call sentinels and local `cowboy/*` relay dispatch.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Ledger, PENDING_CALL_PREFIX, SettlementGuard, UNCERTAIN_REPORT, refused};
use crate::worker_execution::Client;
use crate::worker_execution::relay::AckState;

impl Ledger {
    /// Record a Schema-1 pending user-MCP call sentinel before writing to the server.
    pub(in crate::worker_execution) fn record_pending_call(
        &self,
        call_id: &str,
    ) -> Result<(), &'static str> {
        let mut state = self.state.lock();
        if state.closed {
            return Err("the session is ending");
        }
        if state.unwritable {
            return Err("the execution ledger cannot be written; the call was refused");
        }
        let sentinel = format!("{PENDING_CALL_PREFIX}{call_id}");
        state.record.active_processes.insert(sentinel.clone());
        if let Err(error) = self.persist(&state.record) {
            state.record.active_processes.remove(&sentinel);
            tracing::warn!(%error, "execution ledger pending-call write failed; call refused");
            state.unwritable = true;
            return Err("the execution ledger cannot be written; the call was refused");
        }
        Ok(())
    }

    /// Remove a Schema-1 pending user-MCP call sentinel on clean acknowledgement
    /// or `Failure::NotSent`. Fails closed (converts to uncertain) if persist fails.
    pub(in crate::worker_execution) fn clear_pending_call(&self, call_id: &str) -> bool {
        let mut state = self.state.lock();
        let sentinel = format!("{PENDING_CALL_PREFIX}{call_id}");
        if !state.record.active_processes.remove(&sentinel) {
            return true;
        }
        if let Err(error) = self.persist(&state.record) {
            tracing::warn!(%error, "execution ledger pending-call clear failed; failing closed");
            state.unwritable = true;
            let sequence = self.record_uncertain_locked(&mut state, "tools/call");
            drop(state);
            if let Some(observer) = self.observer.get() {
                observer("tools/call", sequence);
            }
            return false;
        }
        true
    }

    /// Atomically remove one or more pending user-MCP call sentinels and record
    /// `uncertain_effect = "tools/call"` with a single `uncertain_sequence` bump.
    pub(in crate::worker_execution) fn convert_pending_calls_to_uncertain(
        &self,
        call_ids: &[String],
    ) {
        if call_ids.is_empty() {
            return;
        }
        let mut state = self.state.lock();
        for call_id in call_ids {
            state
                .record
                .active_processes
                .remove(&format!("{PENDING_CALL_PREFIX}{call_id}"));
        }
        let sequence = self.record_uncertain_locked(&mut state, "tools/call");
        drop(state);
        if let Some(observer) = self.observer.get() {
            observer("tools/call", sequence);
        }
    }
}

/// Dispatch worker-local `cowboy/*` endpoints before ledger admission, and
/// reject any remaining `cowboy/` method so keeper-internal endpoints stay
/// unreachable from the account unit (while preserving `cowboy/memory`).
pub(super) async fn relay_local_cowboy(
    client: &Arc<Client>,
    ledger: &Arc<Ledger>,
    method: &str,
    params: &Value,
    connection_id: u64,
) -> Option<(Value, Option<SettlementGuard>, AckState)> {
    if method == crate::worker_execution::memory_bridge::MEMORY_REQUEST {
        if params.get("path").and_then(Value::as_str) == Some("/v1/observe")
            && !ledger.observation_open()
        {
            return Some((
                refused("memory evidence is accepted only for a turn"),
                None,
                AckState::None,
            ));
        }
        let reply = match client.memory.get() {
            Some(memory) => memory.forward(params).await,
            None => refused("memory is not available in this session"),
        };
        return Some((reply, None, AckState::None));
    }
    if method == crate::worker_execution::extension_broker::EXTENSIONS_REQUEST {
        let reply = match client.extensions.get() {
            Some(broker) => broker.list(client, params).await,
            None => json!({"result": {"tools": [], "instructions": ""}}),
        };
        return Some((reply, None, AckState::None));
    }
    if method == crate::worker_execution::extension_broker::EXTENSION_CALL {
        return Some(match client.extensions.get().cloned() {
            Some(broker) => broker.call_guarded(client, params, connection_id).await,
            None => (
                refused("this session has no extensions"),
                None,
                AckState::None,
            ),
        });
    }
    if method == UNCERTAIN_REPORT {
        if let Some(call_id) = params.get("callId").and_then(Value::as_str)
            && let Some(broker) = client.extensions.get()
            && broker.fail_pending_ack(call_id)
        {
            return Some((json!({"result": {}}), None, AckState::None));
        }
        let reported = params
            .get("method")
            .and_then(Value::as_str)
            .filter(|reported| {
                !reported.is_empty()
                    && reported.len() <= 64
                    && reported
                        .bytes()
                        .all(|byte| byte.is_ascii_graphic() || byte == b' ')
            })
            .unwrap_or("request");
        ledger.mark_uncertain(reported);
        return Some((json!({"result": {}}), None, AckState::None));
    }
    if method.starts_with("cowboy/") {
        return Some((refused("reserved cowboy method"), None, AckState::None));
    }
    None
}

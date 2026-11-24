//! Antigravity wrap-up steering ACP helpers: 1-slot coalesced hint handoff,
//! P2 initialize capability parsing/clamping, per-attempt nonce injection,
//! and reliable-first `cowboy/wrapUpHint` dispatch.

use agent_client_protocol::schema::v1::{PromptRequest, SessionId};
use agent_client_protocol::{Agent, ConnectionTo, JsonRpcNotification};
use parking_lot::Mutex;
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

use crate::runtime_wire::{CURRENT_TURN_CAPABILITIES_UPDATE, DeliveryTarget};
use crate::wrap_up::{
    ATTEMPT_NONCE_META_KEY, WRAP_UP_STEERING_CAPABILITY_KEY,
    clamp_delivery_capabilities_for_provider, is_antigravity_provider,
};

/// Single-slot O(1) coalesced handoff from `worker.rs` to `src/acp.rs`.
/// Newer targets overwrite older ones without growing any mailbox.
#[derive(Default)]
pub struct WrapUpHintSlot {
    pending: Mutex<Option<DeliveryTarget>>,
    notify: Notify,
}

impl WrapUpHintSlot {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, target: DeliveryTarget) {
        *self.pending.lock() = Some(target);
        self.notify.notify_one();
    }

    #[must_use]
    pub fn take(&self) -> Option<DeliveryTarget> {
        self.pending.lock().take()
    }

    pub async fn notified(&self) {
        self.notify.notified().await;
    }
}

/// Per-prompt wrap-up state attached to `ActivePrompt` inside `src/acp.rs`.
pub struct ActiveWrapUp {
    pub turn_id: Option<String>,
    pub compacting: bool,
    attempt_nonce: Mutex<Option<String>>,
    hint_sent: AtomicBool,
    cancelled: AtomicBool,
}

impl ActiveWrapUp {
    #[must_use]
    pub fn new(turn_id: Option<String>, compacting: bool) -> Self {
        Self {
            turn_id,
            compacting,
            attempt_nonce: Mutex::new(None),
            hint_sent: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
        }
    }

    /// Mint a fresh 128-bit hex nonce for a new attempt ONLY when
    /// `raw_wrap_up_eligible` is true; otherwise leave `None` (pre-P2 isolation).
    pub fn begin_attempt(&self, raw_wrap_up_eligible: bool) -> Option<String> {
        let nonce = raw_wrap_up_eligible.then(|| uuid::Uuid::new_v4().simple().to_string());
        *self.attempt_nonce.lock() = nonce.clone();
        self.hint_sent.store(false, Ordering::Release);
        nonce
    }

    pub fn mark_cancelled(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn claim_hint_nonce(&self, target_turn_id: &str) -> Option<String> {
        if self.compacting
            || self.cancelled.load(Ordering::Acquire)
            || self.turn_id.as_deref() != Some(target_turn_id)
        {
            return None;
        }
        let nonce = self.attempt_nonce.lock().clone()?;
        if self.hint_sent.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(nonce)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, JsonRpcNotification)]
#[notification(method = "cowboy/wrapUpHint")]
#[serde(rename_all = "camelCase")]
pub struct WrapUpHintNotification {
    pub session_id: SessionId,
    pub attempt_nonce: String,
}

/// Parse ACP `initialize._meta` into the Cowboy capability update event and
/// return `(update_json, raw_wrap_up_eligible)`.
#[must_use]
pub fn initialize_capabilities_update(
    provider_id: &str,
    initialize_meta: Option<&Map<String, Value>>,
    managed_read_only: bool,
    launch_wrap_up_eligible: bool,
) -> (Value, bool) {
    let mut supports_current_turn = !managed_read_only
        && initialize_meta
            .and_then(|meta| meta.get("steering"))
            .is_some_and(|steering| {
                steering.get("supported").and_then(Value::as_bool) == Some(true)
                    && steering.get("idleBehavior").and_then(Value::as_str)
                        == Some("promptRequired")
            });
    let mut next_checkpoint = supports_current_turn
        && initialize_meta
            .and_then(|meta| meta.get("steering"))
            .and_then(|steering| steering.get("priorities"))
            .and_then(Value::as_array)
            .is_some_and(|priorities| priorities.iter().any(|priority| priority == "next"));
    clamp_delivery_capabilities_for_provider(
        Some(provider_id),
        &mut supports_current_turn,
        &mut next_checkpoint,
    );
    let handshake_wrap_up = is_antigravity_provider(provider_id)
        && !managed_read_only
        && launch_wrap_up_eligible
        && initialize_meta
            .and_then(|meta| meta.get(WRAP_UP_STEERING_CAPABILITY_KEY))
            .is_some_and(|value| {
                value.get("v").and_then(Value::as_u64) == Some(1)
                    && value.get("adapter").and_then(Value::as_str)
                        == Some("cowboy-antigravity-acp")
            });
    let update = serde_json::json!({
        "sessionUpdate": CURRENT_TURN_CAPABILITIES_UPDATE,
        "currentTurn": supports_current_turn,
        "nextCheckpoint": next_checkpoint,
        "wrapUpSteering": handshake_wrap_up,
    });
    (update, handshake_wrap_up)
}

/// Merge managed round metadata and (ONLY when `attempt_nonce` is `Some`) the
/// P2 Antigravity `cowboy.dev/attemptNonce` onto `PromptRequest`.
#[must_use]
pub fn attach_prompt_meta(
    mut request: PromptRequest,
    managed_meta: Option<&Map<String, Value>>,
    attempt_nonce: Option<&str>,
) -> PromptRequest {
    match (managed_meta, attempt_nonce) {
        (None, None) => request,
        (Some(meta), None) => {
            request = request.meta(meta.clone());
            request
        }
        (managed, Some(nonce)) => {
            let mut merged = managed.cloned().unwrap_or_default();
            merged.insert(
                ATTEMPT_NONCE_META_KEY.to_owned(),
                Value::String(nonce.to_owned()),
            );
            request = request.meta(merged);
            request
        }
    }
}

/// Receive the next reliable `AgentCommand` while giving `cmd_rx` strict
/// `biased;` priority over coalesced best-effort wrap-up hints.
pub async fn recv_command_or_hint<T, F>(
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<T>,
    slot: Option<&WrapUpHintSlot>,
    mut on_hint: F,
) -> Option<T>
where
    F: FnMut(&WrapUpHintSlot),
{
    let Some(slot) = slot else {
        return cmd_rx.recv().await;
    };
    loop {
        tokio::select! {
            biased;
            cmd = cmd_rx.recv() => return cmd,
            () = slot.notified() => on_hint(slot),
        }
    }
}

/// Drain the coalesced 1-slot wrap-up hint and forward `cowboy/wrapUpHint` to
/// the ACP agent only when every session/turn/nonce/compaction invariant holds.
pub fn dispatch_pending_wrap_up_hint<F>(
    slot: &WrapUpHintSlot,
    managed_read_only: bool,
    target_matches: F,
    acp_id: &SessionId,
    active: Option<&ActiveWrapUp>,
    cx: &ConnectionTo<Agent>,
) where
    F: FnOnce(&DeliveryTarget) -> bool,
{
    let Some(target) = slot.take() else {
        return;
    };
    if managed_read_only || !target_matches(&target) {
        return;
    }
    let Some(active) = active else {
        return;
    };
    let Some(attempt_nonce) = active.claim_hint_nonce(&target.turn_id) else {
        return;
    };
    let _ = cx.send_notification(WrapUpHintNotification {
        session_id: acp_id.clone(),
        attempt_nonce,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_clamps_current_turn_and_gates_wrap_up_on_signed_launch() {
        let mut meta = Map::new();
        meta.insert(
            "steering".into(),
            serde_json::json!({"supported": true, "idleBehavior": "promptRequired", "priorities": ["next"]}),
        );
        meta.insert(
            WRAP_UP_STEERING_CAPABILITY_KEY.into(),
            serde_json::json!({"v": 1, "adapter": "cowboy-antigravity-acp"}),
        );

        let (update, eligible) =
            initialize_capabilities_update("antigravity", Some(&meta), false, true);
        assert_eq!(update["currentTurn"], false);
        assert_eq!(update["nextCheckpoint"], false);
        assert_eq!(update["wrapUpSteering"], true);
        assert!(eligible);

        // Pre-P2 launch refuses wrap-up even if untrusted init meta spoofs it.
        let (pre_p2, pre_eligible) =
            initialize_capabilities_update("antigravity", Some(&meta), false, false);
        assert_eq!(pre_p2["wrapUpSteering"], false);
        assert!(!pre_eligible);

        // Managed read-only session never enables wrap-up.
        let (managed, managed_eligible) =
            initialize_capabilities_update("antigravity", Some(&meta), true, true);
        assert_eq!(managed["wrapUpSteering"], false);
        assert!(!managed_eligible);

        let mut unversioned = Map::new();
        unversioned.insert(
            WRAP_UP_STEERING_CAPABILITY_KEY.into(),
            serde_json::json!({"supported": true}),
        );
        let (bad, bad_eligible) =
            initialize_capabilities_update("antigravity", Some(&unversioned), false, true);
        assert_eq!(bad["wrapUpSteering"], false);
        assert!(!bad_eligible);
    }

    #[test]
    fn active_wrap_up_enforces_pre_p2_isolation_and_single_send_per_attempt() {
        let pre_p2 = ActiveWrapUp::new(Some("t1".into()), false);
        assert!(pre_p2.begin_attempt(false).is_none());
        assert!(pre_p2.claim_hint_nonce("t1").is_none());

        let p2 = ActiveWrapUp::new(Some("t1".into()), false);
        let n1 = p2.begin_attempt(true).expect("fresh nonce");
        assert_eq!(n1.len(), 32);
        assert!(p2.claim_hint_nonce("t2").is_none());
        assert_eq!(p2.claim_hint_nonce("t1").as_deref(), Some(n1.as_str()));
        // Second claim in same attempt is suppressed (at most 1 hint per attempt).
        assert!(p2.claim_hint_nonce("t1").is_none());
        // Retry mints a fresh nonce and rearms single-send.
        let n2 = p2.begin_attempt(true).expect("fresh retry nonce");
        assert_ne!(n1, n2);
        assert_eq!(p2.claim_hint_nonce("t1").as_deref(), Some(n2.as_str()));

        // Compaction turns never claim a hint.
        let compact = ActiveWrapUp::new(Some("t3".into()), true);
        assert!(compact.begin_attempt(true).is_some());
        assert!(compact.claim_hint_nonce("t3").is_none());
    }

    #[test]
    fn attach_prompt_meta_injects_nonce_only_when_present() {
        let base = PromptRequest::new(SessionId::new("s1"), vec![]);
        let pre_p2 = attach_prompt_meta(base.clone(), None, None);
        assert!(pre_p2.meta.is_none());

        let p2 = attach_prompt_meta(base, None, Some("abc123"));
        assert_eq!(
            p2.meta.unwrap().get(ATTEMPT_NONCE_META_KEY),
            Some(&Value::String("abc123".into()))
        );
    }
}

//! Renew an idle session's agent context before a prompt reaches it, once the
//! Provider prompt cache has expired. See [`crate::context_rollover`].
use super::*;
use crate::context_rollover::{Policy, Rollover};

fn override_key(id: &str) -> String {
    format!(
        "{}{id}",
        crate::core::settings_keys::SESSION_CONTEXT_ROLLOVER_PREFIX
    )
}

fn session_override(hub: &Hub, id: &str) -> Option<bool> {
    hub.setting(&override_key(id))
        .and_then(|value| value.as_bool())
}

/// A continuation keeps the source's explicit choice.
pub(super) fn inherit(state: &AppState, source: &str, target: &str) {
    if let Some(enabled) = session_override(&state.hub, source) {
        state
            .hub
            .set_setting(override_key(target), serde_json::Value::Bool(enabled));
    }
}

async fn policy(state: &AppState, meta: &crate::core::SessionMeta) -> Policy {
    let preferences = match (meta.owner_user_id.as_deref(), state.store.as_ref()) {
        (Some(owner), Some(store)) => store
            .preferences(owner, crate::context_rollover::PREFERENCE_SCOPE)
            .await
            .ok()
            .flatten()
            .map(|preferences| preferences.values),
        _ => None,
    };
    crate::context_rollover::policy(
        preferences.as_ref(),
        session_override(&state.hub, &meta.id),
        &meta.provider,
    )
}

/// Whether nothing about the session would be lost or raced by a renewal: an
/// idle, ordinary session whose native context is resumable, with no
/// background work and no context send in flight. A live worker is replaced
/// only on a plain local runtime; a dormant one simply starts fresh.
fn quiescent(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    let live = state.supervisor.has_live_worker(&meta.id);
    !meta.system
        && !meta.closing
        && meta.agent_session_id.is_some()
        && meta.background_tasks == 0
        && !state.hub.is_managed_child(&meta.id)
        && if live {
            meta.status == Status::Running && meta.execution_binding.is_none()
        } else {
            meta.status == Status::Exited
        }
}

/// Prompts after at least this much idle are measured, renewed or not, so a
/// renewal's first turn can be compared with ordinary cold resumes.
const MEASURED_IDLE: std::time::Duration = std::time::Duration::from_mins(5);
/// Bound on prompts awaiting their first-turn measurement.
const MAX_PENDING: usize = 1_024;

struct Pending {
    provider: String,
    renewed: bool,
    idle_ms: u64,
    context_used: u64,
    dispatched: std::time::Instant,
}

static PENDING: std::sync::LazyLock<parking_lot::Mutex<HashMap<String, Pending>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

fn measure(meta: &crate::core::SessionMeta, policy: &Policy, idle_ms: u64, outcome: &str) {
    tracing::info!(
        target: "cowboy::evidence",
        event_name = "cowboy.context_rollover.decision",
        session = %meta.id,
        provider = %meta.provider,
        outcome,
        idle_ms,
        context_used = meta.context_used,
        threshold_ms = policy.idle_minutes.map_or(0, |minutes| minutes.saturating_mul(60_000)),
        min_context = policy.min_context_tokens,
        "context renewal decision"
    );
    let mut pending = PENDING.lock();
    if pending.len() >= MAX_PENDING {
        pending.clear();
    }
    pending.insert(
        meta.id.clone(),
        Pending {
            provider: meta.provider.clone(),
            renewed: outcome == "renewed",
            idle_ms,
            context_used: meta.context_used,
            dispatched: std::time::Instant::now(),
        },
    );
}

/// Record the first turn after a measured prompt: what the resume or the
/// renewal actually cost (`cowboy.context_rollover.first_turn`).
pub(in crate::server) async fn observe(state: Arc<AppState>) {
    let mut events = state.hub.subscribe();
    loop {
        let frame = match events.recv().await {
            Ok(frame) => frame,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let crate::core::Outbound::Event { envelope } = frame.outbound() else {
            continue;
        };
        let crate::core::Event::TurnEnd { usage, .. } = &envelope.event else {
            continue;
        };
        let Some(pending) = PENDING.lock().remove(&envelope.session_id) else {
            continue;
        };
        // An adapter that reports no reconciled usage still logs the turn.
        let usage = usage.as_ref();
        tracing::info!(
            target: "cowboy::evidence",
            event_name = "cowboy.context_rollover.first_turn",
            session = %envelope.session_id,
            provider = %pending.provider,
            renewed = pending.renewed,
            idle_ms = pending.idle_ms,
            context_used = pending.context_used,
            outcome = if usage.is_some() { "usage" } else { "usage_unknown" },
            input_tokens = usage.map_or(0, |usage| usage.input_tokens),
            cached_input_tokens = usage.and_then(|usage| usage.cached_input_tokens).unwrap_or_default(),
            cache_write_tokens = usage.and_then(|usage| usage.cache_write_tokens).unwrap_or_default(),
            uncached_input_tokens = usage.and_then(|usage| usage.uncached_input_tokens).unwrap_or_default(),
            output_tokens = usage.map_or(0, |usage| usage.output_tokens),
            duration_ms = pending.dispatched.elapsed().as_secs_f64() * 1000.0,
            "first turn after an idle prompt"
        );
    }
}

/// Renew the context if this prompt arrives after the Provider cache expired.
/// `Ok(true)` means the worker is being replaced: keep the prompt queued until
/// the fresh agent is running. Any failure before the native id is forgotten
/// leaves the session untouched and the prompt goes to its old context.
/// Every authored prompt after [`MEASURED_IDLE`] logs its decision.
pub(in crate::server) async fn before_dispatch(
    state: &AppState,
    id: &str,
    text: &str,
    cmid: Option<&str>,
) -> Result<bool, String> {
    if !cmid.is_some_and(Hub::is_authored_cmid) || crate::session_context::is_command(text) {
        return Ok(false);
    }
    let Some(meta) = state.hub.session_info(id).map(|info| info.meta) else {
        return Ok(false);
    };
    let Some(idle) = state
        .hub
        .session_idle_for(id)
        .filter(|idle| *idle >= MEASURED_IDLE)
    else {
        return Ok(false);
    };
    let idle_ms = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX);
    let policy = policy(state, &meta).await;
    let skip = |outcome: &str| {
        measure(&meta, &policy, idle_ms, outcome);
        Ok(false)
    };
    if policy.idle_minutes.is_none() {
        return skip("no_preset");
    }
    if !policy.enabled {
        return skip("disabled");
    }
    if !policy.due(idle, u64::MAX) {
        return skip("cache_warm");
    }
    if meta.context_used < policy.min_context_tokens {
        return skip("small_context");
    }
    if !quiescent(state, &meta) {
        return skip("not_quiescent");
    }
    let lock = write_lock(id);
    let _write = lock.lock().await;
    let Ok(mut context) = crate::session_context::read(&state.hub, id) else {
        return skip("context_unreadable");
    };
    if context.revision != context.applied_revision
        || !context.waiting.is_empty()
        || context.next_agent.is_some()
        || crate::session_context::pending(&state.hub, id)
    {
        return skip("context_pending");
    }
    let Some(last) = state.hub.last_event_seq(id) else {
        return skip("no_history");
    };
    let rollover = Rollover {
        before: last.saturating_add(1),
        idle_ms,
        context_used: meta.context_used,
        digest: state.hub.rollover_digest(id).unwrap_or_default(),
    };
    let digest_bytes = serde_json::to_vec(&rollover.digest).map_or(0, |bytes| bytes.len());
    context.waiting.push(Link {
        session_id: id.to_owned(),
        before: rollover.before,
        title: meta.title.clone(),
    });
    context.rollover = Some(rollover);
    if let Err(error) = save(state, id, &context).await {
        tracing::warn!(session = id, %error, "context renewal not recorded; resuming the old context");
        return skip("record_failed");
    }
    tracing::info!(
        session = id,
        idle_ms,
        context_used = meta.context_used,
        digest_bytes,
        "renewing agent context after the Provider prompt cache expired"
    );
    measure(&meta, &policy, idle_ms, "renewed");
    state.hub.prepare_context_reset(id);
    state
        .hub
        .mark_context_rolled_over(id, idle_ms, meta.context_used);
    if !state.supervisor.has_live_worker(id) {
        return Ok(false);
    }
    state.supervisor.reset_session(id).map(|()| true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct OverrideRequest {
    /// `null` inherits the owner's default.
    enabled: Option<bool>,
}

async fn view(state: &AppState, id: &str) -> Response {
    let Some(meta) = state.hub.session_info(id).map(|info| info.meta) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let policy = policy(state, &meta).await;
    let idle_ms = state
        .hub
        .session_idle_for(id)
        .map(|idle| u64::try_from(idle.as_millis()).unwrap_or(u64::MAX));
    Json(serde_json::json!({
        "policy": policy,
        "context_used": meta.context_used,
        "idle_ms": idle_ms,
        "due": idle_ms.is_some_and(|idle| policy.due(std::time::Duration::from_millis(idle), meta.context_used))
            && quiescent(state, &meta),
    }))
    .into_response()
}

pub(in crate::server) async fn get(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    view(&state, &id).await
}

pub(in crate::server) async fn set(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<OverrideRequest>,
) -> Response {
    if state.hub.session_info(&id).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    state.hub.set_setting(
        override_key(&id),
        request
            .enabled
            .map_or(serde_json::Value::Null, serde_json::Value::Bool),
    );
    view(&state, &id).await
}

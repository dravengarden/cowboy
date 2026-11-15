//! Renew an idle session's agent context before a prompt reaches it, once the
//! Provider prompt cache has expired. See [`crate::context_rollover`].
//!
//! Renewal only ever replaces a dormant session's context: no worker exists
//! to race, the native id is forgotten under the Supervisor lifecycle lock
//! that every wake path holds, and the prompt continues its own dispatch.
//! A live worker idle past its cache is hibernated first (atomically refused
//! by its Machine if work started), so its next prompt takes the same path.
//! The marker is the commit point: the Supervisor attaches the renewal
//! preamble to whatever prompt first reaches the fresh context
//! ([`crate::session_context::renewal_preamble`]), so no prompt identity has
//! to be bound in advance.
use super::*;
use crate::context_rollover::{Policy, Rollover};

/// Prompts after at least this much idle are measured, renewed or not, so a
/// renewal's first turn can be compared with ordinary cold resumes.
const MEASURED_IDLE: std::time::Duration = std::time::Duration::from_mins(5);
/// Bound on prompts awaiting their first-turn measurement.
const MAX_PENDING: usize = 1_024;
/// How often live workers idle past their cache are hibernated.
const HIBERNATE_PASS: std::time::Duration = std::time::Duration::from_mins(5);

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

/// The effective policy, or `None` when the owner's preferences cannot be
/// read: an unknown choice is never taken as consent.
async fn policy(state: &AppState, meta: &crate::core::SessionMeta) -> Option<Policy> {
    let preferences = match (meta.owner_user_id.as_deref(), state.store.as_ref()) {
        (Some(owner), Some(store)) => store
            .preferences(owner, crate::context_rollover::PREFERENCE_SCOPE)
            .await
            .ok()?
            .map(|preferences| preferences.values),
        _ => None,
    };
    Some(crate::context_rollover::policy(
        preferences.as_ref(),
        session_override(&state.hub, &meta.id),
        &meta.provider,
    ))
}

/// Shared gate for renewal and hibernation: an ordinary, resumable session
/// with no background work. A native fork keeps inherited history only in its
/// native thread, so it is never renewed.
fn ordinary(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    !meta.system
        && !meta.closing
        && meta.agent_session_id.is_some()
        && meta.background_tasks == 0
        && !state.hub.is_managed_child(&meta.id)
        && state.hub.session_fork(&meta.id).is_none()
}

struct Pending {
    cmid: String,
    /// The measured prompt's own echo was seen; only then does a turn end
    /// belong to it.
    armed: bool,
    provider: String,
    renewed: bool,
    idle_ms: u64,
    context_used: u64,
    dispatched: std::time::Instant,
}

static PENDING: std::sync::LazyLock<parking_lot::Mutex<HashMap<String, Pending>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));

fn measure(
    meta: &crate::core::SessionMeta,
    cmid: &str,
    policy: Option<&Policy>,
    idle_ms: u64,
    outcome: &str,
) {
    tracing::info!(
        target: "cowboy::evidence",
        event_name = "cowboy.context_rollover.decision",
        session = %meta.id,
        provider = %meta.provider,
        outcome,
        idle_ms,
        context_used = meta.context_used,
        threshold_ms = policy
            .and_then(|policy| policy.idle_minutes)
            .map_or(0, |minutes| minutes.saturating_mul(60_000)),
        min_context = policy.map_or(0, |policy| policy.min_context_tokens),
        "context renewal decision"
    );
    let mut pending = PENDING.lock();
    // A retry of the same prompt keeps its first decision.
    if pending
        .get(&meta.id)
        .is_some_and(|known| known.cmid == cmid)
    {
        return;
    }
    if pending.len() >= MAX_PENDING {
        pending.clear();
    }
    pending.insert(
        meta.id.clone(),
        Pending {
            cmid: cmid.to_owned(),
            armed: false,
            provider: meta.provider.clone(),
            renewed: outcome == "renewed",
            idle_ms,
            context_used: meta.context_used,
            dispatched: std::time::Instant::now(),
        },
    );
}

fn first_turn(session: &str, usage: Option<&crate::agent_model::TurnUsage>) {
    let pending = {
        let mut pending = PENDING.lock();
        if !pending.get(session).is_some_and(|known| known.armed) {
            return;
        }
        pending.remove(session)
    };
    let Some(pending) = pending else {
        return;
    };
    tracing::info!(
        target: "cowboy::evidence",
        event_name = "cowboy.context_rollover.first_turn",
        session,
        provider = %pending.provider,
        renewed = pending.renewed,
        idle_ms = pending.idle_ms,
        context_used = pending.context_used,
        // An adapter that reports no reconciled usage still logs the turn.
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

/// Hibernate live workers whose cache has expired under their policy, so the
/// next prompt renews a dormant session. The Machine refuses atomically if
/// work started; a refusal is only logged.
async fn hibernate_expired(state: &AppState) {
    for meta in state.hub.session_list() {
        // Queued work (paused or held for editing) must stay runnable by
        // its auto-drain, which never revives a dormant session.
        if meta.status != Status::Running
            || !ordinary(state, &meta)
            || state.hub.session_has_in_flight_prompt(&meta.id)
            || state
                .hub
                .session_info(&meta.id)
                .is_none_or(|info| info.queue_count > 0)
            || !state.supervisor.has_live_worker(&meta.id)
        {
            continue;
        }
        let Some(idle) = state.hub.session_idle_for(&meta.id) else {
            continue;
        };
        let Some(policy) = policy(state, &meta).await else {
            continue;
        };
        if !policy.due(idle, meta.context_used) {
            continue;
        }
        let result = state.supervisor.hibernate_session(&meta.id);
        tracing::info!(
            target: "cowboy::evidence",
            event_name = "cowboy.context_rollover.hibernate",
            session = %meta.id,
            provider = %meta.provider,
            outcome = if result.is_ok() { "requested" } else { "refused" },
            idle_ms = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX),
            context_used = meta.context_used,
            "hibernating a worker whose prompt cache expired"
        );
    }
}

/// Record the first turn after a measured prompt (what the resume or renewal
/// actually cost) and periodically hibernate workers past their cache.
pub(in crate::server) async fn observe(state: Arc<AppState>) {
    let mut events = state.hub.subscribe();
    let mut pass = tokio::time::interval(HIBERNATE_PASS);
    pass.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let frame = tokio::select! {
            _ = pass.tick() => {
                hibernate_expired(&state).await;
                continue;
            }
            frame = events.recv() => frame,
        };
        let frame = match frame {
            Ok(frame) => frame,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let crate::core::Outbound::Event { envelope } = frame.outbound() else {
            continue;
        };
        match &envelope.event {
            crate::core::Event::TurnEnd { usage, .. } => {
                first_turn(&envelope.session_id, usage.as_ref());
            }
            crate::core::Event::Update { .. } => {
                if let Some(cmid) = envelope.cmid.as_deref()
                    && let Some(known) = PENDING.lock().get_mut(&envelope.session_id)
                    && known.cmid == cmid
                {
                    known.armed = true;
                }
            }
            _ => {}
        }
    }
}

/// Renew the context if this prompt reaches a dormant session after its
/// Provider cache expired. Never fails the prompt: anything short of a
/// recorded, atomic renewal leaves the session to resume its old context.
/// Authored prompts after [`MEASURED_IDLE`] log their decision.
pub(in crate::server) async fn before_dispatch(
    state: &AppState,
    id: &str,
    text: &str,
    cmid: Option<&str>,
) {
    let Some(cmid) = cmid.filter(|cmid| Hub::is_authored_cmid(cmid)) else {
        return;
    };
    if crate::session_context::is_command(text) {
        return;
    }
    let Some(meta) = state.hub.session_info(id).map(|info| info.meta) else {
        return;
    };
    // Serializes with context edits, which take the same lock.
    let lock = write_lock(id);
    let _write = lock.lock().await;
    let Ok(context) = crate::session_context::read(&state.hub, id) else {
        return;
    };
    let idle = state.hub.session_idle_for(id).unwrap_or_default();
    let idle_ms = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX);
    let measured = idle >= MEASURED_IDLE;
    let policy = policy(state, &meta).await;
    let skip = |outcome: &str| {
        if measured {
            measure(&meta, cmid, policy.as_ref(), idle_ms, outcome);
        }
    };
    let Some(policy) = policy.as_ref() else {
        return skip("preferences_unavailable");
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
    if !ordinary(state, &meta) {
        return skip("not_eligible");
    }
    if meta.status != Status::Exited || state.supervisor.has_live_worker(id) {
        return skip("live_worker");
    }
    if crate::session_context::pending(&state.hub, id) {
        return skip("context_pending");
    }
    let (Some(last), Some(agent_session_id)) =
        (state.hub.last_event_seq(id), meta.agent_session_id.clone())
    else {
        return skip("no_history");
    };
    let before = last.saturating_add(1);
    let mut renewed = context;
    renewed.rollover = Some(Rollover {
        before,
        idle_ms,
        context_used: meta.context_used,
        digest: state.hub.rollover_digest(id).unwrap_or_default(),
    });
    // Inert until the marker below names the same boundary.
    if let Err(error) = save(state, id, &renewed).await {
        tracing::warn!(session = id, %error, "context renewal not recorded; resuming the old context");
        return skip("record_failed");
    }
    if !state
        .supervisor
        .forget_dormant_context(id, &agent_session_id)
    {
        // A worker started meanwhile and keeps its context.
        return skip("raced");
    }
    state
        .hub
        .mark_context_rolled_over(id, before, idle_ms, meta.context_used);
    tracing::info!(
        session = id,
        idle_ms,
        context_used = meta.context_used,
        "renewing agent context after the Provider prompt cache expired"
    );
    if measured {
        measure(&meta, cmid, Some(policy), idle_ms, "renewed");
    }
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
    let Some(policy) = policy(state, &meta).await else {
        return (StatusCode::SERVICE_UNAVAILABLE, "preferences unavailable").into_response();
    };
    let idle_ms = state
        .hub
        .session_idle_for(id)
        .map(|idle| u64::try_from(idle.as_millis()).unwrap_or(u64::MAX));
    Json(serde_json::json!({
        "policy": policy,
        "context_used": meta.context_used,
        "idle_ms": idle_ms,
        "due": idle_ms.is_some_and(|idle| policy.due(std::time::Duration::from_millis(idle), meta.context_used))
            && ordinary(state, &meta),
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

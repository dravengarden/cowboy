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
use crate::logs::wait_cache::{self, Fields};

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
pub fn inherit(state: &AppState, source: &str, target: &str) {
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
        // A reading a compaction made history measures nothing current; the
        // renewal it would lead to skips it too.
        if meta.context_usage_stale()
            || !policy.due(idle, meta.context_used)
            || state
                .hub
                .session_info(&meta.id)
                .is_none_or(|info| info.queue_count > 0)
        {
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
pub async fn observe(state: Arc<AppState>) {
    tokio::spawn(super::super::background_tasks::wait_observation::observe(
        Arc::clone(&state),
    ));
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
/// Authored prompts after [`MEASURED_IDLE`] and every renewal log their
/// decision.
fn is_command_or_compaction(text: &str, content: &[serde_json::Value]) -> bool {
    crate::session_context::is_command(text)
        || crate::session_context::is_command_blocks(content)
        || crate::wrap_up::is_compaction_prompt_blocks(content)
}

pub async fn before_dispatch(
    state: &AppState,
    id: &str,
    text: &str,
    content: &[serde_json::Value],
    cmid: Option<&str>,
    dispatch_origin: &crate::prompt_origin::DeliveryOrigin,
) {
    if let Some(cmid) = cmid
        && cmid.starts_with(crate::agent_model::CALL_NOTICE_PREFIX)
        && let Some(info) = state.hub.session_info(id)
    {
        let used = state.hub.verified_context_usage(&info.meta);
        observe_decision(
            &info.meta,
            cmid,
            "notify_held",
            "notify_held",
            None,
            used,
            None,
        );
    }
    if !dispatch_origin.is_owner() {
        return;
    }
    let Some(cmid) = cmid else {
        return;
    };
    if is_command_or_compaction(text, content) {
        return;
    }
    let Some(meta) = state.hub.session_info(id).map(|info| info.meta) else {
        return;
    };
    // Serializes with context edits, which take the same lock.
    let lock = write_lock(id);
    let _write = lock.lock().await;
    let Ok(context) = crate::session_context::read(&state.hub, id) else {
        observe_decision(
            &meta,
            cmid,
            "context_unavailable",
            "context_unavailable",
            None,
            None,
            None,
        );
        return;
    };
    let idle = state.hub.session_idle_for(id).unwrap_or_default();
    let idle_ms = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX);
    let measured = idle >= MEASURED_IDLE;
    let policy = policy(state, &meta).await;
    let verified = state.hub.verified_context_usage(&meta);
    let observe = |outcome: &'static str| {
        let shadow = if outcome == "usage_unknown" {
            shadow_preflight(state, &meta, policy.as_ref(), verified)
        } else {
            outcome
        };
        observe_decision(
            &meta,
            cmid,
            outcome,
            shadow,
            policy.as_ref(),
            verified,
            Some(idle_ms),
        );
    };
    let skip = |outcome: &'static str| {
        if measured {
            measure(&meta, cmid, policy.as_ref(), idle_ms, outcome);
            observe(outcome);
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
    if (meta.context_used == 0 || meta.context_usage_stale()) && meta.agent_session_id.is_some() {
        // No current usage reading for a context that will be resumed (none
        // yet, or only the one a compaction made history): report it apart
        // from a measured small context, since its resume may be large.
        return skip("usage_unknown");
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
    let (digest, opening) = digest(state, id).await;
    let mut renewed = context;
    renewed.rollover = Some(Rollover {
        before,
        idle_ms,
        context_used: meta.context_used,
        digest,
    });
    // Inert until the marker below names the same boundary.
    if let Err(error) = save(state, id, &renewed).await {
        tracing::warn!(session = id, %error, "context renewal not recorded; resuming the old context");
        return skip("record_failed");
    }
    // The marker and the forgotten native id are published together under
    // the lifecycle lock, so no wake can launch between them.
    if !state.supervisor.forget_dormant_context(
        id,
        &agent_session_id,
        (before, idle_ms, meta.context_used),
    ) {
        // A worker started meanwhile and keeps its context.
        return skip("raced");
    }
    tracing::info!(
        session = id,
        idle_ms,
        context_used = meta.context_used,
        "renewing agent context after the Provider prompt cache expired"
    );
    // Every renewal is evidence, whatever its idle time.
    measure(&meta, cmid, Some(policy), idle_ms, "renewed");
    observe("renewed");
    if let Some(rollover) = &renewed.rollover {
        measure_digest(id, rollover, opening);
    }
}

/// Compare only the token gate. Passing it says nothing about dormancy,
/// permissions, policy or a lifecycle race. No dispatch reads these fields.
fn shadow_gate(used: Option<u64>, min: Option<u64>) -> &'static str {
    match used.zip(min) {
        Some((used, min)) if used >= min => "passes",
        Some(_) => "small_context",
        None => "usage_unknown",
    }
}

fn observe_decision(
    meta: &crate::core::SessionMeta,
    cmid: &str,
    outcome: &str,
    shadow_outcome: &str,
    policy: Option<&Policy>,
    verified: Option<u64>,
    idle_ms: Option<u64>,
) {
    let min = policy.map(|p| p.min_context_tokens);
    let current = ((meta.context_used != 0 || meta.agent_session_id.is_none())
        && !meta.context_usage_stale())
    .then_some(meta.context_used);
    let shadow = current.or(verified);
    wait_cache::record(
        wait_cache::DECISION,
        &[&meta.id, cmid],
        None,
        Fields::default()
            .string("cowboy.session.id", &meta.id)
            .string("provider", wait_cache::provider(&meta.provider))
            .string("outcome", outcome)
            .string("shadow_outcome", shadow_outcome)
            .integer("idle_ms", idle_ms)
            .integer("current_context_used", meta.context_used)
            .integer("verified_context_used", verified)
            .boolean("current_usage_reported", meta.usage.is_some())
            .integer(
                "threshold_ms",
                policy
                    .and_then(|p| p.idle_minutes)
                    .map(|minutes| minutes.saturating_mul(60_000)),
            )
            .integer("min_context_tokens", min)
            .string("current_context_gate", shadow_gate(current, min))
            .string("shadow_context_gate", shadow_gate(shadow, min)),
    );
}

fn shadow_preflight(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    policy: Option<&Policy>,
    verified: Option<u64>,
) -> &'static str {
    let gate = shadow_gate(verified, policy.map(|p| p.min_context_tokens));
    if gate != "passes" {
        return gate;
    }
    if !ordinary(state, meta) {
        return "not_eligible";
    }
    if meta.status != Status::Exited || state.supervisor.has_live_worker(&meta.id) {
        return "live_worker";
    }
    if crate::session_context::pending(&state.hub, &meta.id) {
        return "context_pending";
    }
    if state.hub.last_event_seq(&meta.id).is_none() {
        return "no_history";
    }
    "would_pass_preflight"
}

/// What the fresh context was handed in place of its history: the preamble
/// size against the cold resume it replaced, and whether each digest part
/// was present, so a later read of the earlier context can be attributed to
/// a missing part rather than to the question.
fn measure_digest(id: &str, rollover: &Rollover, opening: &'static str) {
    let digest = &rollover.digest;
    tracing::info!(
        target: "cowboy::evidence",
        event_name = "cowboy.context_rollover.digest",
        session = id,
        boundary = rollover.before,
        context_used = rollover.context_used,
        preamble_bytes = crate::context_rollover::preamble_body(id, rollover).len() as u64,
        first_request = opening,
        first_request_bytes = digest.first_request.as_ref().map_or(0, |text| text.len() as u64),
        recent_requests = digest.recent_requests.len() as u64,
        recent_requests_bytes = digest.recent_requests.iter().map(|text| text.len() as u64).sum::<u64>(),
        last_reply_bytes = digest.last_reply.as_ref().map_or(0, |text| text.len() as u64),
        "context renewal digest"
    );
}

/// The held-tail digest, with the opening request read back from the
/// durable transcript when a long session trimmed it from memory. A failed
/// read only leaves that excerpt out. Also names where the opening request
/// came from: `held`, `durable`, `durable_failed` or `absent`.
async fn digest(state: &AppState, id: &str) -> (crate::context_rollover::Digest, &'static str) {
    let Some((mut digest, cursor)) = state.hub.rollover_digest(id) else {
        return (crate::context_rollover::Digest::default(), "absent");
    };
    if digest.first_request.is_some() {
        return (digest, "held");
    }
    let (Some(cursor), Some(store)) = (cursor, state.store.as_ref()) else {
        return (digest, "absent");
    };
    match store.context_opening_prompts(id, cursor).await {
        Ok(prompts) => {
            digest.first_request = crate::context_rollover::opening_request(&prompts);
            let opening = if digest.first_request.is_some() {
                "durable"
            } else {
                "absent"
            };
            (digest, opening)
        }
        Err(error) => {
            tracing::warn!(session = id, %error, "renewal digest without its opening request");
            (digest, "durable_failed")
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverrideRequest {
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
        // A compaction left the reading as history: renewal treats it as unknown.
        "context_stale": meta.context_usage_stale(),
        "idle_ms": idle_ms,
        "due": !meta.context_usage_stale() && idle_ms.is_some_and(|idle| policy.due(std::time::Duration::from_millis(idle), meta.context_used))
            && ordinary(state, &meta),
    }))
    .into_response()
}

pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    view(&state, &id).await
}

pub async fn set(
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

#[cfg(test)]
mod tests {
    use super::shadow_gate;

    #[test]
    fn a_verified_zero_is_small_in_shadow_and_missing_stays_unknown() {
        assert_eq!(shadow_gate(None, Some(80_000)), "usage_unknown");
        assert_eq!(shadow_gate(Some(0), Some(80_000)), "small_context");
        assert_eq!(shadow_gate(Some(80_000), Some(80_000)), "passes");
    }
}

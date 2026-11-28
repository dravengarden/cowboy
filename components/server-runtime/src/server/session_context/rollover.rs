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
/// How often live workers idle past their cache are hibernated. A 1-minute
/// cadence keeps the post-expiry `live_worker` skip window bounded without
/// hibernating warm sessions prematurely.
const HIBERNATE_PASS: std::time::Duration = std::time::Duration::from_mins(1);

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

fn session_restricted(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    crate::provider_account::named_only(&meta.provider)
        || state.hub.session_fork(&meta.id).is_some()
}

/// Shared reader/writer fence serializing global & per-session rollover
/// policy mutations against the hibernation sweep's post-RPC read-through-enqueue.
pub(crate) fn policy_lock() -> &'static tokio::sync::RwLock<()> {
    static LOCK: std::sync::LazyLock<tokio::sync::RwLock<()>> =
        std::sync::LazyLock::new(|| tokio::sync::RwLock::new(()));
    &LOCK
}

/// A continuation keeps the source's explicit choice, except an enabling
/// override on a restricted source or target is never inherited.
pub fn inherit(state: &AppState, source: &str, target: &str) {
    let Some(enabled) = session_override(&state.hub, source) else {
        return;
    };
    if enabled {
        let source_restricted = state
            .hub
            .session_info(source)
            .is_some_and(|info| session_restricted(state, &info.meta));
        let target_restricted = state
            .hub
            .session_info(target)
            .is_some_and(|info| session_restricted(state, &info.meta));
        if source_restricted || target_restricted {
            return;
        }
    }
    state
        .hub
        .set_setting(override_key(target), serde_json::Value::Bool(enabled));
    state.hub.bump_rollover_policy_epoch();
}

/// The effective policy, or `None` when the owner's preferences cannot be
/// read: an unknown choice is never taken as consent.
pub(crate) async fn effective_policy(
    state: &AppState,
    meta: &crate::core::SessionMeta,
) -> Option<Policy> {
    policy(state, meta).await
}

async fn policy_with_raw(
    state: &AppState,
    meta: &crate::core::SessionMeta,
) -> Option<(Policy, Option<serde_json::Value>)> {
    let preferences = match (meta.owner_user_id.as_deref(), state.store.as_ref()) {
        (Some(owner), Some(store)) => store
            .preferences(owner, crate::context_rollover::PREFERENCE_SCOPE)
            .await
            .ok()?
            .map(|preferences| preferences.values),
        _ => None,
    };
    let resolved = crate::context_rollover::policy(
        preferences.as_ref(),
        session_override(&state.hub, &meta.id),
        &meta.provider,
    );
    Some((resolved, preferences))
}

async fn policy(state: &AppState, meta: &crate::core::SessionMeta) -> Option<Policy> {
    policy_with_raw(state, meta).await.map(|(policy, _)| policy)
}

/// Shared gate for renewal and hibernation: an ordinary, resumable session
/// with no background work. A native fork keeps inherited history only in its
/// native thread, so it is never renewed.
pub(crate) fn is_ordinary_session(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    ordinary(state, meta)
}

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

/// Bound on concurrent gateway keepalive probes during one hibernation sweep.
const HIBERNATE_SWEEP_CONCURRENCY: usize = 4;

enum KeepaliveHandoffCheck {
    NotConfigured,
    Inactive(crate::supervisor::KeepaliveRuntimeProof),
    ActiveOrUnavailable,
}

fn keepalive_status_is_inactive(status: &serde_json::Value) -> bool {
    status.get("state").and_then(serde_json::Value::as_str) == Some("inactive")
}

/// Fail-closed check for two-stage keepalive -> idle-renewal handoff: binds
/// the host-layer hibernation-epoch capability check, the gateway
/// `provider-cache-status` RPC, and the subsequent hibernation command to the
/// exact authenticated Machine connection token attached to the runtime.
async fn verify_keepalive_handoff(state: &AppState, session_id: &str) -> KeepaliveHandoffCheck {
    let probe = match state.supervisor.capture_keepalive_probe(session_id) {
        Ok(Some(probe)) => probe,
        Ok(None) => return KeepaliveHandoffCheck::NotConfigured,
        Err(_) => return KeepaliveHandoffCheck::ActiveOrUnavailable,
    };
    if !state
        .machine_control
        .connection_supports_hibernation_epoch(probe.connection())
    {
        tracing::debug!(
            session = %session_id,
            machine = %probe.machine_id(),
            "skipping keepalive handoff: Machine predates epoch-bound hibernation"
        );
        return KeepaliveHandoffCheck::ActiveOrUnavailable;
    }
    match state
        .machine_control
        .adapter_request_on_connection(
            probe.connection(),
            "provider-cache-status",
            serde_json::json!({
                "configuration": probe.configuration(),
                "sessionId": session_id,
            }),
        )
        .await
    {
        Ok(status) if keepalive_status_is_inactive(&status) => {
            if state
                .machine_control
                .connection_supports_hibernation_epoch(probe.connection())
            {
                KeepaliveHandoffCheck::Inactive(probe)
            } else {
                KeepaliveHandoffCheck::ActiveOrUnavailable
            }
        }
        Ok(_) => KeepaliveHandoffCheck::ActiveOrUnavailable,
        Err(error) => {
            tracing::debug!(
                session = %session_id,
                machine = %probe.machine_id(),
                %error,
                "skipping idle hibernation: could not verify keepalive inactive state"
            );
            KeepaliveHandoffCheck::ActiveOrUnavailable
        }
    }
}

async fn evaluate_hibernate_candidate(state: &AppState, meta: crate::core::SessionMeta) {
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
        return;
    }
    if crate::provider_account::named_only(&meta.provider) {
        return;
    }
    let Some(idle) = state.hub.session_idle_for(&meta.id) else {
        return;
    };
    // Minimum configurable idle_minutes is 1; skip active sessions before
    // touching the preference store on the 1-minute sweep.
    if idle < std::time::Duration::from_mins(1) {
        return;
    }
    let Some(initial_policy) = policy(state, &meta).await else {
        return;
    };
    if initial_policy
        .idle_minutes
        .filter(|_| initial_policy.enabled)
        .is_none()
    {
        return;
    }
    let Some(pre_info) = state.hub.session_info(&meta.id) else {
        return;
    };
    let Some(pre_idle) = state.hub.session_idle_for(&meta.id) else {
        return;
    };
    if pre_info.meta.context_usage_stale()
        || !initial_policy.due(pre_idle, pre_info.meta.context_used)
        || pre_info.queue_count > 0
    {
        return;
    }
    let (keepalive_proof, keepalive_handoff) = match verify_keepalive_handoff(state, &meta.id).await
    {
        KeepaliveHandoffCheck::NotConfigured => (None, false),
        KeepaliveHandoffCheck::Inactive(proof) => (Some(proof), true),
        KeepaliveHandoffCheck::ActiveOrUnavailable => return,
    };
    // Hold the shared policy read guard from post-RPC policy re-read through
    // command enqueue so concurrent preference or override writes cannot race.
    let _policy_read = policy_lock().read().await;
    let policy_epoch = state.hub.rollover_policy_epoch();
    let Some(fresh_info) = state.hub.session_info(&meta.id) else {
        return;
    };
    let Some(fresh_policy) = policy(state, &fresh_info.meta).await else {
        return;
    };
    let Some(idle_minutes) = fresh_policy.idle_minutes.filter(|_| fresh_policy.enabled) else {
        return;
    };
    let min_idle = std::time::Duration::from_mins(idle_minutes);
    let Some(fresh_idle) = state.hub.session_idle_for(&meta.id) else {
        return;
    };
    if fresh_info.meta.context_usage_stale()
        || !fresh_policy.due(fresh_idle, fresh_info.meta.context_used)
        || fresh_info.queue_count > 0
    {
        return;
    }
    // Serialize keepalive-handoff connection validation and command enqueue
    // against Machine reconnect registration so a retired connection cannot
    // race between `machine_control.install` and `runtime_router.install_if`.
    let _reg_guard = if let Some(proof) = keepalive_proof.as_ref() {
        Some(super::super::machine_handshake::machine_registration_lock(proof.machine_id()).await)
    } else {
        None
    };
    let result = state.supervisor.hibernate_idle_session(
        &meta.id,
        min_idle,
        fresh_policy.min_context_tokens,
        policy_epoch,
        keepalive_proof.as_ref(),
        |connection| {
            state
                .machine_control
                .connection_supports_hibernation_epoch(connection)
        },
    );
    tracing::info!(
        target: "cowboy::evidence",
        event_name = "cowboy.context_rollover.hibernate",
        session = %meta.id,
        provider = %meta.provider,
        outcome = if result.is_ok() { "requested" } else { "refused" },
        keepalive_handoff,
        idle_ms = u64::try_from(fresh_idle.as_millis()).unwrap_or(u64::MAX),
        context_used = fresh_info.meta.context_used,
        "hibernating a worker whose prompt cache expired"
    );
}

/// Hibernate live workers whose cache has expired under their policy, so the
/// next prompt renews a dormant session. Bounded concurrency ensures one slow
/// gateway RPC never delays unrelated sessions.
async fn hibernate_expired(state: &AppState) {
    use futures::StreamExt as _;
    let sessions = state.hub.session_list();
    futures::stream::iter(sessions)
        .for_each_concurrent(HIBERNATE_SWEEP_CONCURRENCY, |meta| {
            evaluate_hibernate_candidate(state, meta)
        })
        .await;
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Record the first turn after a measured prompt (what the resume or renewal
/// actually cost) and run the periodic hibernation sweep on an independent
/// abort-on-drop task so gateway RPC timeouts never lag turn-event observation
/// and server shutdown cancels in-flight sweeps cleanly.
pub async fn observe(state: Arc<AppState>) {
    let _wait_guard = AbortOnDrop(tokio::spawn(
        super::super::background_tasks::wait_observation::observe(Arc::clone(&state)),
    ));
    let sweep_state = Arc::clone(&state);
    let _sweep_guard = AbortOnDrop(tokio::spawn(async move {
        let mut pass = tokio::time::interval(HIBERNATE_PASS);
        pass.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            pass.tick().await;
            hibernate_expired(&sweep_state).await;
        }
    }));
    let mut events = state.hub.subscribe();
    loop {
        let frame = events.recv().await;
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
fn is_command_or_compaction(_text: &str, content: &[serde_json::Value]) -> bool {
    crate::session_context::is_command_blocks(content)
        || crate::wrap_up::is_compaction_prompt_blocks(content)
}

/// Prompts eligible for idle context renewal: direct owner prompts and
/// scheduled drafts with verified Owner authorship (`origin == Some(Owner)` and
/// `trigger == Cowboy("schedule")`). Agent `ScheduleWakeup`s (`__wake__` /
/// non-owner origin), legacy drafts with unknown authorship (`origin == None`),
/// call notices (`call-notice:`), managed calls, superagent prompts, peer
/// prompts, legacy queue rows, and automation API callers stay excluded.
fn allows_rollover(
    origin: Option<&crate::prompt_origin::DeliveryOrigin>,
    trigger: &crate::prompt_origin::DeliveryOrigin,
    dispatch_origin: &crate::prompt_origin::DeliveryOrigin,
    cmid: &str,
) -> bool {
    if cmid.starts_with(crate::agent_model::CALL_NOTICE_PREFIX)
        || cmid.starts_with(crate::agent_model::WAKEUP_PREFIX)
    {
        return false;
    }
    if dispatch_origin.is_owner() {
        return true;
    }
    origin.is_some_and(crate::prompt_origin::DeliveryOrigin::is_owner)
        && matches!(
            trigger,
            crate::prompt_origin::DeliveryOrigin::Cowboy { source } if source == "schedule"
        )
}

#[allow(clippy::too_many_arguments)]
pub async fn before_dispatch(
    state: &AppState,
    id: &str,
    text: &str,
    content: &[serde_json::Value],
    cmid: Option<&str>,
    origin: Option<&crate::prompt_origin::DeliveryOrigin>,
    trigger: &crate::prompt_origin::DeliveryOrigin,
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
            None,
            used,
            None,
        );
        return;
    }
    let Some(cmid) = cmid else {
        return;
    };
    if !allows_rollover(origin, trigger, dispatch_origin, cmid) {
        return;
    }
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
            None,
        );
        return;
    };
    let idle = state.hub.session_idle_for(id).unwrap_or_default();
    let idle_ms = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX);
    let measured = idle >= MEASURED_IDLE;
    let resolved = policy_with_raw(state, &meta).await;
    let policy = resolved.as_ref().map(|(p, _)| p.clone());
    let raw_preferences = resolved.and_then(|(_, raw)| raw);
    let verified = state.hub.verified_context_usage(&meta);
    let observe = |outcome: &'static str| {
        let (shadow, shadow_threshold_ms) = evaluate_shadow_decision(
            state,
            &meta,
            outcome,
            policy.as_ref(),
            raw_preferences.as_ref(),
            idle,
            verified,
        );
        observe_decision(
            &meta,
            cmid,
            outcome,
            shadow,
            policy.as_ref(),
            shadow_threshold_ms,
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
    match commit_rollover(
        state,
        &meta,
        context,
        &agent_session_id,
        last.saturating_add(1),
        idle_ms,
    )
    .await
    {
        Ok((rollover, opening)) => {
            // Every renewal is evidence, whatever its idle time.
            measure(&meta, cmid, Some(policy), idle_ms, "renewed");
            observe("renewed");
            measure_digest(id, &rollover, opening);
        }
        Err(reason) => skip(reason),
    }
}

async fn commit_rollover(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    mut context: crate::session_context::Context,
    agent_session_id: &str,
    before: u64,
    idle_ms: u64,
) -> Result<(Rollover, &'static str), &'static str> {
    let id = &meta.id;
    let (digest, opening) = digest(state, id).await;
    let rollover = Rollover {
        before,
        idle_ms,
        context_used: meta.context_used,
        digest,
    };
    context.rollover = Some(rollover.clone());
    // Inert until the marker below names the same boundary.
    if let Err(error) = save(state, id, &context).await {
        tracing::warn!(session = id, %error, "context renewal not recorded; resuming the old context");
        return Err("record_failed");
    }
    // The marker and the forgotten native id are published together under
    // the lifecycle lock, so no wake can launch between them.
    if !state.supervisor.forget_dormant_context(
        id,
        agent_session_id,
        (before, idle_ms, meta.context_used),
    ) {
        // A worker started meanwhile and keeps its context.
        return Err("raced");
    }
    tracing::info!(
        session = id,
        idle_ms,
        context_used = meta.context_used,
        "renewing agent context after the Provider prompt cache expired"
    );
    Ok((rollover, opening))
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

/// Evaluate observational shadow outcome without mutating dispatch behavior.
/// When a Provider has no active preset (`no_preset`) and has not been
/// explicitly disabled by owner/session settings, evaluates its empirical
/// shadow candidate threshold (e.g. Codex 240m, DeepSeek 360m). Host-policy-
/// restricted Providers (`antigravity`) always remain `no_preset`.
/// Pure candidate-threshold decision before checking runtime dormancy/history.
/// Returns `Ok((candidate_policy, candidate_ms))` when a shadow candidate is due
/// and ready for `shadow_preflight`, or `Err((outcome, threshold_ms))` otherwise.
fn shadow_candidate_stage(
    provider: &str,
    policy: Option<&Policy>,
    raw_preferences: Option<&serde_json::Value>,
    idle: std::time::Duration,
) -> Result<(Policy, u64), (&'static str, Option<u64>)> {
    let Some(candidate_minutes) = crate::context_rollover::shadow_candidate_idle_minutes(provider)
    else {
        return Err(("no_preset", None));
    };
    let candidate_ms = candidate_minutes.saturating_mul(60_000);
    let enabled_default = raw_preferences
        .and_then(|values| values.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let session_opt_out = policy.is_some_and(|p| p.session_override == Some(false));
    let provider_opt_out =
        crate::context_rollover::explicit_provider_disabled(raw_preferences, provider);
    if !enabled_default || session_opt_out || provider_opt_out {
        return Err(("disabled", Some(candidate_ms)));
    }
    let candidate_policy = Policy {
        enabled: true,
        session_override: policy.and_then(|p| p.session_override),
        idle_minutes: Some(candidate_minutes),
        min_context_tokens: policy
            .map_or(crate::context_rollover::DEFAULT_MIN_CONTEXT_TOKENS, |p| {
                p.min_context_tokens
            }),
    };
    if !candidate_policy.due(idle, u64::MAX) {
        return Err(("cache_warm", Some(candidate_ms)));
    }
    Ok((candidate_policy, candidate_ms))
}

fn evaluate_shadow_decision(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    outcome: &'static str,
    policy: Option<&Policy>,
    raw_preferences: Option<&serde_json::Value>,
    idle: std::time::Duration,
    verified: Option<u64>,
) -> (&'static str, Option<u64>) {
    if outcome == "usage_unknown" {
        return (
            shadow_preflight(state, meta, policy, verified),
            policy
                .and_then(|p| p.idle_minutes)
                .map(|minutes| minutes.saturating_mul(60_000)),
        );
    }
    if outcome != "no_preset" {
        return (outcome, None);
    }
    let (candidate_policy, candidate_ms) =
        match shadow_candidate_stage(&meta.provider, policy, raw_preferences, idle) {
            Ok(ready) => ready,
            Err(early) => return early,
        };
    let current = ((meta.context_used != 0 || meta.agent_session_id.is_none())
        && !meta.context_usage_stale())
    .then_some(meta.context_used);
    (
        shadow_preflight(state, meta, Some(&candidate_policy), current.or(verified)),
        Some(candidate_ms),
    )
}

#[allow(clippy::too_many_arguments)]
fn observe_decision(
    meta: &crate::core::SessionMeta,
    cmid: &str,
    outcome: &str,
    shadow_outcome: &str,
    policy: Option<&Policy>,
    shadow_threshold_ms: Option<u64>,
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
            .integer("shadow_threshold_ms", shadow_threshold_ms)
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
    let policy_restricted = crate::provider_account::named_only(&meta.provider);
    let native_fork = state.hub.session_fork(&meta.id).is_some();
    let restriction = if policy_restricted {
        Some("Automatic context renewal is disabled for Antigravity accounts by host policy.")
    } else if native_fork {
        Some(
            "Automatic context renewal is excluded for native fork sessions because inherited history lives only in the native thread.",
        )
    } else {
        None
    };
    let dormant = state.supervisor.is_dormant_for_renewal(&meta.id);
    let pending_work = state.hub.session_info(&meta.id).is_some_and(|info| {
        info.queue_count > 0
            || info.meta.background_tasks > 0
            || info.meta.attention.approvals > 0
            || info.meta.attention.questions > 0
            || info.meta.attention.other > 0
    });
    let keepalive_handoff_supported = state
        .machine_control
        .supports_hibernation_epoch(&meta.machine_id);
    Json(serde_json::json!({
        "policy": policy,
        "context_used": meta.context_used,
        // A compaction left the reading as history: renewal treats it as unknown.
        "context_stale": meta.context_usage_stale(),
        "idle_ms": idle_ms,
        "due": !meta.context_usage_stale() && idle_ms.is_some_and(|idle| policy.due(std::time::Duration::from_millis(idle), meta.context_used))
            && ordinary(state, &meta),
        "dormant": dormant,
        "pending_work": pending_work,
        "keepalive_handoff_supported": keepalive_handoff_supported,
        "recommended_idle_minutes": crate::context_rollover::recommended_idle_minutes(&meta.provider),
        "policy_restricted": policy_restricted || native_fork,
        "restriction": restriction,
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
    let Some(meta) = state.hub.session_info(&id).map(|info| info.meta) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if request.enabled == Some(true) && session_restricted(&state, &meta) {
        let message = if crate::provider_account::named_only(&meta.provider) {
            "Automatic context renewal is disabled for Antigravity accounts by host policy."
        } else {
            "Automatic context renewal is excluded for native fork sessions."
        };
        return (StatusCode::UNPROCESSABLE_ENTITY, message).into_response();
    }
    {
        let _policy_write = policy_lock().write().await;
        state.hub.set_setting(
            override_key(&id),
            request
                .enabled
                .map_or(serde_json::Value::Null, serde_json::Value::Bool),
        );
        state.hub.bump_rollover_policy_epoch();
    }
    view(&state, &id).await
}

#[cfg(test)]
mod tests {
    use super::{
        Policy, allows_rollover, keepalive_status_is_inactive, shadow_candidate_stage, shadow_gate,
    };
    use crate::prompt_origin::DeliveryOrigin;
    use std::time::Duration;

    #[test]
    fn a_verified_zero_is_small_in_shadow_and_missing_stays_unknown() {
        assert_eq!(shadow_gate(None, Some(80_000)), "usage_unknown");
        assert_eq!(shadow_gate(Some(0), Some(80_000)), "small_context");
        assert_eq!(shadow_gate(Some(80_000), Some(80_000)), "passes");
    }

    #[test]
    fn rollover_allows_owner_and_verified_owner_scheduled_drafts_only() {
        let schedule = DeliveryOrigin::cowboy("schedule");
        // Direct owner prompt: allowed.
        assert!(allows_rollover(
            Some(&DeliveryOrigin::Owner),
            &DeliveryOrigin::Owner,
            &DeliveryOrigin::Owner,
            "user-1"
        ));
        // Verified owner-authored scheduled draft: allowed.
        assert!(allows_rollover(
            Some(&DeliveryOrigin::Owner),
            &schedule,
            &schedule,
            "sched-sess-1-draft-1"
        ));
        // Agent ScheduleWakeup (`__wake__` and non-owner origin): denied.
        assert!(!allows_rollover(
            Some(&schedule),
            &schedule,
            &schedule,
            "__wake__sess-1-123"
        ));
        assert!(!allows_rollover(
            Some(&DeliveryOrigin::Owner),
            &schedule,
            &schedule,
            "__wake__sess-1-123"
        ));
        // Legacy scheduled draft with unknown authorship (`origin == None`): denied.
        assert!(!allows_rollover(
            None,
            &schedule,
            &schedule,
            "sched-sess-1-legacy"
        ));
        // Call notices: always denied.
        assert!(!allows_rollover(
            Some(&DeliveryOrigin::Owner),
            &DeliveryOrigin::Owner,
            &DeliveryOrigin::Owner,
            "call-notice:call-123"
        ));
        assert!(!allows_rollover(
            Some(&DeliveryOrigin::Owner),
            &schedule,
            &schedule,
            "call-notice:call-123"
        ));
        // Non-owner origins (direct or scheduled): denied.
        for origin in [
            DeliveryOrigin::Automation,
            DeliveryOrigin::peer("peer-sess"),
            DeliveryOrigin::cowboy("managed_call"),
            DeliveryOrigin::cowboy("superagent"),
            DeliveryOrigin::cowboy("legacy_queue"),
        ] {
            assert!(
                !allows_rollover(Some(&origin), &origin, &origin, "msg-1"),
                "unexpected direct allow: {origin:?}"
            );
            assert!(
                !allows_rollover(Some(&origin), &schedule, &origin, "sched-1"),
                "unexpected scheduled allow: {origin:?}"
            );
        }
    }

    #[test]
    fn shadow_candidate_stage_evaluates_codex_and_deepseek_and_respects_opt_outs_and_antigravity() {
        let base_policy = Policy {
            enabled: false,
            session_override: None,
            idle_minutes: None,
            min_context_tokens: 80_000,
        };
        // Codex: warm below 240m, due at 240m.
        assert_eq!(
            shadow_candidate_stage("codex", Some(&base_policy), None, Duration::from_mins(120)),
            Err(("cache_warm", Some(240 * 60_000)))
        );
        let (codex_due, codex_ms) =
            shadow_candidate_stage("codex", Some(&base_policy), None, Duration::from_mins(240))
                .expect("codex shadow due at 240m");
        assert_eq!(codex_ms, 240 * 60_000);
        assert_eq!(codex_due.idle_minutes, Some(240));
        assert_eq!(codex_due.min_context_tokens, 80_000);

        // DeepSeek: warm at 300m, due at 360m.
        assert_eq!(
            shadow_candidate_stage(
                "claude-deepseek",
                Some(&base_policy),
                None,
                Duration::from_mins(300)
            ),
            Err(("cache_warm", Some(360 * 60_000)))
        );
        assert!(
            shadow_candidate_stage(
                "claude-deepseek",
                Some(&base_policy),
                None,
                Duration::from_mins(360)
            )
            .is_ok()
        );

        // Explicit opt-outs (global, per-provider null, or session override false) disable shadow.
        let global_off = serde_json::json!({"enabled": false});
        assert_eq!(
            shadow_candidate_stage(
                "codex",
                Some(&base_policy),
                Some(&global_off),
                Duration::from_mins(300)
            ),
            Err(("disabled", Some(240 * 60_000)))
        );
        let provider_null = serde_json::json!({"idle_minutes": {"codex": null}});
        assert_eq!(
            shadow_candidate_stage(
                "codex",
                Some(&base_policy),
                Some(&provider_null),
                Duration::from_mins(300)
            ),
            Err(("disabled", Some(240 * 60_000)))
        );
        let session_off = Policy {
            session_override: Some(false),
            ..base_policy.clone()
        };
        assert_eq!(
            shadow_candidate_stage("codex", Some(&session_off), None, Duration::from_mins(300)),
            Err(("disabled", Some(240 * 60_000)))
        );

        // Antigravity and Grok stay no_preset in shadow.
        for provider in ["antigravity", "grok"] {
            assert_eq!(
                shadow_candidate_stage(
                    provider,
                    Some(&base_policy),
                    None,
                    Duration::from_mins(600)
                ),
                Err(("no_preset", None)),
                "{provider}"
            );
        }
    }

    #[test]
    fn keepalive_status_is_inactive_fails_closed_for_protected_or_unknown_payloads() {
        assert!(keepalive_status_is_inactive(&serde_json::json!({
            "state": "inactive",
            "algorithm": "adaptive-replay-v1"
        })));
        assert!(!keepalive_status_is_inactive(&serde_json::json!({
            "state": "protected",
            "attempts": 2
        })));
        assert!(!keepalive_status_is_inactive(&serde_json::json!({
            "state": "disabled"
        })));
        assert!(!keepalive_status_is_inactive(&serde_json::json!({})));
    }
}

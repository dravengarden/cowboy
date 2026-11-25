//! Controller handler for `POST /api/machine/post-edit-inspect` (Decision 2A / R4).
//!
//! Authenticates the calling runtime Machine via its per-primary-connection
//! secret token (`x-cowboy-machine-post-edit-token`) BEFORE reading body bytes,
//! enforces per-target bounded concurrency (`POST_EDIT_TARGET_CONCURRENCY = 4`
//! held through target settlement, R3-H5), shares per-`(SessionCodeScope, path)`
//! locking with public `code.diagnostics`/`code.post_edit` (H-Ctrl-4), rechecks
//! deadline-bounded async owner/superagent authorization and synchronous live
//! worker/turn/trust/scope/caller/deadline fences immediately before every target
//! read and before the sole synchronous baseline commit.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use parking_lot::Mutex;
use tokio::sync::{Semaphore, watch};

use super::super::super::code_reads::{language, session};
use super::super::super::{
    AnalysisState, AppState, CodeReadScope, SourceState, ZedAdapterResponse, agent_api_bridge,
    execution, machine_post_edit, superagent,
};
use crate::core::LanguageServerTrust;
use crate::machine_control::{ConnectionToken, SessionReadScope};
use crate::post_edit_inspect::{
    MAX_POST_EDIT_REQUEST_BYTES, MIN_POST_EDIT_COLD_START_MS, POST_EDIT_INSPECT_TOKEN_HEADER,
    PostEditInspectOutcome, PostEditInspectReply, PostEditInspectRequest, clamp_remaining_ms,
};

const POST_EDIT_TARGET_CONCURRENCY: usize = 4;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

static TARGET_SLOTS: LazyLock<Mutex<HashMap<String, Arc<Semaphore>>>> =
    LazyLock::new(Mutex::default);

fn target_semaphore(machine_id: &str) -> Arc<Semaphore> {
    let mut map = TARGET_SLOTS.lock();
    Arc::clone(
        map.entry(machine_id.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(POST_EDIT_TARGET_CONCURRENCY))),
    )
}

/// Signals caller HTTP disconnection when dropped (e.g. client timeout or close).
struct CallerDisconnectGuard(watch::Sender<bool>);

impl Drop for CallerDisconnectGuard {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

fn reply_response(reply: PostEditInspectReply) -> Response {
    match serde_json::to_vec(&reply) {
        Ok(bytes) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn log_canary_refusal(request: &PostEditInspectRequest, reason: &str) {
    if let Some(canary_id) = &request.canary_inspect_id {
        tracing::info!(
            event_name = "cowboy.code.post_edit_inspect",
            session_id = %request.session_id,
            canary_inspect_id = %canary_id,
            path = %request.path,
            outcome = "refused",
            reason = %reason,
            "canary post-edit inspection refused"
        );
    }
}

fn refused_reply(request: &PostEditInspectRequest, reason: &'static str) -> PostEditInspectReply {
    log_canary_refusal(request, reason);
    PostEditInspectReply::refused(
        request.session_id.clone(),
        request.worker_epoch.clone(),
        request.request_id.clone(),
        request.scope.clone(),
        reason,
    )
}

struct AuthorizedContext {
    observed: SessionReadScope,
    trust: LanguageServerTrust,
}

/// Synchronous zero-await check verifying caller liveness, runtime connection,
/// worker/turn/scope, language-server trust, target SessionReadScope, and deadline.
#[allow(clippy::too_many_arguments)]
fn verify_sync_fence(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    observed: &SessionReadScope,
    trust: &LanguageServerTrust,
    caller_gone: &watch::Receiver<bool>,
    deadline: Instant,
) -> Result<(), &'static str> {
    if *caller_gone.borrow() {
        return Err("unavailable");
    }
    if Instant::now() >= deadline {
        return Err("timeout");
    }
    if !state.machine_control.is_current(runtime_connection) {
        return Err("unavailable");
    }
    if state.execution_closures.lock().get(&request.session_id)
        == Some(&execution::Maintenance::Close)
    {
        return Err("unavailable");
    }
    let Some(info) = state.hub.session_info(&request.session_id) else {
        return Err("permission_denied");
    };
    if info.meta.system
        || info.meta.closing
        || state.hub.is_managed_child(&request.session_id)
        || info.meta.machine_id != runtime_machine_id
    {
        return Err("restricted");
    }
    let binding_ok = info
        .meta
        .execution_binding
        .as_ref()
        .and_then(|binding| binding.for_runtime(runtime_machine_id, &info.meta.cwd).ok())
        .is_some_and(|binding| {
            binding.managed.is_none()
                && request.scope.matches_parts(
                    &binding.id,
                    binding.revision,
                    &binding.environment.incarnation,
                )
        });
    if !binding_ok {
        return Err("unavailable");
    }
    let runtime_ok = state
        .runtime_router
        .runtime(runtime_machine_id)
        .is_some_and(|runtime| {
            runtime.post_edit_owner_matches(
                &request.session_id,
                &request.worker_epoch,
                &request.turn_id,
                &request.scope,
            )
        });
    if !runtime_ok {
        return Err("unavailable");
    }
    if !trust.current(&state.hub) {
        return Err("restricted");
    }
    if !session::current(&state.hub, &state.machine_control, observed) {
        return Err("unavailable");
    }
    Ok(())
}

/// Deadline-bounded async owner + superagent authorization followed immediately
/// by the zero-await synchronous fence (R3-H1, M-Ctrl-6).
#[allow(clippy::too_many_arguments)]
async fn authorize_step(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    observed: &SessionReadScope,
    trust: &LanguageServerTrust,
    caller_gone: &watch::Receiver<bool>,
    deadline: Instant,
) -> Result<(), &'static str> {
    if *caller_gone.borrow() {
        return Err("unavailable");
    }
    if Instant::now() >= deadline {
        return Err("timeout");
    }
    let Some(info) = state.hub.session_info(&request.session_id) else {
        return Err("permission_denied");
    };
    if info.meta.system || info.meta.closing || state.hub.is_managed_child(&request.session_id) {
        return Err("restricted");
    }
    let mut gone = caller_gone.clone();
    race_caller_gone(
        &mut gone,
        deadline,
        agent_api_bridge::read_parent(&state.hub, state.store.as_ref(), &info.meta),
    )
    .await?
    .map_err(|_| "permission_denied")?;
    let is_restricted = race_caller_gone(
        &mut gone,
        deadline,
        superagent::restricted(state, &info.meta),
    )
    .await?
    .map_err(|_| "unavailable")?;
    if is_restricted {
        return Err("superagent_scope_denied");
    }
    verify_sync_fence(
        state,
        runtime_connection,
        runtime_machine_id,
        request,
        observed,
        trust,
        caller_gone,
        deadline,
    )
}

async fn resolve_and_authorize_initial(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    caller_gone: &watch::Receiver<bool>,
    deadline: Instant,
) -> Result<AuthorizedContext, &'static str> {
    let Some(scope) = session::resolve(
        &state.hub,
        &state.machine_control,
        &state.service_id,
        &request.session_id,
    ) else {
        return Err("unavailable");
    };
    let CodeReadScope::Session(observed) = scope else {
        return Err("unavailable");
    };
    // R4-H1: Require a target workspace Machine that registered post-edit
    // support (enforcing Machine-owned `"zed_post_edit"` concurrency slots).
    if !observed
        .connection()
        .is_some_and(machine_post_edit::supports_connection)
    {
        return Err("unsupported");
    }
    let Some(trust) = state
        .hub
        .observe_language_server_trust(&request.session_id)
        .filter(|trust| trust.scope() == observed.session())
    else {
        return Err("restricted");
    };
    authorize_step(
        state,
        runtime_connection,
        runtime_machine_id,
        request,
        &observed,
        &trust,
        caller_gone,
        deadline,
    )
    .await?;
    Ok(AuthorizedContext { observed, trust })
}

fn remaining_ms_until(deadline: Instant) -> u64 {
    u64::try_from(
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn is_changed(reply: &anyhow::Result<ZedAdapterResponse>) -> bool {
    matches!(
        reply,
        Ok(ZedAdapterResponse::BufferLanguage {
            source_state: SourceState::Changed,
            ..
        })
    )
}

fn is_not_open(reply: &anyhow::Result<ZedAdapterResponse>) -> bool {
    matches!(reply, Err(error) if error.to_string().contains("not open"))
}

fn is_post_edit_capacity(reply: &anyhow::Result<ZedAdapterResponse>) -> bool {
    matches!(reply, Err(error) if error.to_string().contains("post_edit_capacity"))
}

fn map_acquire_error(reason: &'static str, cold_open: bool) -> &'static str {
    match reason {
        "restricted" => "restricted",
        "budget" | "capacity" => "capacity",
        _ if cold_open => "not_open",
        _ => "unavailable",
    }
}

/// M9: Analysis is settled ONLY when source matches disk AND every active
/// language server reports `DocumentCurrent` (not `Stale` or `Unknown`).
fn is_document_current(reply: &anyhow::Result<ZedAdapterResponse>) -> bool {
    matches!(
        reply,
        Ok(ZedAdapterResponse::BufferLanguage {
            source_state: SourceState::Matching,
            analysis: Some(servers),
            ..
        }) if !servers.is_empty()
            && servers
                .iter()
                .all(|s| s.state == AnalysisState::DocumentCurrent)
    )
}

/// R3-M5: Whether an observation can still advance to `DocumentCurrent` via
/// fenced polling. Covers `Stale`/`Unknown` analysis, `SourceState::Unavailable`
/// (busy source readers), and absent server lists; stops on `DocumentCurrent`,
/// explicit empty servers on a matching buffer, or transport/adapter errors.
fn can_advance_analysis(reply: &anyhow::Result<ZedAdapterResponse>) -> bool {
    match reply {
        Ok(ZedAdapterResponse::BufferLanguage {
            source_state,
            analysis,
            ..
        }) => {
            if is_document_current(reply) {
                return false;
            }
            if *source_state == SourceState::Matching
                && analysis.as_ref().is_some_and(Vec::is_empty)
            {
                return false;
            }
            true
        }
        _ => false,
    }
}

async fn race_caller_gone<F, T>(
    caller_gone: &mut watch::Receiver<bool>,
    deadline: Instant,
    future: F,
) -> Result<T, &'static str>
where
    F: std::future::Future<Output = T>,
{
    if *caller_gone.borrow() {
        return Err("unavailable");
    }
    let deadline_at = tokio::time::Instant::from_std(deadline);
    tokio::select! {
        biased;
        _ = caller_gone.wait_for(|gone| *gone) => Err("unavailable"),
        res = tokio::time::timeout_at(deadline_at, future) => res.map_err(|_| "timeout"),
    }
}

async fn sleep_fenced(
    duration: Duration,
    caller_gone: &mut watch::Receiver<bool>,
) -> Result<(), &'static str> {
    if *caller_gone.borrow() {
        return Err("unavailable");
    }
    tokio::select! {
        biased;
        _ = caller_gone.wait_for(|gone| *gone) => Err("unavailable"),
        () = tokio::time::sleep(duration) => Ok(()),
    }
}

pub async fn api_machine_post_edit_inspect(
    State(state): State<Arc<AppState>>,
    request: Request,
) -> Response {
    // M-Ctrl-7: Authenticate from headers BEFORE buffering any request body bytes.
    let Some(target) = request
        .headers()
        .get(POST_EDIT_INSPECT_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(machine_post_edit::lookup)
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !state.machine_control.is_current(&target.connection) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_POST_EDIT_REQUEST_BYTES).await
    else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Ok(request) = serde_json::from_slice::<PostEditInspectRequest>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !request.validate_structure() {
        return reply_response(refused_reply(&request, "invalid_request"));
    }
    let Some(clamped_ms) = clamp_remaining_ms(request.remaining_ms) else {
        return reply_response(refused_reply(&request, "timeout"));
    };
    let deadline = Instant::now() + Duration::from_millis(clamped_ms);
    let (caller_tx, caller_rx) = watch::channel(false);
    let _caller_guard = CallerDisconnectGuard(caller_tx);

    let ctx = match resolve_and_authorize_initial(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
        &caller_rx,
        deadline,
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(reason) => return reply_response(refused_reply(&request, reason)),
    };

    // Acquire per-target concurrency slot bounded by remaining deadline.
    // R3-H5: Move the owned permit into the spawned target work task so an HTTP
    // timeout NEVER releases the slot before the underlying target I/O settles.
    let target_sem = target_semaphore(ctx.observed.session().machine_id());
    let target_permit = match tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        target_sem.acquire_owned(),
    )
    .await
    {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => return reply_response(refused_reply(&request, "unavailable")),
        Err(_) => return reply_response(refused_reply(&request, "timeout")),
    };

    let task_state = Arc::clone(&state);
    let task_target = target.clone();
    let task_req = request.clone();
    let work = tokio::spawn(async move {
        run_inspect_fenced(
            task_state,
            task_target,
            task_req,
            ctx,
            caller_rx,
            deadline,
            target_permit,
        )
        .await
    });

    match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), work).await {
        Ok(Ok(reply)) => reply_response(reply),
        Ok(Err(_)) => reply_response(refused_reply(&request, "unavailable")),
        Err(_) => reply_response(refused_reply(&request, "timeout")),
    }
}

async fn run_inspect_fenced(
    state: Arc<AppState>,
    target: machine_post_edit::PostEditTarget,
    request: PostEditInspectRequest,
    ctx: AuthorizedContext,
    mut caller_gone: watch::Receiver<bool>,
    deadline: Instant,
    _target_permit: tokio::sync::OwnedSemaphorePermit,
) -> PostEditInspectReply {
    let lock = super::baselines::file_lock(ctx.observed.session(), &request.path);
    // M7: Race file-lock wait against caller disconnect so queued disconnected
    // callers release target permits immediately before any target I/O starts.
    let _file_guard = match race_caller_gone(&mut caller_gone, deadline, lock.lock()).await {
        Ok(guard) => guard,
        Err(reason) => return refused_reply(&request, reason),
    };

    // R3-H1 / H-Ctrl-2: Recheck async + sync authorization AFTER target-permit and
    // per-file-lock waits, then dispatch the first scoped Zed read immediately
    // with ZERO intervening awaits (no touch_code_observation).
    if let Err(reason) = authorize_step(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
        &ctx.observed,
        &ctx.trust,
        &caller_gone,
        deadline,
    )
    .await
    {
        return refused_reply(&request, reason);
    }

    let (reply, acquired) = match observe_target_file(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
        &ctx.observed,
        &ctx.trust,
        &mut caller_gone,
        deadline,
    )
    .await
    {
        Ok(pair) => pair,
        Err(reason) => return refused_reply(&request, reason),
    };
    if reply.is_err() && !session::current(&state.hub, &state.machine_control, &ctx.observed) {
        return refused_reply(&request, "unavailable");
    }

    let mut observed_file = super::diagnostics::build_observed_file(
        &request.path,
        reply,
        acquired,
        ctx.observed.conn_epoch(),
        true,
        true,
    );

    // Final async + zero-await sync fence immediately before the sole baseline
    // commit under the per-(SessionCodeScope, path) lock (R3-H1, R3-H2, M-Ctrl-5).
    if let Err(reason) = authorize_step(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
        &ctx.observed,
        &ctx.trust,
        &caller_gone,
        deadline,
    )
    .await
    {
        return refused_reply(&request, reason);
    }

    if let Some((pending, diagnostics)) = observed_file.pending.take() {
        observed_file.file["comparison"] =
            super::baselines::record(ctx.observed.session(), &request.path, pending, &diagnostics);
    }

    let (payload, matches_expected) = super::post_edit::project_for_inspect(
        &observed_file.file,
        &request.path,
        &request.expected_lf_sha256,
    );

    if let Some(canary_id) = &request.canary_inspect_id {
        tracing::info!(
            event_name = "cowboy.code.post_edit_inspect",
            session_id = %request.session_id,
            canary_inspect_id = %canary_id,
            path = %request.path,
            outcome = "projected",
            deliver = payload["deliver"].as_bool().unwrap_or(false),
            matches_expected,
            state = observed_file.file["state"].as_str().unwrap_or("unknown"),
            "canary post-edit inspection completed"
        );
    }

    PostEditInspectReply {
        session_id: request.session_id,
        worker_epoch: request.worker_epoch,
        request_id: request.request_id,
        scope: request.scope,
        outcome: PostEditInspectOutcome::Projected {
            payload,
            matches_expected,
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn observe_target_file(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    observed: &SessionReadScope,
    trust: &LanguageServerTrust,
    caller_gone: &mut watch::Receiver<bool>,
    deadline: Instant,
) -> Result<(anyhow::Result<ZedAdapterResponse>, bool), &'static str> {
    let mut latest =
        language::observe_open_buffer_post_edit(&state.code, observed, &request.path).await;
    if is_post_edit_capacity(&latest) {
        return Err("capacity");
    }
    let mut acquired = false;
    let mut refreshed_mirror = false;

    if is_not_open(&latest) {
        // Cold acquire path: authorize and check >= 500 ms remaining AFTER async auth (R3-M14).
        authorize_step(
            state,
            runtime_connection,
            runtime_machine_id,
            request,
            observed,
            trust,
            caller_gone,
            deadline,
        )
        .await?;
        if remaining_ms_until(deadline) < MIN_POST_EDIT_COLD_START_MS {
            return Err("timeout");
        }
        super::acquire::acquire_on_scope(state, observed, &request.path, trust)
            .await
            .map_err(|reason| map_acquire_error(reason, true))?;
        acquired = true;
        authorize_step(
            state,
            runtime_connection,
            runtime_machine_id,
            request,
            observed,
            trust,
            caller_gone,
            deadline,
        )
        .await?;
        latest =
            language::observe_open_buffer_post_edit(&state.code, observed, &request.path).await;
        if is_post_edit_capacity(&latest) {
            return Err("capacity");
        }
    }

    // Unified fenced advancement loop covering already-open, refreshed, and
    // cold-acquired buffers (R4-H2, R4-M3, R4-M4).
    loop {
        if !acquired && !refreshed_mirror && is_changed(&latest) {
            authorize_step(
                state,
                runtime_connection,
                runtime_machine_id,
                request,
                observed,
                trust,
                caller_gone,
                deadline,
            )
            .await?;
            if remaining_ms_until(deadline) < MIN_POST_EDIT_COLD_START_MS {
                return Ok((latest, acquired));
            }
            // R4-H2: Propagate pre-refresh acquisition failures instead of
            // masking them with an earlier Changed observation.
            super::acquire::acquire_on_scope(state, observed, &request.path, trust)
                .await
                .map_err(|reason| map_acquire_error(reason, false))?;
            authorize_step(
                state,
                runtime_connection,
                runtime_machine_id,
                request,
                observed,
                trust,
                caller_gone,
                deadline,
            )
            .await?;
            if remaining_ms_until(deadline) < MIN_POST_EDIT_COLD_START_MS {
                return Ok((latest, acquired));
            }
            refreshed_mirror = true;
            let query = language::Query::RefreshedLanguage {
                refresh: language::AgentOwned::AgentOwned,
            };
            // R4-M4: Keep the actual refresh outcome instead of substituting an
            // earlier stale observation on adapter error.
            latest =
                language::query_open_buffer_post_edit(&state.code, observed, &request.path, query)
                    .await;
            if is_post_edit_capacity(&latest) {
                return Err("capacity");
            }
            continue;
        }
        if !can_advance_analysis(&latest) || Instant::now() >= deadline {
            return Ok((latest, acquired));
        }
        let sleep_for = POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
        if sleep_for.is_zero() {
            return Ok((latest, acquired));
        }
        // Sleep FIRST (fenced on caller liveness), THEN run authorize_step
        // immediately before issuing the next observe_open_buffer read.
        sleep_fenced(sleep_for, caller_gone).await?;
        authorize_step(
            state,
            runtime_connection,
            runtime_machine_id,
            request,
            observed,
            trust,
            caller_gone,
            deadline,
        )
        .await?;
        latest =
            language::observe_open_buffer_post_edit(&state.code, observed, &request.path).await;
        if is_post_edit_capacity(&latest) {
            return Err("capacity");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_req() -> PostEditInspectRequest {
        PostEditInspectRequest {
            session_id: "sess-http-1".into(),
            worker_epoch: "epoch-1".into(),
            request_id: "bpe-1".into(),
            turn_id: "turn-1".into(),
            scope: crate::post_edit_inspect::PostEditScope {
                binding_id: "bind-1".into(),
                revision: 1,
                incarnation: "inc-1".into(),
            },
            path: "src/lib.rs".into(),
            expected_lf_sha256: "a".repeat(64),
            remaining_ms: 4_000,
            canary_inspect_id: None,
        }
    }

    #[test]
    fn token_registration_lookup_and_drop_revocation() {
        let conn = crate::machine_control::ConnectionToken::for_test("m-runtime", "ep-1");
        let tok = format!("tok-{}", uuid::Uuid::new_v4().simple());
        assert!(machine_post_edit::lookup(&tok).is_none());
        let reg = machine_post_edit::register(
            tok.clone(),
            machine_post_edit::PostEditTarget {
                connection: conn.clone(),
                machine_id: "m-runtime".into(),
            },
        );
        let found = machine_post_edit::lookup(&tok).expect("registered token must resolve");
        assert_eq!(found.machine_id, "m-runtime");
        assert!(found.connection.same(&conn));
        assert!(machine_post_edit::supports_connection(&conn));
        let other = crate::machine_control::ConnectionToken::for_test("m-runtime", "ep-2");
        assert!(!machine_post_edit::supports_connection(&other));
        drop(reg);
        assert!(
            machine_post_edit::lookup(&tok).is_none(),
            "dropping primary WS registration must revoke post-edit token"
        );
        assert!(!machine_post_edit::supports_connection(&conn));
    }

    #[test]
    fn acquire_error_and_advancement_rules_match_r4_invariants() {
        assert_eq!(map_acquire_error("restricted", false), "restricted");
        assert_eq!(map_acquire_error("budget", false), "capacity");
        assert_eq!(map_acquire_error("capacity", false), "capacity");
        assert_eq!(map_acquire_error("unavailable", false), "unavailable");
        assert_eq!(map_acquire_error("unavailable", true), "not_open");
        assert!(is_post_edit_capacity(&Err(anyhow::anyhow!(
            "post_edit_capacity"
        ))));
        let matching_empty = Ok(ZedAdapterResponse::BufferLanguage {
            api_version: 1,
            path: "a.txt".into(),
            version: vec![],
            diagnostics: vec![],
            inlay_hints: vec![],
            semantic_tokens: vec![],
            analysis: Some(vec![]),
            buffer_snapshot: None,
            source_state: SourceState::Matching,
            refresh: None,
        });
        assert!(!can_advance_analysis(&matching_empty));
        let unavailable = Ok(ZedAdapterResponse::BufferLanguage {
            api_version: 1,
            path: "a.rs".into(),
            version: vec![],
            diagnostics: vec![],
            inlay_hints: vec![],
            semantic_tokens: vec![],
            analysis: None,
            buffer_snapshot: None,
            source_state: SourceState::Unavailable,
            refresh: None,
        });
        assert!(can_advance_analysis(&unavailable));
    }

    #[tokio::test]
    async fn target_slots_cap_concurrency_at_four_and_shared_file_locks_serialize() {
        let machine = format!("target-{}", uuid::Uuid::new_v4().simple());
        let sem = target_semaphore(&machine);
        let mut permits = Vec::new();
        for _ in 0..POST_EDIT_TARGET_CONCURRENCY {
            permits.push(Arc::clone(&sem).try_acquire_owned().unwrap());
        }
        assert!(Arc::clone(&sem).try_acquire_owned().is_err());
        drop(permits.pop());
        assert!(Arc::clone(&sem).try_acquire_owned().is_ok());

        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "lock-sess".into(),
            "antigravity".into(),
            "/tmp".into(),
            "fixture".into(),
            crate::core::SessionOrigin::Web,
            false,
        );
        let scope = hub.session_code_scope("lock-sess").unwrap();
        let lock_a1 = super::super::baselines::file_lock(&scope, "src/lib.rs");
        let lock_a2 = super::super::baselines::file_lock(&scope, "src/lib.rs");
        let lock_b = super::super::baselines::file_lock(&scope, "src/main.rs");
        assert!(Arc::ptr_eq(&lock_a1, &lock_a2));
        assert!(!Arc::ptr_eq(&lock_a1, &lock_b));
        let req = sample_req();
        assert!(req.validate());
    }
}

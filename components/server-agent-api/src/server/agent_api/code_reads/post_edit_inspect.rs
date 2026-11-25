//! Controller handler for `POST /api/machine/post-edit-inspect` (Decision 2A / R4).
//!
//! Authenticates the calling runtime Machine via its per-primary-connection
//! secret token (`x-cowboy-machine-post-edit-token`), enforces per-target
//! bounded concurrency (`POST_EDIT_TARGET_SLOTS = 4` held through target
//! settlement, R3-H5), serializes per-`(SessionCodeScope, path)` observation +
//! baseline commit (R3-H2), rechecks async owner/superagent authorization and
//! synchronous live worker/turn/trust/scope/deadline fences at every step
//! (R3-H1, R3-M14), and commits the baseline ONCE synchronously under the
//! per-file lock.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use parking_lot::Mutex;
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

use super::super::super::code_reads::{language, session};
use super::super::super::{
    AnalysisState, AppState, CodeReadScope, SourceState, ZedAdapterResponse, agent_api_bridge,
    execution, machine_post_edit, managed_calls, superagent,
};
use crate::core::{LanguageServerTrust, SessionCodeScope};
use crate::machine_control::{ConnectionToken, SessionReadScope};
use crate::post_edit_inspect::{
    MAX_POST_EDIT_REQUEST_BYTES, MIN_POST_EDIT_COLD_START_MS, POST_EDIT_INSPECT_TOKEN_HEADER,
    PostEditInspectOutcome, PostEditInspectReply, PostEditInspectRequest, clamp_remaining_ms,
};

const POST_EDIT_TARGET_CONCURRENCY: usize = 4;
const POLL_INTERVAL: Duration = Duration::from_millis(250);

static TARGET_SLOTS: LazyLock<Mutex<HashMap<String, Arc<Semaphore>>>> =
    LazyLock::new(Mutex::default);
type FileLockMap = HashMap<(SessionCodeScope, String), Weak<AsyncMutex<()>>>;
static FILE_LOCKS: LazyLock<Mutex<FileLockMap>> = LazyLock::new(Mutex::default);

fn target_semaphore(machine_id: &str) -> Arc<Semaphore> {
    let mut map = TARGET_SLOTS.lock();
    Arc::clone(
        map.entry(machine_id.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(POST_EDIT_TARGET_CONCURRENCY))),
    )
}

fn file_lock(scope: &SessionCodeScope, path: &str) -> Arc<AsyncMutex<()>> {
    let mut map = FILE_LOCKS.lock();
    if map.len() > 512 {
        map.retain(|_, weak| weak.strong_count() > 0);
    }
    let key = (scope.clone(), path.to_owned());
    if let Some(existing) = map.get(&key).and_then(Weak::upgrade) {
        return existing;
    }
    let created = Arc::new(AsyncMutex::new(()));
    map.insert(key, Arc::downgrade(&created));
    created
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

fn refused_reply(request: &PostEditInspectRequest, reason: &'static str) -> PostEditInspectReply {
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

/// Synchronous zero-await check verifying runtime connection, worker/turn/scope,
/// language-server trust, target SessionReadScope, and deadline.
fn verify_sync_fence(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    observed: &SessionReadScope,
    trust: &LanguageServerTrust,
    deadline: Instant,
) -> Result<(), &'static str> {
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

/// Async owner + superagent authorization followed immediately by the zero-await
/// synchronous fence (R3-H1).
async fn authorize_step(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    observed: &SessionReadScope,
    trust: &LanguageServerTrust,
    deadline: Instant,
) -> Result<(), &'static str> {
    let Some(info) = state.hub.session_info(&request.session_id) else {
        return Err("permission_denied");
    };
    if info.meta.system || info.meta.closing || state.hub.is_managed_child(&request.session_id) {
        return Err("restricted");
    }
    agent_api_bridge::read_parent(&state.hub, state.store.as_ref(), &info.meta)
        .await
        .map_err(|_| "permission_denied")?;
    if superagent::restricted(state, &info.meta)
        .await
        .map_err(|_| "unavailable")?
    {
        return Err("superagent_scope_denied");
    }
    verify_sync_fence(
        state,
        runtime_connection,
        runtime_machine_id,
        request,
        observed,
        trust,
        deadline,
    )
}

async fn resolve_and_authorize_initial(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
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

fn is_settled(reply: &anyhow::Result<ZedAdapterResponse>) -> bool {
    matches!(
        reply,
        Ok(ZedAdapterResponse::BufferLanguage {
            source_state: SourceState::Matching,
            analysis: Some(servers),
            ..
        }) if !servers.is_empty() && servers.iter().all(|s| s.state != AnalysisState::Unknown)
    )
}

fn is_analysed(reply: &anyhow::Result<ZedAdapterResponse>) -> bool {
    matches!(
        reply,
        Ok(ZedAdapterResponse::BufferLanguage {
            analysis: Some(servers),
            ..
        }) if !servers.is_empty() && servers.iter().all(|s| s.state != AnalysisState::Unknown)
    )
}

pub async fn api_machine_post_edit_inspect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(target) = headers
        .get(POST_EDIT_INSPECT_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(machine_post_edit::lookup)
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !state.machine_control.is_current(&target.connection) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if body.len() > MAX_POST_EDIT_REQUEST_BYTES {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let Ok(request) = serde_json::from_slice::<PostEditInspectRequest>(&body) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if !request.validate() {
        return reply_response(refused_reply(&request, "invalid_request"));
    }
    let Some(clamped_ms) = clamp_remaining_ms(request.remaining_ms) else {
        return reply_response(refused_reply(&request, "timeout"));
    };
    let deadline = Instant::now() + Duration::from_millis(clamped_ms);

    let ctx = match resolve_and_authorize_initial(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
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
        let _target_permit = target_permit;
        run_inspect_fenced(task_state, task_target, task_req, ctx, deadline).await
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
    deadline: Instant,
) -> PostEditInspectReply {
    let lock = file_lock(ctx.observed.session(), &request.path);
    let _file_guard = match tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        lock.lock(),
    )
    .await
    {
        Ok(guard) => guard,
        Err(_) => return refused_reply(&request, "timeout"),
    };

    // R3-H1: Recheck async + sync authorization AFTER target-permit and
    // per-file-lock waits before touching code observation or target I/O.
    if let Err(reason) = authorize_step(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
        &ctx.observed,
        &ctx.trust,
        deadline,
    )
    .await
    {
        return refused_reply(&request, reason);
    }

    managed_calls::touch_code_observation(&state, &ctx.observed).await;

    let (reply, acquired) = match observe_target_file(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
        &ctx.observed,
        &ctx.trust,
        deadline,
    )
    .await
    {
        Ok(pair) => pair,
        Err(reason) => return refused_reply(&request, reason),
    };

    // Build observation WITHOUT mutating global baselines yet (R3-H2).
    let mut observed_file = super::diagnostics::build_observed_file(
        &request.path,
        reply,
        acquired,
        ctx.observed.conn_epoch(),
        true,
    );

    // Final fence (R3-H1 + R3-H2): async auth first, then zero-await sync fence
    // immediately before the sole synchronous baseline commit while holding `_file_guard`.
    if let Err(reason) = authorize_step(
        &state,
        &target.connection,
        &target.machine_id,
        &request,
        &ctx.observed,
        &ctx.trust,
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

    let (payload, matches_expected) =
        super::post_edit::project_for_inspect(&observed_file.file, &request.expected_lf_sha256);

    if let Some(canary_id) = request.canary_inspect_id.as_deref() {
        tracing::info!(
            event_name = "cowboy.post_edit.canary_inspected",
            canary_inspect_id = %canary_id,
            session = %request.session_id,
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

async fn observe_target_file(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    observed: &SessionReadScope,
    trust: &LanguageServerTrust,
    deadline: Instant,
) -> Result<(anyhow::Result<ZedAdapterResponse>, bool), &'static str> {
    let first = language::observe_open_buffer(&state.code, observed, &request.path).await;
    authorize_step(
        state,
        runtime_connection,
        runtime_machine_id,
        request,
        observed,
        trust,
        deadline,
    )
    .await?;

    let not_open = matches!(&first, Err(error) if error.to_string().contains("not open"));
    if !not_open {
        if first.is_ok() {
            // R3-M14: Check remaining >= 500 ms AFTER async auth before acquire/refresh.
            if remaining_ms_until(deadline) >= MIN_POST_EDIT_COLD_START_MS {
                let held =
                    super::acquire::acquire_on_scope(state, observed, &request.path, trust).await;
                if held.is_ok() && is_changed(&first) {
                    authorize_step(
                        state,
                        runtime_connection,
                        runtime_machine_id,
                        request,
                        observed,
                        trust,
                        deadline,
                    )
                    .await?;
                    if remaining_ms_until(deadline) >= MIN_POST_EDIT_COLD_START_MS {
                        let refreshed = refresh_with_fence(
                            state,
                            runtime_connection,
                            runtime_machine_id,
                            request,
                            observed,
                            trust,
                            first,
                            deadline,
                        )
                        .await?;
                        return Ok((refreshed, false));
                    }
                }
            }
        }
        return Ok((first, false));
    }

    // Cold acquire path: require >= 500 ms remaining AFTER async auth (R3-M14).
    if remaining_ms_until(deadline) < MIN_POST_EDIT_COLD_START_MS {
        return Err("timeout");
    }
    if let Err(reason) =
        super::acquire::acquire_on_scope(state, observed, &request.path, trust).await
    {
        return match reason {
            "restricted" => Err("restricted"),
            "budget" => Err("capacity"),
            _ => Err("not_open"),
        };
    }

    loop {
        authorize_step(
            state,
            runtime_connection,
            runtime_machine_id,
            request,
            observed,
            trust,
            deadline,
        )
        .await?;
        let reply = language::observe_open_buffer(&state.code, observed, &request.path).await;
        if is_analysed(&reply) || reply.is_err() || Instant::now() >= deadline {
            return Ok((reply, true));
        }
        let sleep_for = POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
        if sleep_for.is_zero() {
            return Ok((reply, true));
        }
        tokio::time::sleep(sleep_for).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn refresh_with_fence(
    state: &AppState,
    runtime_connection: &ConnectionToken,
    runtime_machine_id: &str,
    request: &PostEditInspectRequest,
    observed: &SessionReadScope,
    trust: &LanguageServerTrust,
    original: anyhow::Result<ZedAdapterResponse>,
    deadline: Instant,
) -> Result<anyhow::Result<ZedAdapterResponse>, &'static str> {
    let query = language::Query::RefreshedLanguage {
        refresh: language::AgentOwned::AgentOwned,
    };
    let reply = language::query_open_buffer(&state.code, observed, &request.path, query).await;
    let reloaded = matches!(
        &reply,
        Ok(ZedAdapterResponse::BufferLanguage {
            refresh: Some(outcome),
            ..
        }) if outcome == "reloaded"
    );
    if !reloaded {
        return Ok(if reply.is_ok() { reply } else { original });
    }
    let mut latest = reply;
    loop {
        authorize_step(
            state,
            runtime_connection,
            runtime_machine_id,
            request,
            observed,
            trust,
            deadline,
        )
        .await?;
        if is_settled(&latest) || Instant::now() >= deadline {
            return Ok(latest);
        }
        let sleep_for = POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
        if sleep_for.is_zero() {
            return Ok(latest);
        }
        tokio::time::sleep(sleep_for).await;
        latest = language::observe_open_buffer(&state.code, observed, &request.path).await;
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
        drop(reg);
        assert!(
            machine_post_edit::lookup(&tok).is_none(),
            "dropping primary WS registration must revoke post-edit token"
        );
    }

    #[tokio::test]
    async fn target_slots_cap_concurrency_at_four_and_file_locks_serialize_same_path() {
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
        let lock_a1 = file_lock(&scope, "src/lib.rs");
        let lock_a2 = file_lock(&scope, "src/lib.rs");
        let lock_b = file_lock(&scope, "src/other.rs");
        assert!(Arc::ptr_eq(&lock_a1, &lock_a2));
        assert!(!Arc::ptr_eq(&lock_a1, &lock_b));

        let resp = reply_response(refused_reply(&sample_req(), "restricted"));
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

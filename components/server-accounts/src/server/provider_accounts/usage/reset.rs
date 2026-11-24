//! Immediate banked resets of a named native account (Claude). The Machine
//! that holds the account's native login runs the same signed collector, CLI
//! identity and token as the account's quota reads; the Controller never
//! talks to the Provider. A reset is a synchronous, typed-confirmed request
//! like the Service card's: it shares the Provider pacing, and an unknown
//! outcome is reported as unknown and never retried. The owner may instead
//! schedule one reset of the account (`schedule`), which the timer spends
//! through the same admission. A named portable account (Codex) resets the
//! same way on the Machine its usage runs on, only with an exact release
//! whose collector declares a refresh-free reset (`portable`).
mod portable;
pub mod schedule;
use super::*;
use crate::machine_control::NativeResetFailure;
use crate::usage::{ResetError, ResetResult};

/// How long a dispatched reset may wait on its Machine before it must not
/// start: well inside the Controller's 45-second wait for its reply.
const DISPATCH_EXPIRY_MS: i64 = 30_000;

/// The account's lane in the shared action-log table. Service collector
/// lanes are signed reset IDs (`anthropic`), never this.
pub fn lane(id: &ProviderAccountId) -> String {
    format!("account-{}", id.as_str())
}

fn declares_reset(binding: &crate::plugin_host::PluginUsageSpec) -> bool {
    binding.reset.is_some() && !binding.reset_argv.is_empty()
}

/// The login, Machine and exact installed release one reset runs on,
/// resolved before the reservation and retained through dispatch.
struct Route {
    target: RouteTarget,
    binding: Box<crate::plugin_host::PluginUsageSpec>,
    installed: Box<crate::machine_protocol::PluginInventory>,
}

/// Where the reset runs: a native login, or a vault account's replica on the
/// Machine its usage placement selects.
enum RouteTarget {
    Native(Box<Target>),
    Portable {
        target: Box<super::portable::Target>,
        machine: String,
    },
}

impl Route {
    /// Provider starts this reset charges: the native identity read and the
    /// spend, or the portable collector's single run.
    fn starts(&self) -> usize {
        match self.target {
            RouteTarget::Native(_) => RESET_COLLECTOR_STARTS,
            RouteTarget::Portable { .. } => 1,
        }
    }

    /// The exact sign-in this reset spends from, as the usage cache binds it
    /// (a native login, or a portable identity, admission and sign-in).
    fn scope(&self) -> Scope {
        match &self.target {
            RouteTarget::Native(target) => Scope::from(target.completion.clone()),
            RouteTarget::Portable { target, .. } => Scope::Portable(target.scope.clone()),
        }
    }
}

/// The reset binding of exactly this installed release, if it declares
/// resets for `provider`.
pub fn exact_reset_binding(
    state: &AppState,
    installed: &crate::machine_protocol::PluginInventory,
    provider: &str,
) -> Option<crate::plugin_host::PluginUsageSpec> {
    state
        .usage
        .exact_usage_binding(installed)
        .filter(|binding| declares_reset(binding) && binding.account == provider)
}

/// The reset route of exactly this resolved native login and its verified
/// release, or why none is eligible (protocol, host policy, support).
fn native_route(state: &AppState, target: Target) -> Result<Route, &'static str> {
    // A window that can never hold the reset's two starts never advertises
    // or admits it.
    if !state.usage.limiter.holds(RESET_COLLECTOR_STARTS) {
        return Err("account_reset_unsupported");
    }
    let completion = &target.completion;
    let (_, version) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
        .ok_or("machine_unavailable")?;
    // The two-start reset needs a Machine that enforces the spacing.
    if version < crate::machine_protocol::NATIVE_RESET_SPACING_PROTOCOL_VERSION {
        return Err("machine_usage_update_required");
    }
    if !state
        .supervisor
        .vendor_request_allowed(&completion.profile.provider_id, &completion.machine_id)
    {
        return Err("runtime_not_allowed");
    }
    let installed = target.installed.clone();
    let binding = exact_reset_binding(state, &installed, &target.provider)
        .ok_or("account_reset_unsupported")?;
    Ok(Route {
        target: RouteTarget::Native(Box::new(target)),
        binding: Box::new(binding),
        installed: Box::new(installed),
    })
}

/// The route of a reset of this owner's account now: only logins on a
/// reset-capable, host-allowed Machine are considered.
async fn route(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<Route, &'static str> {
    match target_for(state, owner, id, Purpose::Reset)
        .await
        .map_err(public_code)?
    {
        Resolved::Native(target) => native_route(state, *target),
        Resolved::Portable(target) => {
            // A sign-in committed while the target was resolved could pair
            // the earlier sign-in with the new vault generation: read it
            // again after the vault was sampled. A later one advances the
            // vault generation the dispatch compares under the account lock.
            let store = state.store.as_ref().ok_or("unavailable")?;
            let sign_in = store
                .latest_vault_sign_in(owner, id)
                .await
                .map_err(|_| "unavailable")?;
            if sign_in != target.scope.sign_in_generation {
                return Err("account_binding_changed");
            }
            portable::route(state, owner, id, *target)
        }
    }
}

/// Whether the native login the read model resolved can spend a reset now,
/// or why not.
pub(super) fn supported_for(state: &AppState, resolved: &Resolved) -> Result<(), &'static str> {
    match resolved {
        Resolved::Native(target) => native_route(state, (**target).clone()).map(|_| ()),
        Resolved::Portable(_) => Err("account_reset_unsupported"),
    }
}

/// Whether a portable account's resolved usage placement can spend a reset,
/// or why not.
pub fn supported_placed(
    state: &AppState,
    target: super::portable::Target,
    placed: super::portable::Placement,
) -> Result<(), &'static str> {
    portable::route_placed(state, target, placed).map(|_| ())
}

fn unsent(code: &'static str) -> NativeResetFailure {
    NativeResetFailure {
        call_may_have_reached_provider: false,
        code,
    }
}

/// One admitted reset: its route, the account's collection lock (a quota
/// read and a reset of one account never overlap) and its Provider starts,
/// reserved at admission (before anything is logged or sent) from the latest
/// moment the Machine may run them.
struct Admitted {
    route: Route,
    admitted_at_ms: i64,
    expires_at_ms: i64,
    /// The Provider spacing the expiry and reservations were derived from;
    /// the dispatch carries the same value.
    spacing_ms: i64,
    permit: crate::usage::limiter::Permit,
    _serial: tokio::sync::OwnedMutexGuard<()>,
}

/// Collector starts one named reset makes: the identity read the Machine runs
/// immediately before the spend, and the spend itself. They are charged as
/// two properly spaced Provider starts; the Machine waits for the second.
const RESET_COLLECTOR_STARTS: usize = 2;

/// How long a request waits for the account's collection lock.
const ADMISSION_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// When a reset refused for the account's own in-flight quota read may retry.
const BUSY_RETRY_MS: i64 = 5_000;

/// Why a reset was not admitted, and for a paced one when the Provider window
/// has room for it (the limiter holds the Provider for it until then).
struct Refused {
    code: &'static str,
    retry_at_ms: Option<i64>,
    /// The Provider the limiter now holds for this account, if it does.
    held: Option<String>,
}

impl From<&'static str> for Refused {
    fn from(code: &'static str) -> Self {
        Self {
            code,
            retry_at_ms: None,
            held: None,
        }
    }
}

fn paced(retry_at_ms: i64) -> Refused {
    Refused {
        code: "usage_reset_paced",
        retry_at_ms: Some(retry_at_ms),
        held: None,
    }
}

/// Admit a reset on an eligible route, within the request (bounded): the
/// Provider window must have room for its starts now. A manual reset obeys
/// the Provider spacing and backoff, never the account poll interval a
/// preceding usage read started. Its starts are charged here. A paced
/// refusal sent nothing, names when the window has room, and holds the
/// Provider against other starts until then, so the caller may resend.
async fn admit(state: &AppState, owner: &str, id: &ProviderAccountId) -> Result<Admitted, Refused> {
    let route = route(state, owner, id).await?;
    let slot = state.usage.native.entry(owner, id.as_str());
    let serial = tokio::time::timeout(ADMISSION_WAIT, Arc::clone(&slot.collection).lock_owned())
        .await
        .map_err(|_| paced(now_ms().saturating_add(BUSY_RETRY_MS)))?;
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    let now = now_ms();
    // One config snapshot: the expiry, both reservations and the dispatch.
    let spacing = state.usage.limiter.spacing_ms();
    let expires_at_ms = expiry(&route, now, spacing);
    // Every start happens no later than the expiry: record the last one there
    // and each earlier one a spacing before it.
    let starts = route.starts();
    let first = expires_at_ms
        .saturating_sub(spacing.saturating_mul(i64::try_from(starts - 1).unwrap_or(0)));
    let permit = state
        .usage
        .limiter
        .start_action_bounded(&route.binding.account, &key, now, starts, (first, spacing))
        .map_err(|budget| {
            if budget.next_allowed_ms == i64::MAX {
                Refused::from("usage_admission_unavailable")
            } else {
                Refused {
                    held: Some(route.binding.account.clone()),
                    ..paced(budget.next_allowed_ms.max(now))
                }
            }
        })?;
    Ok(Admitted {
        route,
        admitted_at_ms: now,
        expires_at_ms,
        spacing_ms: spacing,
        permit,
        _serial: serial,
    })
}

/// How long the pre-dispatch audit insert may take: a few seconds, and never
/// into the last second of the dispatch window.
fn audit_bound(expires_at_ms: i64, now_ms: i64) -> std::time::Duration {
    let left = expires_at_ms.saturating_sub(now_ms).saturating_sub(1_000);
    std::time::Duration::from_millis(u64::try_from(left.clamp(0, 5_000)).unwrap_or(0))
}

/// The absolute expiry after which the Machine starts nothing of the reset:
/// after the native paced second start, or the portable single start.
fn expiry(route: &Route, admitted_at_ms: i64, spacing: i64) -> i64 {
    match route.target {
        RouteTarget::Native(_) => admitted_at_ms
            .saturating_add(spacing)
            .saturating_add(DISPATCH_EXPIRY_MS),
        RouteTarget::Portable { .. } => admitted_at_ms.saturating_add(DISPATCH_EXPIRY_MS),
    }
}

/// Charge the reset's two spaced Provider starts at enqueue, spend it and
/// settle the permit.
async fn consume(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
    admitted: Admitted,
    idempotency_key: &str,
    expected_credit_id: &str,
) -> Result<ResetResult, ResetError> {
    let Admitted {
        route,
        admitted_at_ms,
        expires_at_ms,
        spacing_ms: spacing,
        permit,
        _serial,
    } = admitted;
    let request = crate::machine_protocol::NativeResetRequest {
        provider: route.binding.account.clone(),
        idempotency_key: idempotency_key.to_owned(),
        expected_credit_id: Some(expected_credit_id.to_owned()),
    };
    let timing = crate::machine_protocol::NativeResetDispatch {
        plugin_version: route.installed.plugin_version.clone(),
        generation_digest: route.installed.generation_digest.clone(),
        // From the original admission: the paced second start, then the
        // bound within which the Machine must start the spend.
        expires_at_ms,
        spend_not_before_ms: admitted_at_ms.saturating_add(spacing),
        // Protocol 51: the Machine also keeps it from the identity read.
        min_spacing_ms: Some(spacing),
    };
    let reply = match &route.target {
        RouteTarget::Native(target) => dispatch(state, owner, target, request, timing).await,
        RouteTarget::Portable { target, machine } => {
            portable::dispatch(
                state,
                owner,
                id,
                (target, machine),
                &route,
                request,
                expires_at_ms,
            )
            .await
        }
    };
    let (result, outcome) = settle(reply, expected_credit_id);
    permit.finish(outcome, now_ms());
    drop(_serial);
    // Publish through the existing budgeted path; a reset just charged this
    // account's start, so this normally waits for its next due collection.
    let (state, owner_id, account) = (Arc::clone(state), owner.to_owned(), id.clone());
    tokio::spawn(async move {
        let _ = refresh_one(&state, &owner_id, &account).await;
        state.hub.broadcast_account_usage_changed(&owner_id);
    });
    result
}

/// The Machine's closed reply (or the dispatch failure) as a reset result
/// and the pacing outcome it charges. Display messages are fixed per code.
fn settle(
    reply: Result<Value, NativeResetFailure>,
    expected_credit_id: &str,
) -> (
    Result<ResetResult, ResetError>,
    crate::usage::limiter::Outcome,
) {
    use crate::usage::limiter::Outcome;
    let reply = match reply {
        Ok(reply) => reply,
        Err(failure) => {
            // An uncertain dispatch never shows a message saying nothing was
            // sent; its code still classifies the backoff below.
            let display = if failure.call_may_have_reached_provider {
                "reset_outcome_unknown"
            } else {
                failure.code
            };
            let error = ResetError::new(
                failure.call_may_have_reached_provider,
                None,
                error_message(display),
            );
            // An authentication failure backs off whether or not it was sent;
            // otherwise nothing reached the Provider: keep the charge only.
            let outcome = if matches!(
                failure.code,
                "native_usage_authentication_required" | "portable_usage_authentication_required"
            ) {
                Outcome::Authentication
            } else if failure.call_may_have_reached_provider {
                Outcome::Success
            } else {
                Outcome::Unused
            };
            return (Err(error), outcome);
        }
    };
    // A credit ID the Machine returns is kept only in the confirmed grammar.
    let credit = reply
        .pointer("/error/credit_id")
        .or_else(|| reply.get("credit_id"))
        .and_then(Value::as_str)
        .filter(|id| valid_credit_id(id))
        .map(str::to_owned);
    let Some(error) = reply.get("error") else {
        // A success must be a closed outcome for exactly the pinned grant.
        let outcome = reply["outcome"]
            .as_str()
            .filter(|outcome| crate::machine_protocol::NATIVE_RESET_OUTCOMES.contains(outcome))
            .filter(|_| credit.as_deref() == Some(expected_credit_id));
        return match outcome {
            Some(outcome) => (
                Ok(ResetResult {
                    outcome: outcome.to_owned(),
                    credit_id: credit,
                }),
                Outcome::Success,
            ),
            None => (
                Err(ResetError::new(
                    true,
                    credit,
                    error_message("reset_outcome_unknown"),
                )),
                Outcome::Success,
            ),
        };
    };
    let code = closed_reset_code(error["code"].as_str());
    // Unsent only for a well-formed error naming a closed refusal code with
    // an explicit `call_may_have_reached_provider: false` and no success
    // payload; any other shape or code may follow a spent reset.
    let reached = !(error.is_object()
        && reply.get("outcome").is_none()
        && error["call_may_have_reached_provider"] == Value::Bool(false)
        && error["code"].as_str() == Some(code)
        && code != "reset_outcome_unknown");
    let display = if reached {
        "reset_outcome_unknown"
    } else {
        code
    };
    // A refused or changed reset is an answer, not an outage. Only an
    // authentication failure backs off, reached or not.
    let outcome = if code == "reset_authentication_required" {
        Outcome::Authentication
    } else {
        Outcome::Success
    };
    (
        Err(ResetError::new(reached, credit, error_message(display))),
        outcome,
    )
}

fn closed_reset_code(code: Option<&str>) -> &'static str {
    match code {
        Some("reset_authentication_required") => "reset_authentication_required",
        Some("reset_credit_changed") => "reset_credit_changed",
        Some("reset_unavailable") => "reset_unavailable",
        Some("reset_refused") => "reset_refused",
        _ => "reset_outcome_unknown",
    }
}

/// Enqueue the reset on the login's Machine under the login binding and
/// host policy, rechecked inside the binding transaction. It carries the
/// selected exact release and an absolute expiry after which the Machine
/// never starts it. Once enqueued, only its reply decides the outcome, even
/// if the binding transaction then fails to commit.
async fn dispatch(
    state: &AppState,
    owner: &str,
    target: &Target,
    request: crate::machine_protocol::NativeResetRequest,
    release: crate::machine_protocol::NativeResetDispatch,
) -> Result<Value, NativeResetFailure> {
    let store = state.store.as_ref().ok_or(unsent("unavailable"))?;
    let completion = &target.completion;
    let (connection, version) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
        .ok_or(unsent("machine_unavailable"))?;
    // Only a Machine that enforces the spacing runs the two-start reset.
    if version < crate::machine_protocol::NATIVE_RESET_SPACING_PROTOCOL_VERSION
        || release.min_spacing_ms.is_none()
    {
        return Err(unsent("machine_usage_update_required"));
    }
    let mut sent = None;
    let admitted = store
        .with_native_login_binding(owner, &target.operation, completion, || {
            if !state
                .supervisor
                .vendor_request_allowed(&completion.profile.provider_id, &completion.machine_id)
            {
                return Ok(Err("runtime_not_allowed"));
            }
            Ok(state
                .machine_control
                .begin_native_reset(&connection, &completion.profile, request, release)
                .map(|pending| sent = Some(pending))
                .map_err(|code| dispatch_error(&code)))
        })
        .await;
    match (sent, admitted) {
        (Some(pending), _) => pending.await,
        (None, Ok(Err(code))) => Err(unsent(code)),
        (None, _) => Err(unsent("account_binding_changed")),
    }
}

/// The owner's registry entry. Another owner's account is indistinguishable
/// from a missing one.
async fn owned(
    store: &crate::store::Store,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(), &'static str> {
    store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?
        .accounts
        .iter()
        .any(|account| &account.id == id)
        .then_some(())
        .ok_or("account_unavailable")
}

/// What started a reset: the owner's confirmation now, or their schedule.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trigger {
    Manual,
    Scheduled,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
        }
    }
}

/// Append one action-log row; whether it was written.
async fn log(
    store: &crate::store::Store,
    (lane, key, trigger): (&str, &str, Trigger),
    (status, phase): (&str, &str),
    message: &str,
    credit_id: Option<&str>,
) -> bool {
    store
        .append_provider_action_log(
            lane,
            trigger.as_str(),
            status,
            phase,
            message,
            credit_id,
            Some(key),
            now_ms(),
        )
        .await
        .is_ok()
}

fn refusal(refused: impl Into<Refused>) -> Response {
    let Refused {
        code, retry_at_ms, ..
    } = refused.into();
    let status = match code {
        "account_unavailable" => StatusCode::NOT_FOUND,
        // A 5xx reads as an unknown outcome to the Web client: only a
        // Service without a store, which sent nothing, may answer with one.
        "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        "usage_reset_paced" => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_REQUEST,
    };
    let mut error = json!({"code":code,"message":error_message(code)});
    let retry_after_ms = retry_at_ms.map(|at| at.saturating_sub(now_ms()).max(0));
    if let Some(after) = retry_after_ms {
        error["retry_after_ms"] = json!(after);
    }
    let mut response = (status, Json(json!({"error":error}))).into_response();
    if let Some(after) = retry_after_ms {
        // Whole seconds, rounded up: a client never resends before the slot.
        let seconds = (after.saturating_add(999) / 1_000).max(1);
        if let Ok(value) = axum::http::HeaderValue::from_str(&seconds.to_string()) {
            response
                .headers_mut()
                .insert(axum::http::header::RETRY_AFTER, value);
        }
    }
    response
}

/// A confirmed grant ID: bounded and opaque, like the collector's own.
fn valid_credit_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':'))
}

/// How one admitted spend ended, after its terminal action-log row.
enum Spent {
    /// The pre-dispatch audit row could not be written (in time): nothing
    /// was sent and the Provider start was released unused.
    NotAudited,
    Done(ResetResult),
    Failed(ResetError),
}

/// Spend an admitted reset of exactly `expected` under `key`: the audit row
/// first (bounded so it never holds the Provider permit past the dispatch
/// window), then the dispatch, then its terminal row. The caller owns
/// whatever runs this to completion; an unknown outcome is never retried.
async fn spend(
    state: &Arc<AppState>,
    store: &crate::store::Store,
    (owner, id): (&str, &ProviderAccountId),
    admitted: Admitted,
    (key, trigger): (&str, Trigger),
    expected: &str,
) -> Spent {
    let lane = lane(id);
    let row = (lane.as_str(), key, trigger);
    let started = match trigger {
        Trigger::Manual => "Manual reset attempt started",
        Trigger::Scheduled => "Scheduled reset attempt started",
    };
    let bound = audit_bound(admitted.expires_at_ms, now_ms());
    // A failed insert, like a slow one, settles unsent: no spend without it.
    let audited = tokio::time::timeout(
        bound,
        log(
            store,
            row,
            ("started", "preflight"),
            started,
            Some(expected),
        ),
    )
    .await;
    if !matches!(audited, Ok(true)) {
        admitted
            .permit
            .finish(crate::usage::limiter::Outcome::Unused, now_ms());
        return Spent::NotAudited;
    }
    match consume(state, owner, id, admitted, key, expected).await {
        Ok(result) => {
            let phase = ("succeeded", "provider_response");
            log(
                store,
                row,
                phase,
                &result.outcome,
                result.credit_id.as_deref(),
            )
            .await;
            Spent::Done(result)
        }
        Err(error) => {
            let phase = if error.call_may_have_reached_provider {
                ("unknown", "consume")
            } else {
                ("failed", "preflight")
            };
            let message = error.to_string();
            log(store, row, phase, &message, error.credit_id.as_deref()).await;
            Spent::Failed(error)
        }
    }
}

/// `POST /api/provider-accounts/{account_id}/usage/reset`: spend the
/// confirmed grant now. Requires the typed `confirm` confirmation and the
/// confirmed `expected_credit_id`. Synchronous for the caller; the admitted
/// spend runs in a Service-owned task that holds the permit and the account
/// lock, awaits settlement and writes the terminal log row, so a caller
/// disconnect never abandons it. An unknown outcome is
/// `409 reset_outcome_unknown` and is never retried by the Service.
pub async fn http_reset(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
    Json(request): Json<ResetRequest>,
) -> Response {
    let owner = auth.principal.user_id.clone();
    if request.confirm != "confirm" {
        return refusal("reset_confirmation_required");
    }
    if !valid_credit_id(&request.expected_credit_id) {
        return refusal("reset_credit_invalid");
    }
    let Some(store) = state.store.clone() else {
        return refusal("unavailable");
    };
    if let Err(code) = owned(&store, &owner, &id).await {
        return refusal(code);
    }
    // Admission (bounded) belongs to the request: a disconnect before it
    // leaves nothing behind. The admitted spend then belongs to the Service.
    let admitted = match admit(&state, &owner, &id).await {
        Ok(admitted) => admitted,
        Err(code) => return refusal(code),
    };
    let task = tokio::spawn(async move {
        let key = new_reset_idempotency_key();
        let expected = request.expected_credit_id;
        let spent = spend(
            &state,
            &store,
            (&owner, &id),
            admitted,
            (&key, Trigger::Manual),
            &expected,
        )
        .await;
        match spent {
            Spent::NotAudited => {
                let body = json!({"error":{"code":"reset_failed",
                    "message":error_message("unavailable"),"credit_id":null}});
                (StatusCode::CONFLICT, Json(body)).into_response()
            }
            Spent::Done(result) => Json(result).into_response(),
            Spent::Failed(error) => {
                let code = if error.call_may_have_reached_provider {
                    "reset_outcome_unknown"
                } else {
                    "reset_failed"
                };
                let body = json!({"error":{"code":code,"message":error.to_string(),
                    "credit_id":error.credit_id}});
                (StatusCode::CONFLICT, Json(body)).into_response()
            }
        }
    });
    match task.await {
        Ok(response) => response,
        // The spend task itself failed: its outcome is unknown.
        Err(_) => (
            StatusCode::CONFLICT,
            Json(json!({"error":{"code":"reset_outcome_unknown",
                "message":error_message("reset_outcome_unknown")}})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_closed_machine_replies_are_results_and_unknown_is_never_a_refusal() {
        use crate::usage::limiter::Outcome;
        let (result, outcome) = settle(
            Ok(json!({"outcome":"reset","credit_id":"grant:1"})),
            "grant:1",
        );
        assert_eq!(result.unwrap().outcome, "reset");
        assert_eq!(outcome, Outcome::Success);
        // An outcome outside the closed set is unknown.
        let (result, _) = settle(Ok(json!({"outcome":"consumed"})), "grant:1");
        assert!(result.unwrap_err().call_may_have_reached_provider);
        for (reply, reached, outcome, message) in [
            (
                json!({"error":{"code":"reset_credit_changed","call_may_have_reached_provider":false}}),
                false,
                Outcome::Success,
                error_message("reset_credit_changed"),
            ),
            // Reached: unknown, whatever its code says.
            (
                json!({"error":{"code":"reset_refused","call_may_have_reached_provider":true}}),
                true,
                Outcome::Success,
                error_message("reset_outcome_unknown"),
            ),
            (
                json!({"error":{"code":"collector says {\"x\"}"}}),
                true,
                Outcome::Success,
                error_message("reset_outcome_unknown"),
            ),
            // A 401 after the call still backs off, but its outcome is unknown.
            (
                json!({"error":{"code":"reset_authentication_required",
                    "call_may_have_reached_provider":true}}),
                true,
                Outcome::Authentication,
                error_message("reset_outcome_unknown"),
            ),
        ] {
            let (result, charged) = settle(Ok(reply.clone()), "grant:1");
            let error = result.unwrap_err();
            assert_eq!(error.call_may_have_reached_provider, reached, "{reply}");
            assert_eq!(charged, outcome, "{reply}");
            assert_eq!(error.to_string(), message, "{reply}");
        }
        let (result, outcome) = settle(Err(unsent("machine_unavailable")), "grant:1");
        assert!(!result.unwrap_err().call_may_have_reached_provider);
        assert_eq!(outcome, Outcome::Unused);
    }

    /// A success for another grant, or naming none, is unknown; a malformed
    /// credit ID never reaches the log; a portable authentication refusal
    /// backs off.
    #[test]
    fn a_success_must_name_the_pinned_grant() {
        use crate::usage::limiter::Outcome;
        for reply in [
            json!({"outcome":"reset","credit_id":"grant:2"}),
            json!({"outcome":"already_used"}),
            json!({"outcome":"reset","credit_id":"../x"}),
        ] {
            let (result, _) = settle(Ok(reply.clone()), "grant:1");
            let error = result.unwrap_err();
            assert!(error.call_may_have_reached_provider, "{reply}");
            assert_ne!(error.credit_id.as_deref(), Some("../x"), "{reply}");
        }
        let (result, _) = settle(
            Ok(json!({"outcome":"already_used","credit_id":"grant:1"})),
            "grant:1",
        );
        assert_eq!(result.unwrap().outcome, "already_used");
        let (_, outcome) = settle(
            Err(unsent("portable_usage_authentication_required")),
            "grant:1",
        );
        assert_eq!(outcome, Outcome::Authentication);
    }

    /// An error reply is unsent only with a well-formed closed refusal and an
    /// explicit false; an uncertain dispatch never says nothing was sent.
    #[test]
    fn only_a_closed_explicit_refusal_is_unsent_and_uncertainty_says_so() {
        use crate::usage::limiter::Outcome;
        for reply in [
            json!({"error":{"code":"collector text","call_may_have_reached_provider":false}}),
            json!({"error":"reset_refused"}),
            json!({"error":{"code":"reset_outcome_unknown","call_may_have_reached_provider":false}}),
            // A success and an error together are never an unsent refusal.
            json!({"outcome":"reset","credit_id":"grant:1",
                "error":{"code":"reset_refused","call_may_have_reached_provider":false}}),
        ] {
            let (result, _) = settle(Ok(reply.clone()), "grant:1");
            assert!(
                result.unwrap_err().call_may_have_reached_provider,
                "{reply}"
            );
        }
        let (result, outcome) = settle(
            Ok(json!({"error":{"code":"reset_authentication_required",
                "call_may_have_reached_provider":true}})),
            "grant:1",
        );
        assert_eq!(
            result.unwrap_err().to_string(),
            error_message("reset_outcome_unknown")
        );
        assert_eq!(outcome, Outcome::Authentication);
        let uncertain = NativeResetFailure {
            call_may_have_reached_provider: true,
            code: "native_usage_authentication_required",
        };
        let (result, outcome) = settle(Err(uncertain), "grant:1");
        assert_eq!(
            result.unwrap_err().to_string(),
            error_message("reset_outcome_unknown")
        );
        assert_eq!(outcome, Outcome::Authentication);
        assert_eq!(audit_bound(100_000, 0), std::time::Duration::from_secs(5));
        assert_eq!(audit_bound(1_500, 0), std::time::Duration::from_millis(500));
        assert_eq!(audit_bound(0, 10), std::time::Duration::ZERO);
    }

    #[test]
    fn confirmed_credit_ids_are_bounded_before_admission() {
        assert!(valid_credit_id("grant_1:2"));
        for invalid in ["", "grant 1", "../x", &"x".repeat(65)] {
            assert!(!valid_credit_id(invalid), "{invalid}");
        }
        assert_eq!(
            refusal("reset_credit_invalid").status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn an_authentication_refusal_backs_off_even_before_dispatch() {
        let (result, outcome) = settle(
            Err(unsent("native_usage_authentication_required")),
            "grant:1",
        );
        assert!(!result.unwrap_err().call_may_have_reached_provider);
        assert_eq!(outcome, crate::usage::limiter::Outcome::Authentication);
    }

    #[test]
    fn only_a_collector_declaring_resets_spends_them() {
        let usage = |host: &str| -> crate::plugin_host::PluginUsageSpec {
            let host: Value = serde_json::from_str(host).unwrap();
            serde_json::from_value(host["usage"].clone()).unwrap()
        };
        assert!(declares_reset(&usage(include_str!(
            "../../../../../../plugins/claude-code/host.json"
        ))));
        assert!(!declares_reset(&usage(include_str!(
            "../../../../../../plugins/claude-deepseek/host.json"
        ))));
    }

    #[tokio::test]
    async fn another_owner_cannot_reach_the_account() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let account: ProviderAccountId = "11111111-1111-4111-8111-111111111111".parse().unwrap();
        assert_eq!(
            owned(&store, "intruder", &account).await,
            Err("account_unavailable")
        );
        assert_eq!(
            refusal("account_unavailable").status(),
            StatusCode::NOT_FOUND
        );
        // The lane fits the shared action-log Provider-ID shape.
        assert_eq!(lane(&account).len(), "account-".len() + 36);
    }
}

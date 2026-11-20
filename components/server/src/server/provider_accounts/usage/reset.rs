//! Immediate banked resets of a named native account (Claude). The Machine
//! that holds the account's native login runs the same signed collector, CLI
//! identity and token as the account's quota reads; the Controller never
//! talks to the Provider. A reset is a synchronous, typed-confirmed request
//! like the Service card's: it shares the Provider pacing, and an unknown
//! outcome is reported as unknown and never retried. Named accounts have no
//! scheduled resets, and portable accounts no resets at all.
use super::*;
use crate::machine_control::NativeResetFailure;
use crate::usage::{ResetError, ResetResult};

/// How long a dispatched reset may wait on its Machine before it must not
/// start: well inside the Controller's 45-second wait for its reply.
const DISPATCH_EXPIRY_MS: i64 = 30_000;

/// The account's lane in the shared action-log table. Service collector
/// lanes are signed reset IDs (`anthropic`), never this.
pub(in crate::server) fn lane(id: &ProviderAccountId) -> String {
    format!("account-{}", id.as_str())
}

fn declares_reset(binding: &crate::plugin_host::PluginUsageSpec) -> bool {
    binding.reset.is_some() && !binding.reset_argv.is_empty()
}

/// The login, Machine and exact installed release one reset runs on,
/// resolved before the reservation and retained through dispatch.
struct Route {
    target: Target,
    binding: Box<crate::plugin_host::PluginUsageSpec>,
    installed: Box<crate::machine_protocol::PluginInventory>,
}

/// The reset route of exactly this resolved native login, or why none is
/// eligible (protocol, host policy, exact release support).
async fn native_route(state: &AppState, target: Target) -> Result<Route, &'static str> {
    let completion = &target.completion;
    let (_, version) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
        .ok_or("machine_unavailable")?;
    if version < crate::machine_protocol::NATIVE_RESET_PROTOCOL_VERSION {
        return Err("machine_usage_update_required");
    }
    if !state
        .supervisor
        .vendor_request_allowed(&completion.profile.provider_id, &completion.machine_id)
    {
        return Err("runtime_not_allowed");
    }
    let machines = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .list_machines()
        .await
        .map_err(|_| "unavailable")?;
    let installed = active_machine_plugin(
        &machines,
        &completion.machine_id,
        &completion.profile.provider_id,
    )
    .map_err(|_| "machine_unavailable")?;
    let binding = state
        .usage
        .exact_usage_binding(&installed)
        .filter(|binding| declares_reset(binding) && binding.account == target.provider)
        .ok_or("account_reset_unsupported")?;
    Ok(Route {
        target,
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
        Resolved::Native(target) => native_route(state, *target).await,
        Resolved::Portable(_) => Err("account_reset_unsupported"),
    }
}

/// Whether the binding the read model resolved can spend a reset now.
pub(super) async fn supported_for(state: &AppState, resolved: &Resolved) -> bool {
    match resolved {
        Resolved::Native(target) => native_route(state, (**target).clone()).await.is_ok(),
        Resolved::Portable(_) => false,
    }
}

fn unsent(code: &'static str) -> NativeResetFailure {
    NativeResetFailure {
        call_may_have_reached_provider: false,
        code,
    }
}

/// Spend one confirmed grant. It holds the account's collection lock, so a
/// quota read and a reset of one account never overlap, and charges a start
/// from the shared Provider pacing.
async fn consume(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
    idempotency_key: &str,
    expected_credit_id: &str,
) -> Result<ResetResult, ResetError> {
    let local = |code| ResetError::new(false, None, error_message(code));
    let route = route(state, owner, id).await.map_err(local)?;
    let slot = state.usage.native.entry(owner, id.as_str());
    let serial = Arc::clone(&slot.collection).lock_owned().await;
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    let permit = state
        .usage
        .limiter
        .start_action(&route.binding.account, &key, now_ms())
        .map_err(|_| local("usage_reset_paced"))?;
    let request = crate::machine_protocol::NativeResetRequest {
        provider: route.binding.account.clone(),
        idempotency_key: idempotency_key.to_owned(),
        expected_credit_id: Some(expected_credit_id.to_owned()),
    };
    let (result, outcome) = settle(dispatch(state, owner, &route, request).await);
    permit.finish(outcome, now_ms());
    drop(serial);
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
) -> (
    Result<ResetResult, ResetError>,
    crate::usage::limiter::Outcome,
) {
    use crate::usage::limiter::Outcome;
    let reply = match reply {
        Ok(reply) => reply,
        Err(failure) => {
            let error = ResetError::new(
                failure.call_may_have_reached_provider,
                None,
                error_message(failure.code),
            );
            // Nothing reached the Provider: keep the charge, add no backoff.
            let outcome = if failure.call_may_have_reached_provider {
                Outcome::Success
            } else {
                Outcome::Unused
            };
            return (Err(error), outcome);
        }
    };
    let credit = reply
        .pointer("/error/credit_id")
        .or_else(|| reply.get("credit_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(error) = reply.get("error") else {
        let outcome = reply["outcome"]
            .as_str()
            .filter(|outcome| crate::machine_protocol::NATIVE_RESET_OUTCOMES.contains(outcome));
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
    // Unknown unless the Machine says it was never sent.
    let reached = error["call_may_have_reached_provider"] != Value::Bool(false);
    let display = if reached && code != "reset_authentication_required" {
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
    route: &Route,
    request: crate::machine_protocol::NativeResetRequest,
) -> Result<Value, NativeResetFailure> {
    let store = state.store.as_ref().ok_or(unsent("unavailable"))?;
    let completion = &route.target.completion;
    let (connection, version) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
        .ok_or(unsent("machine_unavailable"))?;
    if version < crate::machine_protocol::NATIVE_RESET_PROTOCOL_VERSION {
        return Err(unsent("machine_usage_update_required"));
    }
    let release = crate::machine_protocol::NativeResetDispatch {
        plugin_version: route.installed.plugin_version.clone(),
        generation_digest: route.installed.generation_digest.clone(),
        expires_at_ms: now_ms().saturating_add(DISPATCH_EXPIRY_MS),
    };
    let mut sent = None;
    let admitted = store
        .with_native_login_binding(owner, &route.target.operation, completion, || {
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

async fn log(
    store: &crate::store::Store,
    (lane, key): (&str, &str),
    (status, phase): (&str, &str),
    message: &str,
    credit_id: Option<&str>,
) {
    let _ = store
        .append_provider_action_log(
            lane,
            "manual",
            status,
            phase,
            message,
            credit_id,
            Some(key),
            now_ms(),
        )
        .await;
}

fn refusal(code: &'static str) -> Response {
    let status = match code {
        "account_unavailable" => StatusCode::NOT_FOUND,
        "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        "usage_reset_paced" => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        Json(json!({"error":{"code":code,"message":error_message(code)}})),
    )
        .into_response()
}

/// `POST /api/provider-accounts/{account_id}/usage/reset`: spend the
/// confirmed grant now. Requires the typed `confirm` confirmation and the
/// confirmed `expected_credit_id`. Synchronous; an unknown outcome is
/// `409 reset_outcome_unknown` and is never retried by the Service.
pub(in crate::server) async fn http_reset(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
    Json(request): Json<ResetRequest>,
) -> Response {
    let owner = &auth.principal.user_id;
    if request.confirm != "confirm" {
        return refusal("reset_confirmation_required");
    }
    let Some(store) = state.store.as_ref() else {
        return refusal("unavailable");
    };
    if let Err(code) = owned(store, owner, &id).await {
        return refusal(code);
    }
    let (lane, key) = (lane(&id), new_reset_idempotency_key());
    let expected = request.expected_credit_id.as_str();
    let row = (lane.as_str(), key.as_str());
    log(
        store,
        row,
        ("started", "preflight"),
        "Manual reset attempt started",
        Some(expected),
    )
    .await;
    match consume(&state, owner, &id, &key, expected).await {
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
            Json(result).into_response()
        }
        Err(error) => {
            let (status, phase, code) = if error.call_may_have_reached_provider {
                ("unknown", "consume", "reset_outcome_unknown")
            } else {
                ("failed", "preflight", "reset_failed")
            };
            let message = error.to_string();
            log(
                store,
                row,
                (status, phase),
                &message,
                error.credit_id.as_deref(),
            )
            .await;
            (
                StatusCode::CONFLICT,
                Json(json!({"error":{"code":code,"message":message,"credit_id":error.credit_id}})),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_closed_machine_replies_are_results_and_unknown_is_never_a_refusal() {
        use crate::usage::limiter::Outcome;
        let (result, outcome) = settle(Ok(json!({"outcome":"reset","credit_id":"grant:1"})));
        assert_eq!(result.unwrap().outcome, "reset");
        assert_eq!(outcome, Outcome::Success);
        // An outcome outside the closed set is unknown.
        let (result, _) = settle(Ok(json!({"outcome":"consumed"})));
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
            // A 401 after the call still backs off.
            (
                json!({"error":{"code":"reset_authentication_required",
                    "call_may_have_reached_provider":true}}),
                true,
                Outcome::Authentication,
                error_message("reset_authentication_required"),
            ),
        ] {
            let (result, charged) = settle(Ok(reply.clone()));
            let error = result.unwrap_err();
            assert_eq!(error.call_may_have_reached_provider, reached, "{reply}");
            assert_eq!(charged, outcome, "{reply}");
            assert_eq!(error.to_string(), message, "{reply}");
        }
        let (result, outcome) = settle(Err(unsent("machine_unavailable")));
        assert!(!result.unwrap_err().call_may_have_reached_provider);
        assert_eq!(outcome, Outcome::Unused);
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

//! Banked usage resets of a named account: a native login (Claude) on the
//! Machine that holds it, or a portable vault account (Codex, Grok) on the
//! Machine and account replica its quota reads use. Either way the same signed
//! collector, CLI identity and token as the account's quota reads run the
//! reset; the Controller never talks to the Provider. Resets reuse the Service collector's contract:
//! typed owner confirmation, the shared Provider pacing, persisted one-shot
//! schedules ([`fire_due_reset`]) and unknown outcomes that are never retried.
use super::*;
use crate::machine_control::NativeResetFailure;
use crate::usage::{ResetError, ResetReservation, ResetResult};

mod receipts;

/// The account's lane in the shared scheduled-action and action-log tables.
/// Service collector lanes are signed reset IDs (`anthropic`), never this.
pub(in crate::server) fn lane(id: &ProviderAccountId) -> String {
    format!("account-{}", id.as_str())
}

/// The owner's registry entry. Another owner's account is indistinguishable
/// from a missing one.
async fn owned(
    store: &crate::store::Store,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<crate::store::provider_accounts::Account, &'static str> {
    store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?
        .accounts
        .into_iter()
        .find(|account| &account.id == id)
        .ok_or("account_unavailable")
}

fn declares_reset(binding: &crate::plugin_host::PluginUsageSpec) -> bool {
    binding.reset.is_some() && !binding.reset_argv.is_empty()
}

/// Where and how one reset runs, resolved before a reservation or schedule
/// claim and retained through dispatch: the Machine, and the usage binding of
/// the exact installed release there (which decides reset support and the
/// claim policy), never a presentation default.
enum Route {
    Native {
        target: Box<Target>,
        binding: Box<crate::plugin_host::PluginUsageSpec>,
    },
    Portable {
        target: Box<portable::Target>,
        machine: String,
        plugin: Box<crate::machine_protocol::PluginInventory>,
        binding: Box<crate::plugin_host::PluginUsageSpec>,
    },
}

impl Route {
    fn binding(&self) -> &crate::plugin_host::PluginUsageSpec {
        match self {
            Self::Native { binding, .. } | Self::Portable { binding, .. } => binding,
        }
    }
}

/// The route a reset of this owner's account would take now, or why none is
/// eligible: operation-specific Machine eligibility (protocol, host policy,
/// placement) is resolved before any support is advertised or schedule taken.
async fn route(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<Route, &'static str> {
    match target(state, owner, id).await.map_err(public_code)? {
        Resolved::Native(target) => native_route(state, *target).await,
        Resolved::Portable(target) => {
            let (machine, plugin) = portable::placement(state, owner, id, &target)?;
            let binding = state
                .usage
                .exact_usage_binding(&plugin)
                .filter(|binding| {
                    declares_reset(binding) && binding.account == target.binding.account
                })
                .ok_or("account_reset_unsupported")?;
            Ok(Route::Portable {
                target,
                machine,
                plugin: Box::new(plugin),
                binding: Box::new(binding),
            })
        }
    }
}

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
        .runtime_allowed(&completion.profile.provider_id, &completion.machine_id)
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
    Ok(Route::Native {
        target: Box::new(target),
        binding: Box::new(binding),
    })
}

/// Whether a reset of this account is supported and eligible now.
pub(super) async fn supported(state: &AppState, owner: &str, id: &ProviderAccountId) -> bool {
    route(state, owner, id).await.is_ok()
}

/// The authority a schedule of this account is bound to: its current
/// sign-in (native login, portable identity and admission revision, or an
/// entered key's generation). A schedule outlives no change of it.
pub(super) async fn authority(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Option<String> {
    let scope = target(state, owner, id).await.ok()?.scope();
    let bytes = serde_json::to_vec(&(owner, id, scope)).ok()?;
    Some(format!("sha256:{}", crate::admin::hex_sha256(&bytes)))
}

/// Reserve a paced reset start on an eligible route. It holds the account's
/// collection lock, so a quota read and a reset of one account never overlap.
async fn reserve(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(ResetReservation, Route), &'static str> {
    let route = route(state, owner, id).await?;
    let slot = state.usage.native.entry(owner, id.as_str());
    let serial = Arc::clone(&slot.collection).lock_owned().await;
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    let permit = state
        .usage
        .limiter
        .start_action(&route.binding().account, &key, now_ms())
        .map_err(|_| "usage_reset_paced")?;
    Ok((ResetReservation::new(key, permit, serial), route))
}

fn local(code: &'static str) -> ResetError {
    ResetError::new(false, None, error_message(code))
}

/// Spend one reset on the reserved route. Admission is rechecked at enqueue:
/// the native login binding (or the portable account lock) and host policy.
async fn consume(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
    (mut reservation, route): (ResetReservation, Route),
    idempotency_key: &str,
    expected_credit_id: Option<&str>,
) -> Result<ResetResult, ResetError> {
    let permit = reservation.take(&crate::usage::limiter::Limiter::account_key(
        owner,
        id.as_str(),
    ))?;
    let request = crate::machine_protocol::NativeResetRequest {
        provider: route.binding().account.clone(),
        idempotency_key: idempotency_key.to_owned(),
        expected_credit_id: expected_credit_id.map(str::to_owned),
    };
    let reply = match &route {
        Route::Native { target, .. } => dispatch_native(state, owner, target, request).await,
        Route::Portable {
            target,
            machine,
            plugin,
            ..
        } => portable::consume_reset(state, owner, id, (target, machine, plugin), request).await,
    };
    let (result, outcome) = settle(reply);
    permit.finish(outcome, now_ms());
    drop(reservation);
    // Publish through the existing budgeted path; a reset just charged this
    // account's start, so this normally waits for its next due collection.
    let (state, owner_id, account) = (Arc::clone(state), owner.to_owned(), id.clone());
    tokio::spawn(async move {
        let _ = refresh_one(&state, &owner_id, &account).await;
        state.hub.broadcast_account_usage_changed(&owner_id);
    });
    result
}

/// The collector's reply (or the dispatch failure) as a reset result and the
/// pacing outcome it charges.
fn settle(
    reply: Result<Value, NativeResetFailure>,
) -> (
    Result<ResetResult, ResetError>,
    crate::usage::limiter::Outcome,
) {
    use crate::machine_protocol::portable_usage::{UsageFailureKind, classify_usage_failure};
    use crate::usage::limiter::Outcome;
    match reply {
        Ok(reply) => {
            let result = crate::usage::parse_reset_response(&reply);
            // A refused or changed reset is an answer, not an outage. Only
            // authentication failures back off.
            let outcome = match &result {
                Err(error)
                    if classify_usage_failure(&error.to_string())
                        == UsageFailureKind::Authentication =>
                {
                    Outcome::Authentication
                }
                _ => Outcome::Success,
            };
            (result, outcome)
        }
        Err(failure) => (
            Err(ResetError::new(
                failure.call_may_have_reached_provider,
                None,
                error_message(failure.code),
            )),
            // Nothing reached the Provider: keep the charge, add no backoff.
            if failure.call_may_have_reached_provider {
                Outcome::Success
            } else {
                Outcome::Unused
            },
        ),
    }
}

/// The native login's Machine runs the reset under the login binding and the
/// host's runtime policy (`--provider-runtime-machine`), checked again when
/// it is enqueued. Once enqueued, only its reply decides the outcome, even if
/// the binding transaction then fails to commit.
async fn dispatch_native(
    state: &AppState,
    owner: &str,
    target: &Target,
    request: crate::machine_protocol::NativeResetRequest,
) -> Result<Value, NativeResetFailure> {
    let unsent = |code| NativeResetFailure {
        call_may_have_reached_provider: false,
        code,
    };
    let store = state.store.as_ref().ok_or(unsent("unavailable"))?;
    let completion = &target.completion;
    let (connection, version) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
        .ok_or(unsent("machine_unavailable"))?;
    if version < crate::machine_protocol::NATIVE_RESET_PROTOCOL_VERSION {
        return Err(unsent("machine_usage_update_required"));
    }
    let mut sent = None;
    let admitted = store
        .with_native_login_binding(owner, &target.operation, completion, || {
            if !state
                .supervisor
                .runtime_allowed(&completion.profile.provider_id, &completion.machine_id)
            {
                return Ok(Err("runtime_not_allowed"));
            }
            Ok(state
                .machine_control
                .begin_native_reset(&connection, &completion.profile, request)
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

/// A named account's persisted schedule, as the timer sees it.
struct NamedLane<'a> {
    state: &'a Arc<AppState>,
    owner: &'a str,
    owner_enabled: bool,
    id: &'a ProviderAccountId,
    lane: String,
    /// The route [`ResetLane::reserve`] resolved, retained for `consume`.
    route: parking_lot::Mutex<Option<Route>>,
}

impl ResetLane for NamedLane<'_> {
    fn lane(&self) -> &str {
        &self.lane
    }
    async fn admitted(&self, flight: &crate::store::ResetFlight) -> bool {
        // A disabled owner, or any change of the sign-in that scheduled it,
        // withdraws the spending authority.
        self.owner_enabled
            && flight.authority.is_some()
            && authority(self.state, self.owner, self.id).await == flight.authority
    }
    async fn reserve(&self) -> Result<(ResetReservation, bool), ResetError> {
        let (reservation, route) = reserve(self.state, self.owner, self.id)
            .await
            .map_err(local)?;
        let claims_before = route.binding().claims_reset_before_attempt();
        *self.route.lock() = Some(route);
        Ok((reservation, claims_before))
    }
    async fn consume(
        &self,
        reservation: ResetReservation,
        idempotency_key: &str,
    ) -> Result<ResetResult, ResetError> {
        let route = self
            .route
            .lock()
            .take()
            .ok_or_else(|| local("account_binding_changed"))?;
        // The timer spends the Provider's next grant, as Service schedules do.
        consume(
            self.state,
            self.owner,
            self.id,
            (reservation, route),
            idempotency_key,
            None,
        )
        .await
    }
    async fn cleared(&self) {
        self.state.hub.broadcast_account_usage_changed(self.owner);
    }
}

/// Fire due named-account schedules until the server stops. Schedules live
/// only in the store, so a restart resumes them and each fires once. A
/// disabled owner's schedules are withdrawn, never fired.
pub(in crate::server) async fn run(state: Arc<AppState>) {
    loop {
        if let Some(store) = state.store.as_ref()
            && let Ok(users) = store.list_users().await
        {
            for user in &users {
                let Ok(registry) = store.provider_accounts(&user.id).await else {
                    continue;
                };
                for account in &registry.accounts {
                    let lane = NamedLane {
                        state: &state,
                        owner: &user.id,
                        owner_enabled: user.disabled_at_ms.is_none(),
                        id: &account.id,
                        lane: lane(&account.id),
                        route: parking_lot::Mutex::new(None),
                    };
                    fire_if_due(&lane, store).await;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
    }
}

/// The owner's scheduled reset of this account, if any.
pub(super) async fn schedule_of(
    store: &crate::store::Store,
    id: &ProviderAccountId,
) -> Option<Value> {
    let action = store.load_provider_reset(&lane(id)).await.ok().flatten()?;
    Some(json!({"fire_at_ms": action.fire_at_ms}))
}

/// Persist a schedule bound to `authority`; it is withdrawn, unspent, once
/// that authority changes.
async fn persist_schedule(
    store: &crate::store::Store,
    owner: &str,
    id: &ProviderAccountId,
    fire_at_ms: i64,
    authority: &str,
) -> Result<(), &'static str> {
    owned(store, owner, id).await?;
    let key = new_reset_idempotency_key();
    let lane = lane(id);
    store
        .upsert_provider_reset(&lane, fire_at_ms, &key)
        .await
        .map_err(|_| "unavailable")?;
    if !store
        .bind_provider_reset_authority(&lane, &key, authority)
        .await
        .map_err(|_| "unavailable")?
    {
        return Err("unavailable");
    }
    log_reset(
        store,
        &lane,
        "scheduled",
        "scheduled",
        "timer",
        "Reset scheduled",
        None,
        Some(&key),
    )
    .await;
    Ok(())
}

async fn cancel_schedule(
    store: &crate::store::Store,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(), &'static str> {
    owned(store, owner, id).await?;
    let lane = lane(id);
    store
        .delete_provider_reset(&lane)
        .await
        .map_err(|_| "unavailable")?;
    log_reset(
        store,
        &lane,
        "scheduled",
        "cancelled",
        "timer",
        "Scheduled reset cancelled",
        None,
        None,
    )
    .await;
    Ok(())
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

/// A named reset request. `operation_id` (8 to 128 of `[A-Za-z0-9_-]`) makes
/// it idempotent: a repeat returns the original receipt and never spends again.
#[derive(Deserialize)]
pub(in crate::server) struct NamedResetRequest {
    confirm: String,
    expected_credit_id: String,
    #[serde(default)]
    operation_id: Option<String>,
}

fn receipt_response((status, body): receipts::Receipt) -> Response {
    (status, Json(body)).into_response()
}

/// `POST /api/provider-accounts/{account_id}/usage/reset`: spend the
/// confirmed grant now. Requires the typed `confirm` confirmation. The spend
/// runs in a Service-owned task: a disconnecting caller recovers its terminal
/// or unknown receipt by repeating the same `operation_id`.
pub(in crate::server) async fn http_reset(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
    Json(request): Json<NamedResetRequest>,
) -> Response {
    let owner = auth.principal.user_id.clone();
    if request.confirm != "confirm" {
        return refusal("reset_confirmation_required");
    }
    let Some(store) = state.store.clone() else {
        return refusal("unavailable");
    };
    let lane = lane(&id);
    let key = match request.operation_id.as_deref() {
        Some(operation) => match receipts::operation_key(&owner, &lane, operation) {
            Ok(key) => key,
            Err(code) => return refusal(code),
        },
        None => new_reset_idempotency_key(),
    };
    if let Err(code) = owned(&store, &owner, &id).await {
        return refusal(code);
    }
    if let Some(receipt) = receipts::existing(&store, &lane, &key).await {
        return receipt_response(receipt);
    }
    let reserved = match reserve(&state, &owner, &id).await {
        Ok(reserved) => reserved,
        Err(code) => return refusal(code),
    };
    let Some(publish) = receipts::begin(&lane, &key) else {
        drop(reserved);
        return receipts::existing(&store, &lane, &key)
            .await
            .map_or_else(|| refusal("reset_in_progress"), receipt_response);
    };
    let expected = request.expected_credit_id;
    log_reset(
        &store,
        &lane,
        "manual",
        "started",
        "preflight",
        "Manual reset attempt started",
        Some(&expected),
        Some(&key),
    )
    .await;
    let task = tokio::spawn(async move {
        let result = consume(&state, &owner, &id, reserved, &key, Some(&expected)).await;
        let receipt = record(&store, &lane, &key, result).await;
        publish.send_replace(Some(receipt.clone()));
        receipt
    });
    // A caller disconnect detaches this wait, never the spend or its receipt.
    receipt_response(
        task.await
            .unwrap_or_else(|_| receipts::unknown(error_message("native_reset_outcome_unknown"))),
    )
}

/// Log a manual attempt's terminal row and project it as its receipt.
async fn record(
    store: &crate::store::Store,
    lane: &str,
    key: &str,
    result: Result<ResetResult, ResetError>,
) -> receipts::Receipt {
    match result {
        Ok(result) => {
            log_reset(
                store,
                lane,
                "manual",
                "succeeded",
                "provider_response",
                &result.outcome,
                result.credit_id.as_deref(),
                Some(key),
            )
            .await;
            (StatusCode::OK, json!(result))
        }
        Err(error) => {
            let (status, phase, code) = if error.call_may_have_reached_provider {
                ("unknown", "consume", "reset_outcome_unknown")
            } else {
                ("failed", "preflight", "reset_failed")
            };
            let message = error.to_string();
            log_reset(
                store,
                lane,
                "manual",
                status,
                phase,
                &message,
                error.credit_id.as_deref(),
                Some(key),
            )
            .await;
            (
                StatusCode::CONFLICT,
                json!({"error":{"code":code,"message":message,"credit_id":error.credit_id}}),
            )
        }
    }
}

/// `PUT .../usage/reset/schedule`: spend the Provider's next grant once at
/// `fire_at_ms` (at least a minute ahead). Replaces an existing schedule.
pub(in crate::server) async fn http_schedule(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
    Json(request): Json<ResetScheduleRequest>,
) -> Response {
    let owner = &auth.principal.user_id;
    if request.confirm != "confirm" {
        return refusal("reset_confirmation_required");
    }
    if request.fire_at_ms < now_ms().saturating_add(60_000) {
        return refusal("reset_schedule_too_soon");
    }
    let Some(store) = state.store.as_ref() else {
        return refusal("unavailable");
    };
    // Only an eligible route is scheduled, bound to the current sign-in.
    if let Err(code) = route(&state, owner, &id).await {
        return refusal(code);
    }
    let Some(authority) = authority(&state, owner, &id).await else {
        return refusal("account_binding_changed");
    };
    if let Err(code) = persist_schedule(store, owner, &id, request.fire_at_ms, &authority).await {
        return refusal(code);
    }
    state.hub.broadcast_account_usage_changed(owner);
    response(read_one(&state, owner, &id).await)
}

/// `DELETE .../usage/reset/schedule`: cancel only the scheduled reset. Works
/// while the account cannot run, so an owner can always withdraw it.
pub(in crate::server) async fn http_cancel(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
) -> Response {
    let owner = &auth.principal.user_id;
    let Some(store) = state.store.as_ref() else {
        return refusal("unavailable");
    };
    if let Err(code) = cancel_schedule(store, owner, &id).await {
        return refusal(code);
    }
    state.hub.broadcast_account_usage_changed(owner);
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::provider_accounts::{Change, Mutation, MutationResult};

    async fn create(store: &crate::store::Store, owner: &str) -> ProviderAccountId {
        let MutationResult::Applied(created) = store
            .mutate_provider_accounts(
                owner,
                "create",
                &Mutation {
                    expected_revision: 0,
                    change: Change::Create {
                        provider_id: "claude-code".into(),
                        label: "Lark".into(),
                    },
                },
            )
            .await
            .unwrap()
        else {
            panic!("create")
        };
        created.account_id
    }

    #[tokio::test]
    async fn schedules_are_owner_scoped_and_another_owner_sees_not_found() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let account = create(&store, "owner").await;
        let fire_at = now_ms() + 3_600_000;
        // The lane fits the shared table's Provider-ID shape.
        assert_eq!(lane(&account).len(), "account-".len() + 36);
        assert_eq!(
            persist_schedule(&store, "intruder", &account, fire_at, "sha256:a").await,
            Err("account_unavailable")
        );
        assert!(schedule_of(&store, &account).await.is_none());
        persist_schedule(&store, "owner", &account, fire_at, "sha256:a")
            .await
            .unwrap();
        assert_eq!(
            schedule_of(&store, &account).await,
            Some(json!({"fire_at_ms": fire_at}))
        );
        // Another owner can neither cancel nor learn whether it exists.
        assert_eq!(
            cancel_schedule(&store, "intruder", &account).await,
            Err("account_unavailable")
        );
        assert!(schedule_of(&store, &account).await.is_some());
        assert_eq!(
            refusal("account_unavailable").status(),
            StatusCode::NOT_FOUND
        );
        cancel_schedule(&store, "owner", &account).await.unwrap();
        assert!(schedule_of(&store, &account).await.is_none());
        let logs = store.provider_action_logs(10).await.unwrap();
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().all(|log| log.provider == lane(&account)));
    }

    #[tokio::test]
    async fn named_reset_logs_are_visible_only_to_their_owner() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let account = create(&store, "owner").await;
        log_reset(
            &store,
            &lane(&account),
            "manual",
            "succeeded",
            "provider_response",
            "reset",
            Some("grant:1"),
            Some("cowboy-reset-op-1"),
        )
        .await;
        log_reset(
            &store,
            "anthropic",
            "manual",
            "succeeded",
            "provider_response",
            "reset",
            None,
            None,
        )
        .await;
        let logs = store.provider_action_logs(10).await.unwrap();
        let visible = |caller: &'static str| {
            let (store, logs) = (store.clone(), logs.clone());
            async move {
                crate::server::usage_api::owned_action_logs(&store, caller, logs)
                    .await
                    .into_iter()
                    .map(|log| log.provider)
                    .collect::<Vec<_>>()
            }
        };
        assert_eq!(visible("owner").await.len(), 2);
        assert_eq!(visible("intruder").await, ["anthropic"]);
    }

    #[test]
    fn local_refusals_are_never_unknown() {
        for code in [
            "machine_unavailable",
            "machine_usage_update_required",
            "runtime_not_allowed",
            "usage_reset_paced",
        ] {
            let error = local(code);
            assert!(!error.call_may_have_reached_provider, "{code}");
            assert_eq!(error.to_string(), error_message(code));
        }
    }

    #[test]
    fn portable_resets_follow_their_collector_plugin_binding() {
        let usage = |host: &str| -> crate::plugin_host::PluginUsageSpec {
            let host: Value = serde_json::from_str(host).unwrap();
            serde_json::from_value(host["usage"].clone()).unwrap()
        };
        // Codex and Grok collectors spend banked resets; the DeepSeek
        // balance collector serving the shared API-key scope declares none.
        assert!(declares_reset(&usage(include_str!(
            "../../../../../../plugins/codex/host.json"
        ))));
        assert!(declares_reset(&usage(include_str!(
            "../../../../../../plugins/grok/host.json"
        ))));
        assert!(!declares_reset(&usage(include_str!(
            "../../../../../../plugins/claude-deepseek/host.json"
        ))));
        assert_eq!(
            error_message("portable_reset_outcome_unknown"),
            error_message("native_reset_outcome_unknown")
        );
    }
}

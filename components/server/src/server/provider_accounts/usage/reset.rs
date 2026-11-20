//! Banked usage resets of a named native account (Claude). The Machine that
//! holds the account's native login runs the same signed collector, CLI
//! identity and token as the account's quota reads; the Controller never
//! talks to the Provider. Resets reuse the Service collector's contract:
//! typed owner confirmation, the shared Provider pacing, persisted one-shot
//! schedules ([`fire_due_reset`]) and unknown outcomes that are never retried.
use super::*;
use crate::usage::{ResetError, ResetReservation, ResetResult};

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

/// The owner's admitted target whose signed usage binding declares resets.
async fn reset_target(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(Target, bool), &'static str> {
    let target = target(state, owner, id).await.map_err(public_code)?;
    let binding = state
        .usage
        .plugin_bindings()
        .into_iter()
        .find(|binding| {
            binding.account == target.provider
                && binding.reset.is_some()
                && !binding.reset_argv.is_empty()
        })
        .ok_or("account_reset_unsupported")?;
    Ok((target, binding.claims_reset_before_attempt()))
}

/// Reserve a paced reset start for this account. It holds the account's
/// collection lock, so a quota read and a reset of one account never overlap.
async fn reserve(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(ResetReservation, bool), &'static str> {
    let (target, claims_before) = reset_target(state, owner, id).await?;
    let slot = state.usage.native.entry(owner, id.as_str());
    let serial = Arc::clone(&slot.collection).lock_owned().await;
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    let permit = state
        .usage
        .limiter
        .start_action(&target.provider, &key, now_ms())
        .map_err(|_| "usage_reset_paced")?;
    Ok((ResetReservation::new(key, permit, serial), claims_before))
}

fn local(code: &'static str) -> ResetError {
    ResetError::new(false, None, error_message(code))
}

/// Spend one reset through the account's sign-in Machine. Admission is the
/// quota read's: binding, identity, availability, sign-in Machine and the
/// host's runtime policy (`--provider-runtime-machine`), checked again here.
async fn consume(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
    mut reservation: ResetReservation,
    idempotency_key: &str,
    expected_credit_id: Option<&str>,
) -> Result<ResetResult, ResetError> {
    let permit = reservation.take(&crate::usage::limiter::Limiter::account_key(
        owner,
        id.as_str(),
    ))?;
    let (result, outcome) = dispatch(state, owner, id, idempotency_key, expected_credit_id).await;
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

async fn dispatch(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    idempotency_key: &str,
    expected_credit_id: Option<&str>,
) -> (
    Result<ResetResult, ResetError>,
    crate::usage::limiter::Outcome,
) {
    use crate::usage::limiter::Outcome;
    let unsent = |code| (Err(local(code)), Outcome::Unused);
    let target = match reset_target(state, owner, id).await {
        Ok((target, _)) => target,
        Err(code) => return unsent(code),
    };
    let Some(store) = state.store.as_ref() else {
        return unsent("unavailable");
    };
    let completion = &target.completion;
    let Some((connection, version)) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
    else {
        return unsent("machine_unavailable");
    };
    if version < crate::machine_protocol::NATIVE_RESET_PROTOCOL_VERSION {
        return unsent("machine_usage_update_required");
    }
    let request = crate::machine_protocol::NativeResetRequest {
        provider: target.provider.clone(),
        idempotency_key: idempotency_key.to_owned(),
        expected_credit_id: expected_credit_id.map(str::to_owned),
    };
    let pending = store
        .with_native_login_binding(owner, &target.operation, completion, || {
            Ok(
                if state
                    .supervisor
                    .runtime_allowed(&completion.profile.provider_id, &completion.machine_id)
                {
                    state
                        .machine_control
                        .begin_native_reset(&connection, &completion.profile, request)
                        .map_err(|code| dispatch_error(&code))
                } else {
                    Err("runtime_not_allowed")
                },
            )
        })
        .await;
    let pending = match pending {
        Ok(Ok(pending)) => pending,
        Ok(Err(code)) => return unsent(code),
        Err(_) => return unsent("account_binding_changed"),
    };
    match pending.await {
        Ok(reply) => {
            let result = crate::usage::parse_reset_response(reply);
            // A refused or changed reset is an answer, not an outage. Only
            // authentication failures back off.
            let outcome = match &result {
                Err(error) if error.to_string().contains("authentication required") => {
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

/// A named account's persisted schedule, as the timer sees it.
struct NamedLane<'a> {
    state: &'a Arc<AppState>,
    owner: &'a str,
    id: &'a ProviderAccountId,
    lane: String,
}

impl ResetLane for NamedLane<'_> {
    fn lane(&self) -> &str {
        &self.lane
    }
    async fn reserve(&self) -> Result<(ResetReservation, bool), ResetError> {
        reserve(self.state, self.owner, self.id)
            .await
            .map_err(local)
    }
    async fn consume(
        &self,
        reservation: ResetReservation,
        idempotency_key: &str,
    ) -> Result<ResetResult, ResetError> {
        // The timer spends the Provider's next grant, as Service schedules do.
        consume(
            self.state,
            self.owner,
            self.id,
            reservation,
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
/// only in the store, so a restart resumes them and each fires once.
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
                        id: &account.id,
                        lane: lane(&account.id),
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

async fn persist_schedule(
    store: &crate::store::Store,
    owner: &str,
    id: &ProviderAccountId,
    fire_at_ms: i64,
) -> Result<(), &'static str> {
    owned(store, owner, id).await?;
    let key = new_reset_idempotency_key();
    let lane = lane(id);
    store
        .upsert_provider_reset(&lane, fire_at_ms, &key)
        .await
        .map_err(|_| "unavailable")?;
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

/// `POST /api/provider-accounts/{account_id}/usage/reset`: spend the
/// confirmed grant now. Requires the typed `confirm` confirmation.
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
    let reservation = match reserve(&state, owner, &id).await {
        Ok((reservation, _)) => reservation,
        Err(code) => return refusal(code),
    };
    let (lane, key) = (lane(&id), new_reset_idempotency_key());
    let expected = Some(request.expected_credit_id.as_str());
    log_reset(
        store,
        &lane,
        "manual",
        "started",
        "preflight",
        "Manual reset attempt started",
        expected,
        Some(&key),
    )
    .await;
    match consume(&state, owner, &id, reservation, &key, expected).await {
        Ok(result) => {
            log_reset(
                store,
                &lane,
                "manual",
                "succeeded",
                "provider_response",
                &result.outcome,
                result.credit_id.as_deref(),
                Some(&key),
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
            log_reset(
                store,
                &lane,
                "manual",
                status,
                phase,
                &message,
                error.credit_id.as_deref(),
                Some(&key),
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
    if let Err(code) = reset_target(&state, owner, &id).await {
        return refusal(code);
    }
    if let Err(code) = persist_schedule(store, owner, &id, request.fire_at_ms).await {
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
            persist_schedule(&store, "intruder", &account, fire_at).await,
            Err("account_unavailable")
        );
        assert!(schedule_of(&store, &account).await.is_none());
        persist_schedule(&store, "owner", &account, fire_at)
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
}

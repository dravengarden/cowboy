//! MCP dispatch uses a durable fence, never an event buffer as a receipt.
use super::*;
use crate::store::provider_accounts::logins::{DispatchRejection, Receipt, Request};

/// Cancel only the request originally reserved by this owner and parent-scoped
/// operation. No caller-supplied request ID, account or Machine is accepted.
/// The response proves Machine acceptance only, never process exit or logout.
pub(in crate::server) async fn cancel_operation(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
) -> Result<Value, &'static str> {
    let receipt = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .cancel_provider_login(owner, operation)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")?;
    let executor = {
        let mut requests = state.provider_auth_executors.lock();
        if let Some(executor) = requests.get_mut(&receipt.request_id) {
            if !executor.owned_by(owner, &receipt.provider_id)
                || executor.account_id != receipt.input.account_id
                || executor.machine_id != receipt.input.machine_id
            {
                return Err("authentication_receipt_mismatch");
            }
            executor.cancel_requested = true;
            Some(executor.clone())
        } else {
            None
        }
    };
    let Some(executor) = executor else {
        if super::native_cancel::recover(
            state.store.as_ref().ok_or("unavailable")?,
            &state.machine_control,
            &receipt,
        )
        .await?
        {
            return Ok(
                json!({"status":"cancellation_requested", "request_id":receipt.request_id,"cancel_requested":true}),
            );
        }
        return observation(state, owner, &receipt).await;
    };
    super::cancel(state, &executor, &receipt.request_id)
        .await
        .map_err(|_| "authentication_cleanup_required")?;
    super::send_confirmed_cancel(
        &state.machine_control,
        &state.provider_auth_executors,
        &executor,
        &receipt.request_id,
    )
    .await
    .map_err(|_| "authentication_cleanup_required")?;
    Ok(
        json!({"status":"cancellation_requested", "request_id":receipt.request_id,"cancel_requested":true}),
    )
}

fn grant_phase(
    login: &Receipt,
    grant: Option<&crate::store::provider_accounts::grants::Receipt>,
) -> Result<Value, &'static str> {
    let Some(grant) = grant else {
        return Ok(Value::Null);
    };
    if grant.operation_id != login.request_id
        || grant.account_id != login.input.account_id
        || grant.provider_id != login.provider_id
        || grant.machine_id != login.input.machine_id
    {
        return Err("authentication_receipt_mismatch");
    }
    // Historical metadata only: neither current enrollment nor live worker
    // authority can be inferred from a committed grant.
    serde_json::to_value(&grant.phase).map_err(|_| "unavailable")
}

pub(super) async fn observation(
    state: &AppState,
    owner: &str,
    receipt: &Receipt,
) -> Result<Value, &'static str> {
    let grant = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .provider_grant_receipt(owner, &receipt.request_id)
        .await
        .map_err(|_| "unavailable")?;
    let grant = grant_phase(receipt, grant.as_ref())?;
    if receipt.dispatch_rejection.is_some()
        && (!grant.is_null() || receipt.native_completion.is_some())
    {
        return Err("authentication_receipt_mismatch");
    }
    let native_status =
        super::native_status::observe(state.store.as_ref().ok_or("unavailable")?, owner, receipt)
            .await?;
    let requests = state.provider_auth_executors.lock();
    let status =
        requests
            .get(&receipt.request_id)
            .filter(|executor| {
                executor.owned_by(owner, &receipt.provider_id)
                    && executor.account_id == receipt.input.account_id
                    && executor.machine_id == receipt.input.machine_id
            })
            .map_or("unknown", |executor| {
                if executor.cancel_requested {
                    "cancelling"
                } else if !executor.active(&state.machine_control, now_ms()) {
                    "inactive"
                } else if executor.named_login.as_ref().is_some_and(|login| {
                    login.completion.try_lock().is_ok_and(|complete| *complete)
                }) {
                    "completed"
                } else {
                    "active"
                }
            });
    let mut value = json!({"status":native_status.unwrap_or(status),"dispatch_reserved":true,"cancel_requested":receipt.cancel_requested,"request_id":receipt.request_id,
    "provider_id":receipt.provider_id,"account_id":receipt.input.account_id,"machine_id":receipt.input.machine_id,
        "historical_grant":grant,"native_generation":receipt.native_generation,
        "native_binding_revision":receipt.native_binding_revision,
    "continue_url":format!("/#subscription-sign-in/{}/{}", receipt.input.account_id.as_str(), receipt.request_id),
    "native_completion":receipt.native_completion.as_ref().map(|completion| json!({
        "state":"recorded", "generation":completion.profile.generation
    }))});
    if let Some(reason) = receipt.dispatch_rejection {
        if status != "unknown" {
            return Err("authentication_receipt_mismatch");
        }
        value["status"] = json!("not_started");
        value["error"] = json!({"code":reason});
    }
    Ok(value)
}

pub(in crate::server) async fn operation_receipt(
    state: &AppState,
    owner: &str,
    operation: &str,
) -> Result<Value, &'static str> {
    let receipt = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .provider_login_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?;
    match receipt {
        Some(receipt) => {
            let recovered = super::native_recovery::observe(state, owner, &receipt).await?;
            observation(state, owner, &recovered).await
        }
        None => Ok(json!({"status":"unknown","dispatch_reserved":false})),
    }
}

pub(in crate::server) async fn start_operation(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    input: Request,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    if let Some(receipt) = store
        .provider_login_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?
    {
        if !receipt.matches(&input) {
            return Err("operation_reused");
        }
        let recovered = super::native_recovery::observe(state, owner, &receipt).await?;
        return observation(state, owner, &recovered).await;
    }
    let start = Start {
        operation: operation.into(),
        machine_id: input.machine_id.clone(),
        provider_version: input.provider_version.clone(),
        generation_digest: input.generation_digest.clone(),
        method: input.method.clone(),
    };
    // Preparation validates the exact current signed release and recipient,
    // but does not launch a native login. Unsupported releases consume no ID.
    let (executor, package) = prepare(state, owner, input.account_id.clone(), &start)
        .await
        .map_err(|_| "login_not_available")?;
    let claim = store
        .claim_provider_login(owner, operation, &input)
        .await
        .map_err(|_| "outcome_unknown")?;
    if claim.fresh {
        // Only the fresh durable claimant can enqueue. Never restore dispatch
        // authority from a receipt, including after a crash before this point.
        if retire_expired(state, &executor).await.is_err() {
            let receipt = store
                .reject_provider_login(owner, operation, &claim, DispatchRejection::CleanupRequired)
                .await
                .map_err(|_| "outcome_unknown")?;
            return observation(state, owner, &receipt).await;
        }
        let (revision, recipient) = if let Some(login) = &executor.native_login {
            (login.registry_revision, &login.recipient_key)
        } else if let Some(login) = &executor.named_login {
            (login.registry_revision, &login.recipient_key)
        } else {
            return Err("named_login_required");
        };
        let scope = crate::store::provider_accounts::logins::DispatchScope {
            registry_revision: revision,
            recipient_fingerprint: crate::admin::hex_sha256(recipient.as_bytes()),
        };
        let response = store
            .with_provider_login_dispatch(owner, operation, &claim, &scope, || {
                Ok(if claim.receipt.native_generation.is_some() {
                    super::native::begin_reserved(
                        &state.machine_control,
                        &state.provider_auth_executors,
                        executor,
                        &claim,
                    )
                } else {
                    begin_reserved(
                        &state.machine_control,
                        &state.provider_auth_executors,
                        executor,
                        &package,
                        Some(&claim.receipt.request_id),
                    )
                })
            })
            .await
            .map_err(|_| "outcome_unknown")?;
        if !response.status().is_success() {
            let receipt = store
                .reject_provider_login(owner, operation, &claim, DispatchRejection::StartRejected)
                .await
                .map_err(|_| "outcome_unknown")?;
            return observation(state, owner, &receipt).await;
        }
    }
    // Cancellation may commit after claim but before executor registration.
    // Recheck after registration: a concurrent canceller either sees the
    // executor, or leaves this durable intent for the fresh claimant to apply.
    let current = store
        .provider_login_receipt(owner, operation)
        .await
        .map_err(|_| "outcome_unknown")?
        .ok_or("outcome_unknown")?;
    if current.cancel_requested {
        return cancel_operation(state, owner, operation).await;
    }
    observation(state, owner, &current).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::provider_accounts::grants::{Phase, Receipt as Grant};

    fn fixture() -> (Receipt, Grant) {
        let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let login = Receipt {
            request_id: "request".into(),
            provider_id: "provider".into(),
            cancel_requested: false,
            dispatch_rejection: None,
            native_completion: None,
            native_logout: None,
            native_generation: None,
            native_binding_revision: None,
            dispatch_recipient_fingerprint: None,
            dispatch_registry_revision: None,
            input: Request {
                account_id: account,
                machine_id: "machine".into(),
                provider_version: "1".into(),
                generation_digest: "digest".into(),
                method: "native".into(),
            },
        };
        let grant = Grant {
            operation_id: login.request_id.clone(),
            account_id: login.input.account_id.clone(),
            provider_id: login.provider_id.clone(),
            machine_id: login.input.machine_id.clone(),
            recipient_fingerprint: "private-recipient".into(),
            candidate_fingerprint: "private-candidate".into(),
            identity_fingerprint: "private-identity".into(),
            expected_generation: 0,
            registry_revision: 1,
            phase: Phase::Prepared,
        };
        (login, grant)
    }

    #[test]
    fn historical_grant_reports_only_phase_and_generation() {
        let (login, mut grant) = fixture();
        assert_eq!(grant_phase(&login, None).unwrap(), Value::Null);
        for (phase, expected) in [
            (Phase::Prepared, json!({"state":"prepared"})),
            (
                Phase::Committed { generation: 7 },
                json!({"state":"committed","generation":7}),
            ),
            (Phase::Cancelled, json!({"state":"cancelled"})),
        ] {
            grant.phase = phase;
            assert_eq!(grant_phase(&login, Some(&grant)).unwrap(), expected);
        }
    }

    #[test]
    fn historical_grant_must_match_the_exact_login_scope() {
        let (login, original) = fixture();
        for field in 0..4 {
            let mut grant = original.clone();
            match field {
                0 => grant.operation_id = "other".into(),
                1 => grant.account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap(),
                2 => grant.provider_id = "other".into(),
                _ => grant.machine_id = "other".into(),
            }
            assert_eq!(
                grant_phase(&login, Some(&grant)),
                Err("authentication_receipt_mismatch")
            );
        }
    }
}

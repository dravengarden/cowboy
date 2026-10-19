//! Metadata-only native completion ingress. No portable vault operations.
use super::*;
use crate::machine_control::ConnectionToken;
use crate::provider_account::NativeProfileReceipt;
use crate::store::provider_accounts::logins::NativeCompletion;

pub(super) fn begin_reserved(
    control: &MachineControl,
    requests: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    mut executor: ProviderAuthExecutor,
    claim: &crate::store::provider_accounts::logins::Claim,
) -> Response {
    let receipt = &claim.receipt;
    let Some(generation) = receipt.native_generation.filter(|value| *value > 0) else {
        return (StatusCode::CONFLICT, "native_generation_required").into_response();
    };
    if !claim.fresh
        || receipt.cancel_requested
        || receipt.dispatch_rejection.is_some()
        || receipt.native_completion.is_some()
        || executor.native_login.is_none()
        || executor.named_login.is_some()
        || receipt.provider_id != "claude-code"
        || !executor.active(control, now_ms())
        || !executor.accepts_candidate(
            &receipt.input.machine_id,
            &receipt.input.account_id,
            &receipt.provider_id,
            &receipt.input.method,
            &receipt.input.provider_version,
            &receipt.input.generation_digest,
            &executor.auth_contract_fingerprint,
        )
    {
        return (StatusCode::CONFLICT, "native_login_claim_mismatch").into_response();
    }
    let mut executors = requests.lock();
    if executors.len() >= 1024
        || executors.contains_key(&receipt.request_id)
        || executors.values().any(|old| {
            old.account_id == executor.account_id && old.machine_id == executor.machine_id
        })
    {
        return (StatusCode::CONFLICT, "authentication_already_active").into_response();
    }
    executor.expected_generation = generation - 1;
    let command = crate::machine_protocol::MachineCommand::BeginNativeLogin {
        request_id: receipt.request_id.clone(),
        provider: executor.provider_id.clone(),
        account_id: executor.account_id.clone(),
        spec: crate::provider_account::NativeLoginSpec {
            generation,
            method: executor.auth_method.clone(),
            provider_version: executor.provider_version.clone(),
            generation_digest: executor.generation_digest.clone(),
        },
    };
    if let Err(error) = control.send_on_connection(&executor.connection, command) {
        return (StatusCode::CONFLICT, error).into_response();
    }
    let response = (
        StatusCode::ACCEPTED,
        Json(json!({
            "request_id":receipt.request_id, "account_id":executor.account_id,
            "provider_id":executor.provider_id, "machine_id":executor.machine_id,
            "native_generation":generation, "expires_at_ms":executor.expires_at_ms, "resumed":false,
        })),
    )
        .into_response();
    executors.insert(receipt.request_id.clone(), executor);
    response
}

pub(super) fn validate_executor(
    executor: &ProviderAuthExecutor,
    control: &MachineControl,
    connection: &ConnectionToken,
    profile: &NativeProfileReceipt,
) -> Result<(), &'static str> {
    if profile.account_id.is_default()
        || executor.native_login.is_none()
        || executor.named_login.is_some()
        || profile.provider_id != "claude-code"
        || !executor.connection.same(connection)
        || !executor.active(control, now_ms())
        || executor.expected_generation.checked_add(1) != Some(profile.generation)
        || !executor.accepts_candidate(
            connection.machine_id(),
            &profile.account_id,
            &profile.provider_id,
            &profile.method,
            &profile.provider_version,
            &profile.generation_digest,
            &profile.auth_contract_fingerprint,
        )
    {
        return Err("native_completion_request_mismatch");
    }
    Ok(())
}

pub(in crate::server) async fn accept_native_completion(
    state: &Arc<AppState>,
    connection: &ConnectionToken,
    profile: &NativeProfileReceipt,
) -> Result<(), &'static str> {
    let executor = state
        .provider_auth_executors
        .lock()
        .get(&profile.request_id)
        .cloned()
        .ok_or("authentication_request_unavailable")?;
    validate_executor(&executor, &state.machine_control, connection, profile)?;
    let package = state
        .provider_catalog
        .package(
            &profile.provider_id,
            &profile.provider_version,
            &profile.generation_digest,
        )
        .ok_or("release_unavailable")?;
    if package.manifest.compatibility.auth_contract_fingerprint != profile.auth_contract_fingerprint
    {
        return Err("authentication_contract_mismatch");
    }
    let method = package
        .manifest
        .authentication
        .methods
        .iter()
        .find(|method| method.id == profile.method)
        .ok_or("authentication_method_unavailable")?;
    if method.native_identity.is_none()
        || !matches!(
            method.executor,
            cowboy_provider_sdk::AuthExecutor::CommandV1 { .. }
        )
    {
        return Err("native_identity_verification_required");
    }
    validate_native_evidence(
        &profile.provider_id,
        method.native_identity.as_ref(),
        Some(&profile.identity),
    )?;
    let login = executor
        .native_login
        .as_ref()
        .ok_or("named_login_required")?;
    let completion = NativeCompletion {
        machine_id: connection.machine_id().into(),
        recipient_fingerprint: crate::admin::hex_sha256(login.recipient_key.as_bytes()),
        profile: profile.clone(),
    };
    let store = state.store.as_ref().ok_or("persistence_required")?;
    let operation = store
        .provider_login_operation_for_request(&executor.owner_user_id, &profile.request_id)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")?;
    // Recheck after database lookup. Cancellation is also serialized against
    // the DB writer; connection replacement must not lend authority to replay.
    {
        let executors = state.provider_auth_executors.lock();
        let current = executors
            .get(&profile.request_id)
            .ok_or("authentication_request_unavailable")?;
        validate_executor(current, &state.machine_control, connection, profile)?;
    }
    store
        .record_native_login_completion(&executor.owner_user_id, &operation, &completion)
        .await
        .map_err(|_| "native_completion_not_recorded")?;
    let registry = store
        .bind_native_login_identity(
            &executor.owner_user_id,
            &operation,
            &completion,
            login.registry_revision,
        )
        .await
        .map_err(|_| "native_identity_not_bound")?;
    store
        .with_native_login_binding(&executor.owner_user_id, &operation, &completion, || {
            let executors = state.provider_auth_executors.lock();
            let current = executors
                .get(&profile.request_id)
                .ok_or_else(|| anyhow::anyhow!("authentication_request_unavailable"))?;
            validate_executor(current, &state.machine_control, connection, profile)
                .map_err(anyhow::Error::msg)?;
            state.hub.announce_preferences(
                &executor.owner_user_id,
                "provider_accounts",
                registry.revision,
            );
            Ok(())
        })
        .await
        .map_err(|_| "native_binding_not_current")?;
    // Identity binding neither marks SignedIn nor issues a runtime grant.
    Ok(())
}

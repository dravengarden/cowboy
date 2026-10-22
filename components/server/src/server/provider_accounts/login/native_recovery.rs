//! Recover native completion and identity binding, never dispatch or grant runtime.
use super::*;
use crate::store::provider_accounts::logins::{NativeCompletion, Receipt};

pub(super) async fn observe(
    state: &AppState,
    owner: &str,
    receipt: &Receipt,
) -> Result<Receipt, &'static str> {
    if receipt.provider_id != "claude-code"
        || receipt.cancel_requested
        || receipt.dispatch_rejection.is_some()
        || receipt.native_generation.is_none()
        || receipt.input.account_id.is_default()
    {
        return Ok(receipt.clone());
    }
    let Some(fingerprint) = &receipt.dispatch_recipient_fingerprint else {
        return Ok(receipt.clone()); // Historical absence cannot establish the original recipient.
    };
    if receipt.native_completion.is_some() {
        return finish_binding(state, owner, receipt).await;
    }
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    if !registry.accounts.iter().any(|account| {
        account.id == receipt.input.account_id
            && account.provider_id == receipt.provider_id
            && account.available()
    }) {
        return Err("account_unavailable");
    }
    let key = store
        .machine_encryption_public_key(&receipt.input.machine_id)
        .await
        .map_err(|_| "unavailable")?;
    if !key.is_some_and(|key| crate::admin::hex_sha256(key.as_bytes()) == *fingerprint) {
        return Err("machine_enrollment_changed");
    }
    let Some((connection, _)) = state
        .machine_control
        .connection_protocol(&receipt.input.machine_id)
    else {
        return Ok(receipt.clone());
    };
    let Some(profile) = state
        .machine_control
        .query_native_profile(&connection, receipt)
        .await
        .map_err(|_| "native_observation_unavailable")?
    else {
        return Ok(receipt.clone());
    };
    let package = state
        .provider_catalog
        .package(
            &receipt.provider_id,
            &receipt.input.provider_version,
            &receipt.input.generation_digest,
        )
        .ok_or("release_unavailable")?;
    if profile.auth_contract_fingerprint != package.manifest.compatibility.auth_contract_fingerprint
    {
        return Err("authentication_contract_mismatch");
    }
    let method = package
        .manifest
        .authentication
        .methods
        .iter()
        .find(|method| method.id == receipt.input.method)
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
        &receipt.provider_id,
        method.native_identity.as_ref(),
        Some(&profile.identity),
    )?;
    let operation = store
        .provider_login_operation_for_request(owner, &receipt.request_id)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")?;
    if !state.machine_control.is_current(&connection) {
        return Err("authentication_connection_changed");
    }
    // The DB writer rechecks owner, request, generation, cancellation, account
    // admission and original enrollment. This is historical evidence only.
    let recorded = store
        .record_native_login_completion(
            owner,
            &operation,
            &NativeCompletion {
                machine_id: receipt.input.machine_id.clone(),
                recipient_fingerprint: fingerprint.clone(),
                profile,
            },
        )
        .await
        .map_err(|_| "native_completion_not_recorded")?;
    finish_binding(state, owner, &recorded).await
}

async fn finish_binding(
    state: &AppState,
    owner: &str,
    receipt: &Receipt,
) -> Result<Receipt, &'static str> {
    // Binding is a separate durable stage. A crash between these stages must
    // not require another login or silently adopt a newer admission revision.
    let (Some(revision), Some(completion)) = (
        receipt.dispatch_registry_revision,
        &receipt.native_completion,
    ) else {
        return Ok(receipt.clone());
    };
    if receipt.native_binding_revision.is_some() {
        return Ok(receipt.clone());
    }
    if receipt.dispatch_recipient_fingerprint.as_ref() != Some(&completion.recipient_fingerprint) {
        return Err("machine_enrollment_changed");
    }
    let store = state.store.as_ref().ok_or("unavailable")?;
    let operation = store
        .provider_login_operation_for_request(owner, &receipt.request_id)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")?;
    let registry = store
        .bind_native_login_identity(owner, &operation, completion, revision)
        .await
        .map_err(|_| "native_identity_not_bound")?;
    state
        .hub
        .announce_preferences(owner, "provider_accounts", registry.revision);
    store
        .provider_login_receipt(owner, &operation)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")
}

use super::*;
use crate::store::provider_accounts::grants::{Outcome, Phase, Prepare, Receipt};

pub(in crate::server) struct Candidate<'a> {
    pub machine: &'a str,
    pub request: &'a str,
    pub provider: &'a str,
    pub account: &'a ProviderAccountId,
    pub identity: Option<&'a crate::provider_account::NativeIdentity>,
    pub version: &'a str,
    pub digest: &'a str,
    pub contract: &'a str,
    pub bundle: crate::machine_protocol::PortableCredentialBundle,
}

pub(in crate::server) async fn cancel(
    state: &AppState,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    if let Some(scope) = executor.auth_scope(state) {
        scope.cancel_authentication(&executor.provider_id, request);
    }
    let Some(login) = &executor.named_login else {
        return Ok(());
    };
    let store = state.store.as_ref().ok_or("persistence_required")?;
    let Some(mut intent) = store
        .provider_grant_receipt(&executor.owner_user_id, request)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    intent.phase = Phase::Prepared;
    let receipt = match store
        .cancel_provider_grant(&executor.owner_user_id, &intent)
        .await
        .map_err(|e| e.to_string())?
    {
        Outcome::Applied(receipt) => receipt,
        Outcome::Rejected(code) => return Err(code.into()),
    };
    if receipt.phase == Phase::Cancelled {
        login
            .scope
            .discard_cancelled_grant(&receipt)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn snapshot(state: &AppState, candidate: &Candidate<'_>) -> Result<ProviderAuthExecutor, String> {
    let executors = state.provider_auth_executors.lock();
    let executor = executors
        .get(candidate.request)
        .ok_or("authentication_request_unavailable")?;
    if !executor.active(&state.machine_control, now_ms())
        || executor.named_login.is_none()
        || !executor.accepts_candidate(
            candidate.machine,
            candidate.account,
            candidate.provider,
            &candidate.bundle.method_id,
            candidate.version,
            candidate.digest,
            candidate.contract,
        )
    {
        return Err("authentication_candidate_mismatch".into());
    }
    Ok(executor.clone())
}

pub(in crate::server) async fn accept(
    state: &Arc<AppState>,
    candidate: Candidate<'_>,
) -> Result<(), String> {
    crate::provider_account::portable_credentials_allowed(candidate.account, candidate.provider)?;
    let executor = snapshot(state, &candidate)?;
    let login = executor
        .named_login
        .as_ref()
        .ok_or("named_login_required")?;
    let package = state
        .provider_catalog
        .package(candidate.provider, candidate.version, candidate.digest)
        .ok_or("release_unavailable")?;
    if package.manifest.compatibility.auth_contract_fingerprint != candidate.contract {
        return Err("authentication_contract_mismatch".into());
    }
    let method = package
        .manifest
        .authentication
        .methods
        .iter()
        .find(|method| method.id == candidate.bundle.method_id)
        .ok_or("authentication_method_unavailable")?;
    if method.native_identity.is_none() {
        return Err("native_identity_verification_required".into());
    }
    validate_native_evidence(
        candidate.provider,
        method.native_identity.as_ref(),
        candidate.identity,
    )
    .map_err(str::to_owned)?;
    let identity = candidate.identity.ok_or("native_identity_required")?;
    let fingerprint = crate::provider_service::grants::candidate_fingerprint(
        &package,
        &candidate.bundle,
        &identity
            .fingerprint(candidate.provider)
            .map_err(str::to_owned)?,
    )
    .map_err(|e| e.to_string())?;
    let store = state.store.as_ref().ok_or("persistence_required")?;
    if executor.promotion_started {
        let receipt = store
            .provider_grant_receipt(&executor.owner_user_id, candidate.request)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("authentication_operation_unavailable")?;
        if receipt.candidate_fingerprint != fingerprint {
            return Err("authentication_candidate_mismatch".into());
        }
        return recover_pending(state, &executor, candidate.request).await;
    }
    let outcome = store
        .prepare_provider_grant(
            &executor.owner_user_id,
            candidate.request,
            &Prepare {
                expected_revision: login.registry_revision,
                account_id: executor.account_id.clone(),
                provider_id: executor.provider_id.clone(),
                machine_id: executor.machine_id.clone(),
                recipient_fingerprint: crate::admin::hex_sha256(login.recipient_key.as_bytes()),
                candidate_fingerprint: fingerprint,
                expected_generation: executor.expected_generation,
                identity: identity.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let intent = match outcome {
        Outcome::Applied(receipt) => receipt,
        Outcome::Rejected(code) => return Err(code.into()),
    };
    if intent.phase != Phase::Prepared {
        return Err("authentication_operation_resolved".into());
    }
    let proof = commit_current(state, &candidate, &package, &intent);
    let proof = match proof {
        Ok(Some(proof)) => proof,
        Ok(None) => return Ok(()),
        Err(error) => {
            // Cancellation can win before the async preparation finishes.
            // Reconcile its newly created intent now, without reviving login.
            if snapshot(state, &candidate).is_err() {
                cancel(state, &executor, candidate.request).await?;
            }
            return Err(error);
        }
    };
    let recovery = login
        .scope
        .finish_grant(store, &executor.owner_user_id, &intent, &proof)
        .await
        .map_err(|e| e.to_string())?;
    if !matches!(recovery.receipt.phase, Phase::Committed { .. }) {
        return Err("authentication_cancelled".into());
    }
    recover_pending(state, &executor, candidate.request).await
}

pub(in crate::server) async fn recover_pending(
    state: &Arc<AppState>,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    let login = executor
        .named_login
        .as_ref()
        .ok_or("named_login_required")?;
    login
        .complete(|| ensure_active(state, request), async {
            let store = state.store.as_ref().ok_or("persistence_required")?;
            let recovery = state
                .provider_auth
                .reconcile_grant(
                    store,
                    &executor.owner_user_id,
                    request,
                    &executor.machine_id,
                    &login.recipient_key,
                )
                .await
                .map_err(|e| e.to_string())?;
            if !matches!(recovery.receipt.phase, Phase::Committed { .. }) {
                return Err("authentication_not_committed".into());
            }
            ensure_active(state, request)?;
            publish_committed(state, executor, request).await?;
            Ok(())
        })
        .await?;
    record_completion(&state.machine_control, executor, request);
    Ok(())
}

fn ensure_active(state: &AppState, request: &str) -> Result<(), String> {
    if state
        .provider_auth_executors
        .lock()
        .get(request)
        .is_some_and(|executor| executor.active(&state.machine_control, now_ms()))
    {
        Ok(())
    } else {
        Err("authentication_request_unavailable".into())
    }
}

async fn publish_committed(
    state: &Arc<AppState>,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    sync_and_mark_account_grant(
        state,
        &executor.machine_id,
        &executor.provider_id,
        &executor.account_id,
    )
    .await?;
    ensure_active(state, request)?;
    finalize(&state.machine_control, executor, request).await?;
    ensure_active(state, request)?;
    Ok(())
}

pub(super) fn record_completion(
    control: &MachineControl,
    executor: &ProviderAuthExecutor,
    request: &str,
) {
    if control.events(&executor.machine_id).iter().any(|event| {
        matches!(event, crate::machine_protocol::MachineEvent::LoginState {
            request_id, state: crate::machine_protocol::AuthState::SignedIn, ..
        } if request_id == request)
    }) {
        return;
    }
    control.record_service_login(
        &executor.machine_id,
        crate::machine_control::ServiceLoginNotice {
            request_id: request.into(),
            provider: executor.provider_id.clone(),
            state: crate::machine_protocol::AuthState::SignedIn,
            account_label: None,
            detail: Some(
                "Subscription grant committed; temporary login credentials removed".into(),
            ),
        },
    );
}

pub(super) async fn finalize(
    control: &MachineControl,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    let correlation = machine_request_id("provider-auth-finalize");
    control
        .command_on_connection(
            &executor.connection,
            correlation.clone(),
            crate::machine_protocol::MachineCommand::FinalizeProviderAuthCandidate {
                request_id: correlation,
                account_id: executor.account_id.clone(),
                provider_id: executor.provider_id.clone(),
                auth_method: executor.auth_method.clone(),
                candidate_request_id: request.into(),
            },
        )
        .await
        .map_err(|error| error.detail)
}

fn commit_current(
    state: &AppState,
    candidate: &Candidate<'_>,
    package: &cowboy_provider_sdk::ProviderPackage,
    intent: &Receipt,
) -> Result<Option<crate::provider_service::grants::VerifiedCommit>, String> {
    let mut executors = state.provider_auth_executors.lock();
    let executor = executors
        .get_mut(candidate.request)
        .ok_or("authentication_request_unavailable")?;
    if !executor.active(&state.machine_control, now_ms())
        || !executor.accepts_candidate(
            candidate.machine,
            candidate.account,
            candidate.provider,
            &candidate.bundle.method_id,
            candidate.version,
            candidate.digest,
            candidate.contract,
        )
    {
        return Err("authentication_request_expired".into());
    }
    let scope = Arc::clone(
        &executor
            .named_login
            .as_ref()
            .ok_or("named_login_required")?
            .scope,
    );
    executor.promote_once(|_| {
        scope
            .commit_grant(package, &candidate.bundle, intent)
            .map_err(|e| e.to_string())
    })
}

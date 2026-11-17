//! A named sign-in's credential candidate, committed to the account vault.
//!
//! The Machine ran the signed native sign-in method in a fresh candidate
//! home, read its identity with the signed recipe and exported the
//! credential under the method's portable twin. The Service binds that
//! identity to the account (the first sign-in binds it; a different one is
//! refused), commits the credential at account scope under the owner's
//! account lock and delivers it to every enrolled Machine. Refresh then
//! follows the Service compare-and-swap lineage like the default account.
use super::*;

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

/// Nothing durable exists before the account commit: cancelling only ends
/// the transient sign-in state. A committed sign-in is not undone.
#[allow(clippy::unused_async)] // Callers keep the historical async contract.
pub(in crate::server) async fn cancel(
    state: &AppState,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    if let Some(scope) = executor.auth_scope(state) {
        scope.cancel_authentication(&executor.provider_id, request);
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

/// Verify the candidate against its signed release and identity evidence:
/// returns the identity and the bundle under the portable twin method.
pub(super) fn verified(
    package: &cowboy_provider_sdk::ProviderPackage,
    account: &ProviderAccountId,
    native_method: &str,
    identity: Option<&crate::provider_account::NativeIdentity>,
    mut bundle: crate::machine_protocol::PortableCredentialBundle,
) -> Result<
    (
        crate::provider_account::NativeIdentity,
        crate::machine_protocol::PortableCredentialBundle,
    ),
    &'static str,
> {
    if account.is_default() {
        return Err("named_account_required");
    }
    let auth = &package.manifest.authentication;
    let method = auth
        .methods
        .iter()
        .find(|method| method.id == native_method)
        .ok_or("authentication_method_unavailable")?;
    let twin = crate::provider_account::portable_contract::portable_twin(auth, method)
        .ok_or("native_identity_verification_required")?;
    validate_native_evidence(
        &package.manifest.id,
        account,
        method.native_identity.as_ref(),
        identity,
    )?;
    if bundle.portable_schema != auth.portable_schema {
        return Err("authentication_contract_mismatch");
    }
    twin.id.clone_into(&mut bundle.method_id);
    Ok((identity.ok_or("native_identity_required")?.clone(), bundle))
}

/// The packages one account credential is committed for: the exact signed
/// release of `package`, and the newest release of every other member of
/// its account-vault scope.
pub(in crate::server) fn scope_packages(
    state: &AppState,
    package: &cowboy_provider_sdk::ProviderPackage,
) -> Vec<cowboy_provider_sdk::ProviderPackage> {
    let mut packages: Vec<_> = state
        .provider_catalog
        .portable_account_scope(&package.manifest.id)
        .iter()
        .filter(|member| **member != package.manifest.id)
        .filter_map(|member| state.provider_catalog.latest_package(member))
        .map(|(_, _, package)| package)
        .collect();
    packages.push(package.clone());
    packages
}

/// The executor is still this live request for exactly this candidate.
fn admitted(
    state: &AppState,
    executor: &ProviderAuthExecutor,
    request: &str,
    method: &str,
) -> bool {
    state
        .provider_auth_executors
        .lock()
        .get(request)
        .is_some_and(|current| {
            current.active(&state.machine_control, now_ms())
                && current.accepts_candidate(
                    &executor.machine_id,
                    &executor.account_id,
                    &executor.provider_id,
                    method,
                    &executor.provider_version,
                    &executor.generation_digest,
                    &executor.auth_contract_fingerprint,
                )
        })
}

/// Bind `identity` and commit `bundle` to the account vault, above every
/// native generation of the account (`Store::commit_subscription_sign_in`):
/// the request, its dispatch admission and enrollment, and the live
/// candidate are rechecked under the owner lock before anything binds, and
/// the login's receipt records the committed generation. `None` when this
/// request already committed.
async fn commit_signed_in(
    state: &AppState,
    executor: &ProviderAuthExecutor,
    request: &str,
    (method, identity): (&str, &crate::provider_account::NativeIdentity),
    (packages, bundle): (
        &[cowboy_provider_sdk::ProviderPackage],
        &crate::machine_protocol::PortableCredentialBundle,
    ),
) -> Result<Option<Vec<crate::provider_service::ProviderAuthenticationStatus>>, String> {
    let store = state.store.as_ref().ok_or("persistence_required")?;
    let login = executor
        .named_login
        .as_ref()
        .ok_or("named_login_required")?;
    store
        .commit_subscription_sign_in(
            &executor.owner_user_id,
            &executor.account_id,
            identity,
            crate::store::provider_accounts::portable::SignInLogin {
                request_id: request,
                machine_id: &executor.machine_id,
            },
            || {
                anyhow::ensure!(
                    admitted(state, executor, request, method),
                    "authentication_request_expired"
                );
                Ok(())
            },
            |floor| {
                let mut executors = state.provider_auth_executors.lock();
                let current = executors
                    .get_mut(request)
                    .context("authentication_request_expired")?;
                let statuses = current
                    .promote_once(|_| {
                        login
                            .scope
                            .commit_account(packages, bundle, &executor.provider_id, floor)
                    })
                    .inspect_err(|error| {
                        tracing::warn!(account = executor.account_id.as_str(), %error,
                            "rejected subscription sign-in credential");
                    })
                    .context("credential_rejected")?;
                let generation = login
                    .scope
                    .status(&executor.provider_id)
                    .map(|status| status.auth_generation)
                    .context("credential_rejected")?;
                Ok((statuses, generation))
            },
        )
        .await
        .map_err(|error| {
            let code = error.to_string();
            match code.as_str() {
                "native_identity_mismatch"
                | "native_identity_already_bound"
                | "account_unavailable"
                | "authentication_request_expired"
                | "authentication_request_unavailable"
                | "machine_enrollment_changed"
                | "login_admission_revision_changed"
                | "credential_rejected" => code,
                _ => "unavailable".into(),
            }
        })
}

pub(in crate::server) async fn accept(
    state: &Arc<AppState>,
    candidate: Candidate<'_>,
) -> Result<(), String> {
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
    let method = candidate.bundle.method_id.clone();
    let (identity, bundle) = verified(
        &package,
        candidate.account,
        &method,
        candidate.identity,
        candidate.bundle,
    )?;
    let packages = scope_packages(state, &package);
    let request = candidate.request;
    login
        .complete(|| ensure_active(state, request), async {
            let statuses = commit_signed_in(
                state,
                &executor,
                request,
                (&method, &identity),
                (&packages, &bundle),
            )
            .await?;
            if let Some(statuses) = statuses {
                distribute_and_mark_scope(state, &login.scope, &executor.account_id, statuses)
                    .await;
            }
            ensure_active(state, request)?;
            finalize(&state.machine_control, &executor, request).await
        })
        .await?;
    record_completion(&state.machine_control, &executor, request);
    Ok(())
}

/// A sign-in committed before an uncertain response: finish its delivery
/// and the candidate cleanup without committing again.
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
            if !executor.promotion_started {
                return Err("authentication_not_committed".into());
            }
            finalize(&state.machine_control, executor, request).await
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
            detail: Some("Signed in for every Machine; temporary login credentials removed".into()),
        },
    );
}

/// After a Controller restart a committed vault sign-in has no executor:
/// its durable receipt still lets the Machine remove the candidate home.
/// Best effort and idempotent; delivery recovers on Machine reconnect.
pub(super) async fn finalize_committed(
    state: &AppState,
    receipt: &crate::store::provider_accounts::logins::Receipt,
) {
    if receipt.vault_generation.is_none()
        || state
            .provider_auth_executors
            .lock()
            .contains_key(&receipt.request_id)
    {
        return;
    }
    let Ok(connection) = state
        .machine_control
        .operation_connection(&receipt.input.machine_id)
    else {
        return;
    };
    let correlation = machine_request_id("provider-auth-finalize");
    if let Err(error) = state
        .machine_control
        .command_on_connection(
            &connection,
            correlation.clone(),
            crate::machine_protocol::MachineCommand::FinalizeProviderAuthCandidate {
                request_id: correlation,
                account_id: receipt.input.account_id.clone(),
                provider_id: receipt.provider_id.clone(),
                auth_method: receipt.input.method.clone(),
                candidate_request_id: receipt.request_id.clone(),
            },
        )
        .await
    {
        tracing::debug!(detail = %error.detail, "committed sign-in cleanup remains pending");
    }
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

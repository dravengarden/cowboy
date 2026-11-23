//! Named sign-in: the Provider CLI logs in on one Machine the owner picks.
//! A subscription's verified credential lands in the account's vault for
//! every Machine; a native-only Provider (Claude) keeps it in that Machine's
//! native home. Machine placement never selects a different account.
use super::*;
use crate::provider_account::ProviderAccountId;
use crate::provider_service::ProviderAuthService;

mod candidate;
mod native;
mod native_cancel;
mod native_events;
mod native_recovery;
mod native_status;
pub use native::accept_native_completion;
pub use native_events::poll as native_events;
pub mod continuation;
mod discovery;
mod operations;
mod options;
pub use continuation::{continue_request, http_continue, http_continue_cancel};
pub use discovery::{http_logins, logins_value};
pub use operations::{cancel_operation, operation_receipt, start_operation};
pub use options::{options, options_value};
#[cfg(test)]
mod tests;
pub use candidate::{Candidate, accept, cancel, recover_pending, scope_packages};

#[derive(Clone)]
pub struct NamedLogin {
    pub scope: Arc<ProviderAuthService>,
    pub completion: Arc<tokio::sync::Mutex<bool>>,
    pub registry_revision: i64,
    pub recipient_key: String,
}

/// A native-only Provider's login: credentials stay in the Machine's native
/// home and have no Controller vault scope.
#[derive(Clone)]
pub struct NativeLogin {
    pub recipient_key: String,
    pub registry_revision: i64,
}

/// Rollout admission is independent of credential storage. Adding a signed
/// identity recipe or upgrading a Machine must not open an unfinished lifecycle.
fn public_login_allowed(enabled: bool) -> Result<(), &'static str> {
    if !enabled {
        Err("native_login_lifecycle_not_enabled")
    } else {
        Ok(())
    }
}

/// Where a sign-in's credential lands. A native-only Provider (Claude) signs
/// in inside the Machine's per-account native home, whose generation the
/// fresh DB claim assigns during dispatch; nothing reaches a vault. Every
/// other sign-in runs on this Machine but lands in the account's own vault
/// for every Machine (never a Machine grant), which needs Machine protocol 47.
fn login_storage(
    root: &ProviderAuthService,
    account: &ProviderAccountId,
    provider: &str,
    vault_supported: bool,
    recipient_key: String,
    registry_revision: i64,
) -> Result<(Option<NativeLogin>, Option<NamedLogin>, u64), String> {
    if account.is_default() {
        return Err("named_account_required".into());
    }
    if crate::provider_account::native_only(provider) {
        // In particular, never call account() or machine_grant(): they
        // create portable storage even before credentials are written.
        return Ok((
            Some(NativeLogin {
                recipient_key,
                registry_revision,
            }),
            None,
            0,
        ));
    }
    if !vault_supported {
        return Err("machine_update_required".into());
    }
    let scope = root.account(account).map_err(|error| error.to_string())?;
    let expected_generation = scope
        .status(provider)
        .map_or(0, |status| status.auth_generation);
    Ok((
        None,
        Some(NamedLogin {
            scope,
            completion: Default::default(),
            registry_revision,
            recipient_key,
        }),
        expected_generation,
    ))
}

impl NamedLogin {
    async fn complete(
        &self,
        validate: impl FnOnce() -> Result<(), String>,
        work: impl std::future::Future<Output = Result<(), String>>,
    ) -> Result<(), String> {
        // Cloned executors share this lock; failures and cancelled futures
        // leave completion retryable without relying on bounded event history.
        let mut completed = self.completion.lock().await;
        validate()?;
        if !*completed {
            work.await?;
            *completed = true;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    operation: String,
    machine_id: String,
    provider_version: String,
    generation_digest: String,
    method: String,
}

pub async fn start(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account_id): Path<ProviderAccountId>,
    Json(request): Json<Start>,
) -> Response {
    let operation = match http_operation(&request.operation) {
        Ok(operation) => operation,
        Err(code) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":{"code":code}})),
            )
                .into_response();
        }
    };
    let input = crate::store::provider_accounts::logins::Request {
        account_id,
        machine_id: request.machine_id,
        provider_version: request.provider_version,
        generation_digest: request.generation_digest,
        method: request.method,
    };
    operation_response(start_operation(&state, &auth.principal.user_id, &operation, input).await)
}

fn http_operation(operation: &str) -> Result<String, &'static str> {
    if !crate::managed_calls::valid_id(operation) || matches!(operation, "." | "..") {
        return Err("invalid_operation");
    }
    Ok(format!(
        "http-login-{}",
        crate::admin::hex_sha256(operation.as_bytes())
    ))
}

fn operation_response(result: Result<Value, &'static str>) -> Response {
    match result {
        Ok(value) => ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response(),
        Err(code) => (
            if matches!(code, "unavailable" | "outcome_unknown") {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::CONFLICT
            },
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

pub async fn http_receipt(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(operation): Path<String>,
) -> Response {
    let operation = match http_operation(&operation) {
        Ok(operation) => operation,
        Err(code) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":{"code":code}})),
            )
                .into_response();
        }
    };
    operation_response(operation_receipt(&state, &auth.principal.user_id, &operation).await)
}

pub async fn http_cancel(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(operation): Path<String>,
) -> Response {
    let operation = match http_operation(&operation) {
        Ok(operation) => operation,
        Err(code) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":{"code":code}})),
            )
                .into_response();
        }
    };
    operation_response(cancel_operation(&state, &auth.principal.user_id, &operation).await)
}

async fn retire_expired(state: &Arc<AppState>, next: &ProviderAuthExecutor) -> Result<(), String> {
    let old = {
        let mut requests = state.provider_auth_executors.lock();
        requests
            .iter_mut()
            .find(|(_, old)| {
                old.account_id == next.account_id
                    && old.machine_id == next.machine_id
                    && old.owner_user_id == next.owner_user_id
                    && !old.active(&state.machine_control, now_ms())
            })
            .map(|(id, old)| {
                old.cancel_requested = true;
                (id.clone(), old.clone())
            })
    };
    if let Some((id, old)) = old {
        if old.native_login.is_some() {
            // A disconnected executor cannot command its old transport. Reuse
            // durable, enrollment-checked cancellation on the current Machine;
            // never discard its reservation merely because it disconnected.
            let operation = state
                .store
                .as_ref()
                .ok_or("persistence_required")?
                .provider_login_operation_for_request(&old.owner_user_id, &id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or("login_operation_not_found")?;
            cancel_operation(state, &old.owner_user_id, &operation)
                .await
                .map_err(str::to_owned)?;
            return Ok(());
        }
        cancel(state, &old, &id).await?;
        dispatch_cancel(
            &state.machine_control,
            &state.provider_auth_executors,
            &old,
            &id,
        )
        .await?;
    }
    Ok(())
}

/// Preserve the cancelled request when dispatch fails. Enqueue is only a
/// cancellation request, never proof that the native process has stopped.
pub fn send_cancel(
    control: &MachineControl,
    requests: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    let mut requests = requests.lock();
    let Some(current) = requests.get(request) else {
        return Ok(()); // A concurrent cancellation already removed it.
    };
    if !current.cancel_requested
        || !current.owned_by(&executor.owner_user_id, &executor.provider_id)
        || current.account_id != executor.account_id
        || current.machine_id != executor.machine_id
        || !current.connection.same(&executor.connection)
    {
        return Err("authentication_request_mismatch".into());
    }
    control.send_on_connection(
        &executor.connection,
        crate::machine_protocol::MachineCommand::CancelLogin {
            request_id: request.into(),
        },
    )?;
    requests.remove(request);
    Ok(())
}

/// Unlike the legacy enqueue-only command, this observes Machine acceptance
/// under a distinct correlation ID. It still cannot establish process exit.
pub async fn send_confirmed_cancel(
    control: &MachineControl,
    requests: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    {
        let requests = requests.lock();
        let current = requests
            .get(request)
            .ok_or("authentication_request_unavailable")?;
        if !current.cancel_requested
            || !current.owned_by(&executor.owner_user_id, &executor.provider_id)
            || current.account_id != executor.account_id
            || !current.connection.same(&executor.connection)
        {
            return Err("authentication_request_mismatch".into());
        }
    }
    let correlation = format!("login-cancel-{}", uuid::Uuid::new_v4());
    control
        .command_on_connection(
            &executor.connection,
            correlation.clone(),
            crate::machine_protocol::MachineCommand::CancelLoginRequest {
                request_id: correlation,
                login_request_id: request.into(),
            },
        )
        .await
        .map_err(|error| format!("{error:?}"))?;
    let mut requests = requests.lock();
    // The reply belongs to the original transport. A reconnect while it was
    // in flight must leave the request available for recovery.
    if !control.is_current(&executor.connection) {
        return Err("authentication_connection_changed".into());
    }
    if requests.get(request).is_some_and(|current| {
        !(current.cancel_requested
            && current.owned_by(&executor.owner_user_id, &executor.provider_id)
            && current.account_id == executor.account_id
            && current.machine_id == executor.machine_id
            && current.connection.same(&executor.connection))
    }) {
        return Err("authentication_request_mismatch".into());
    }
    requests.remove(request);
    Ok(())
}

/// Drop only the exact cancelled in-memory tracker after correlated acceptance.
/// This does not remove durable receipts or attest process/profile cleanup.
fn forget_cancelled_executor(
    requests: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    let mut requests = requests.lock();
    if requests.get(request).is_some_and(|current| {
        !(current.cancel_requested
            && current.owned_by(&executor.owner_user_id, &executor.provider_id)
            && current.account_id == executor.account_id
            && current.machine_id == executor.machine_id
            && current.connection.same(&executor.connection))
    }) {
        return Err("authentication_request_mismatch".into());
    }
    requests.remove(request);
    Ok(())
}

pub async fn dispatch_cancel(
    control: &MachineControl,
    requests: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    executor: &ProviderAuthExecutor,
    request: &str,
) -> Result<(), String> {
    if executor.account_id.is_default() {
        send_cancel(control, requests, executor, request)
    } else {
        send_confirmed_cancel(control, requests, executor, request).await
    }
}

async fn prepare(
    state: &Arc<AppState>,
    owner: &str,
    account_id: ProviderAccountId,
    request: &Start,
) -> Result<(ProviderAuthExecutor, cowboy_provider_sdk::ProviderPackage), String> {
    if account_id.is_default() {
        return Err("named_account_required".into());
    }
    let store = state.store.as_ref().ok_or("persistence_required")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|e| e.to_string())?;
    let account = registry
        .accounts
        .iter()
        .find(|a| a.id == account_id && a.available())
        .ok_or("account_unavailable")?;
    let package = state
        .provider_catalog
        .package(
            &account.provider_id,
            &request.provider_version,
            &request.generation_digest,
        )
        .ok_or("release_unavailable")?;
    public_login_allowed(
        state
            .service_config
            .get(&crate::config::schema::ACCOUNT_NATIVE_CLAUDE_LOGIN),
    )?;
    super::named_only_admitted(state, &account.provider_id, &account_id).await?;
    let method = package
        .manifest
        .authentication
        .methods
        .iter()
        .find(|m| m.id == request.method)
        .ok_or("authentication_method_unavailable")?;
    if method.native_identity.is_none()
        || crate::provider_account::portable_contract::portable_twin(
            &package.manifest.authentication,
            method,
        )
        .is_none()
    {
        return Err("native_identity_verification_unavailable".into());
    }
    if !state
        .supervisor
        .runtime_allowed(&account.provider_id, &request.machine_id)
    {
        return Err("machine_not_permitted".into());
    }
    let (connection, _) = state
        .machine_control
        .connection_protocol(&request.machine_id)
        .ok_or("machine_unavailable")?;
    let installed =
        current_machine_plugin(state, &request.machine_id, &account.provider_id).await?;
    if installed.plugin_version != request.provider_version
        || installed.generation_digest != request.generation_digest
    {
        return Err("installed_release_mismatch".into());
    }
    let recipient_key = store
        .machine_encryption_public_key(&request.machine_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("machine_recipient_unavailable")?;
    let (native_login, named_login, expected_generation) = login_storage(
        &state.provider_auth,
        &account_id,
        &account.provider_id,
        state.machine_control.machine_supports(
            &request.machine_id,
            crate::machine_protocol::PORTABLE_SUBSCRIPTION_PROTOCOL_VERSION,
        ),
        recipient_key,
        registry.revision,
    )?;
    Ok((
        ProviderAuthExecutor {
            cancel_requested: false,
            native_login,
            named_login,
            owner_user_id: owner.into(),
            account_id,
            machine_id: request.machine_id.clone(),
            connection,
            provider_id: account.provider_id.clone(),
            provider_version: request.provider_version.clone(),
            generation_digest: request.generation_digest.clone(),
            auth_method: request.method.clone(),
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            expected_generation,
            promotion_started: false,
            expires_at_ms: now_ms().saturating_add(15 * 60 * 1_000),
        },
        package,
    ))
}

#[cfg(test)]
fn begin(
    control: &MachineControl,
    requests: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    executor: ProviderAuthExecutor,
    package: &cowboy_provider_sdk::ProviderPackage,
) -> Response {
    begin_reserved(control, requests, executor, package, None)
}

fn begin_reserved(
    control: &MachineControl,
    requests: &parking_lot::Mutex<HashMap<String, ProviderAuthExecutor>>,
    mut executor: ProviderAuthExecutor,
    package: &cowboy_provider_sdk::ProviderPackage,
    reserved_id: Option<&str>,
) -> Response {
    let Some(login) = &executor.named_login else {
        return StatusCode::CONFLICT.into_response();
    };
    let mut executors = requests.lock();
    // Fence expired named requests; enqueue alone cannot release their slot.
    executors.retain(|id, old| {
        let Some(old_login) = &old.named_login else {
            return true;
        };
        if old.promotion_started || old.cancel_requested || old.active(control, now_ms()) {
            return true;
        }
        old_login.scope.cancel_authentication(&old.provider_id, id);
        old.cancel_requested = true;
        let _ = control.send_on_connection(
            &old.connection,
            crate::machine_protocol::MachineCommand::CancelLogin {
                request_id: id.clone(),
            },
        );
        true
    });
    if let Some((id, active)) = executors.iter().find(|(_, old)| {
        old.account_id == executor.account_id && old.machine_id == executor.machine_id
    }) {
        if reserved_id.is_some_and(|reserved| reserved != id) {
            return (StatusCode::CONFLICT, "authentication_already_active").into_response();
        }
        if !active.active(control, now_ms())
            || active
                .named_login
                .as_ref()
                .is_none_or(|old| old.recipient_key != login.recipient_key)
        {
            return (StatusCode::CONFLICT, "authentication_cleanup_required").into_response();
        }
        let request = ProviderAuthStartRequest {
            machine_id: Some(executor.machine_id.clone()),
            provider_version: executor.provider_version.clone(),
            generation_digest: executor.generation_digest.clone(),
            method: executor.auth_method.clone(),
        };
        return resumed_provider_authentication_response(
            id.clone(),
            active,
            &request,
            &executor.owner_user_id,
        );
    }
    if executors.len() >= 1024 {
        return (StatusCode::CONFLICT, "authentication_capacity_exceeded").into_response();
    }
    let id = reserved_id.map_or_else(|| machine_request_id("provider-login"), str::to_owned);
    if executors.contains_key(&id) {
        return (StatusCode::CONFLICT, "authentication_request_collision").into_response();
    }
    let scope = &login.scope;
    executor.expected_generation = scope
        .status(&executor.provider_id)
        .map_or(0, |status| status.auth_generation);
    if let Err(error) = scope.begin_authentication(package, &id) {
        return (StatusCode::CONFLICT, error.to_string()).into_response();
    }
    if let Err(error) = control.send_on_connection(
        &executor.connection,
        crate::machine_protocol::MachineCommand::BeginLogin {
            request_id: id.clone(),
            account_id: executor.account_id.clone(),
            provider: executor.provider_id.clone(),
            auth_method: Some(executor.auth_method.clone()),
        },
    ) {
        scope.cancel_authentication(&executor.provider_id, &id);
        return (StatusCode::CONFLICT, error).into_response();
    }
    let response = (StatusCode::ACCEPTED, Json(json!({"request_id":id,"account_id":executor.account_id,
        "provider_id":executor.provider_id,"machine_id":executor.machine_id,"method":executor.auth_method,
        "expires_at_ms":executor.expires_at_ms,"resumed":false}))).into_response();
    executors.insert(id, executor);
    response
}

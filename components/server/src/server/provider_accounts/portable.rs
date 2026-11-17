//! Named portable accounts: Providers whose only credential is a secret the
//! owner enters once (for example a DeepSeek API key). The key is committed
//! to the account's own Service vault, never to the default account or a
//! Machine grant, and is sealed to every enrolled Machine on delivery.
//! One account serves every portable Provider sharing its credential scope.
use super::*;
use crate::provider_account::ProviderAccountId;
use crate::provider_service::{ProviderAuthService, ProviderAuthenticationStatus};

const MAX_SECRET_BYTES: usize = 16 * 1024;

/// Whether `provider` takes named portable accounts at all.
pub(in crate::server) fn provider(state: &AppState, provider: &str) -> bool {
    !state
        .provider_catalog
        .portable_account_scope(provider)
        .is_empty()
}

/// The account vault that holds `provider`'s portable credential, if any.
/// Lookup-only; native-login Providers never resolve here.
pub(in crate::server) fn scope(
    state: &AppState,
    account: &ProviderAccountId,
    provider: &str,
) -> Option<Arc<ProviderAuthService>> {
    crate::provider_account::portable_credentials_allowed(account, provider).ok()?;
    state
        .provider_auth
        .existing_account(account)
        .filter(|scope| scope.status(provider).is_some())
}

/// Recheck at creation: the owner's account is available and serves
/// `provider`, and its vault holds a Ready key. The registry recheck runs
/// for every caller, including forks and inherited selections.
pub(in crate::server) async fn admitted_status(
    state: &AppState,
    owner: &str,
    account: &ProviderAccountId,
    provider: &str,
) -> Result<ProviderAuthenticationStatus, String> {
    let members = state.provider_catalog.portable_account_scope(provider);
    state
        .store
        .as_ref()
        .ok_or("account_store_unavailable")?
        .provider_accounts(owner)
        .await
        .map_err(|error| error.to_string())?
        .resolve_in_scope(provider, &members, Some(account))?;
    ready(scope(state, account, provider).and_then(|scope| scope.status(provider)))
}

fn ready(
    status: Option<ProviderAuthenticationStatus>,
) -> Result<ProviderAuthenticationStatus, String> {
    status
        .filter(|status| {
            status.authentication_state
                == crate::provider_service::ServiceAuthenticationState::Ready
        })
        .ok_or_else(|| {
            "This subscription has no API key. Enter one in Settings → Subscriptions.".into()
        })
}

/// The Service-held credential generation for scheduling. Machine inventory
/// reports only the default account's replica, so it is not consulted; a
/// Machine missing the account replica refuses the launch itself.
pub(in crate::server) fn scheduling_generation(
    required: bool,
    status: &ProviderAuthenticationStatus,
) -> Result<Option<u64>, String> {
    if !required {
        return Ok(None);
    }
    ready(Some(status.clone())).map(|status| Some(status.auth_generation))
}

/// Build the portable bundle for one signed secret-input method. The secret
/// is validated like the Machine's own secret executor and never echoed.
fn secret_bundle(
    package: &cowboy_provider_sdk::ProviderPackage,
    method: &str,
    secret: &str,
) -> Result<crate::machine_protocol::PortableCredentialBundle, &'static str> {
    let secret = secret.trim();
    if secret.is_empty() || secret.len() > MAX_SECRET_BYTES || secret.contains(['\0', '\r', '\n']) {
        return Err("invalid_secret");
    }
    let Some(cowboy_provider_sdk::AuthExecutor::SecretInputV1 { bundle_key, .. }) = package
        .manifest
        .authentication
        .methods
        .iter()
        .find(|candidate| candidate.id == method)
        .map(|method| &method.executor)
    else {
        return Err("authentication_method_unavailable");
    };
    Ok(crate::machine_protocol::PortableCredentialBundle {
        portable_schema: package.manifest.authentication.portable_schema.clone(),
        method_id: method.to_owned(),
        values: BTreeMap::from([(
            bundle_key.clone(),
            base64::engine::general_purpose::STANDARD.encode(secret.as_bytes()),
        )]),
    })
}

/// No `Debug`: the request carries the secret.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Credentials {
    method: String,
    secret: String,
}

pub(in crate::server) async fn http_commit(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account_id): Path<ProviderAccountId>,
    Json(request): Json<Credentials>,
) -> Response {
    match commit(&state, &auth.principal.user_id, &account_id, &request).await {
        Ok(value) => (
            StatusCode::CREATED,
            [(header::CACHE_CONTROL, "no-store")],
            Json(value),
        )
            .into_response(),
        Err(code) => (
            if code == "unavailable" {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

/// Commit a new key for every portable Provider in the account's scope at
/// the account's own vault, then deliver it. Never touches the default account.
pub(in crate::server) async fn commit(
    state: &Arc<AppState>,
    owner: &str,
    account_id: &ProviderAccountId,
    request: &Credentials,
) -> Result<Value, &'static str> {
    if account_id.is_default() {
        return Err("named_account_required");
    }
    let registry = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let account = registry
        .accounts
        .iter()
        .find(|account| &account.id == account_id && account.available())
        .ok_or("account_unavailable")?;
    let members = state
        .provider_catalog
        .portable_account_scope(&account.provider_id);
    let packages: Vec<_> = members
        .iter()
        .filter_map(|member| state.provider_catalog.latest_package(member))
        .map(|(_, _, package)| package)
        .collect();
    let package = packages
        .iter()
        .find(|package| package.manifest.id == account.provider_id)
        .ok_or("portable_account_provider_unavailable")?;
    let bundle = secret_bundle(package, &request.method, &request.secret)?;
    let scope = state
        .provider_auth
        .account(account_id)
        .map_err(|_| "unavailable")?;
    let statuses = scope
        .commit_shared(&packages, &bundle, None, &account.provider_id, None)
        .map_err(|error| {
            tracing::warn!(account = account_id.as_str(), %error, "rejected portable account credential");
            "credential_rejected"
        })?;
    let (statuses, replicas) = distribute_and_mark_scope(state, &scope, account_id, statuses).await;
    let count = |field: fn(&ProviderDistributionOutcome) -> usize| -> usize {
        replicas.values().map(field).sum()
    };
    Ok(
        json!({"status":"committed","account_id":account_id,"authentications":statuses,
        "replicas_succeeded":count(|value| value.succeeded),"replicas_failed":count(|value| value.failed),
        "replicas_pending":count(|value| value.pending)}),
    )
}

/// Deliver every account-scope portable credential to one (re)connected
/// Machine. Grants of native accounts are reconciled separately.
pub(in crate::server) async fn reconcile_machine(state: &Arc<AppState>, machine_id: &str) {
    for status in state.provider_auth.portable_account_statuses() {
        if let Err(error) = sync_provider_account_auth_to_machine(
            state,
            machine_id,
            &status.provider_id,
            &status.account_id,
        )
        .await
        {
            tracing::warn!(%error, %machine_id, account = status.account_id.as_str(), provider = %status.provider_id,
                "portable account credential delivery remains pending");
        }
    }
}

/// Extend each named account's key to scope members installed after it was
/// entered, like the default account does, then deliver the new entries.
pub(in crate::server) async fn adopt(state: &Arc<AppState>) {
    let accounts: BTreeSet<_> = state
        .provider_auth
        .portable_account_statuses()
        .into_iter()
        .map(|status| status.account_id)
        .collect();
    for account in accounts {
        let Some(scope) = state.provider_auth.existing_account(&account) else {
            continue;
        };
        for authentication in state.provider_catalog.authentication_scopes() {
            let packages: Vec<_> = state
                .provider_catalog
                .packages_for_authentication_scope(&authentication)
                .into_iter()
                .filter(|package| {
                    crate::provider_catalog::portable_account_contract(
                        &package.manifest.authentication,
                    )
                })
                .collect();
            match scope.adopt_shared_scope(&packages) {
                Ok(adopted) if !adopted.is_empty() => {
                    distribute_and_mark_scope(state, &scope, &account, adopted).await;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, account = account.as_str(), scope = %authentication,
                    "cannot extend a named account's shared credential")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(source: &str) -> cowboy_provider_sdk::ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(source).unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    #[test]
    fn an_entered_key_becomes_the_signed_bundle_without_echoing_it() {
        let deepseek = package(include_str!(
            "../../../../../plugins/codex-deepseek/provider.json"
        ));
        let bundle = secret_bundle(&deepseek, "api-key", "  sk-entered\n").unwrap();
        assert_eq!(bundle.method_id, "api-key");
        assert_eq!(bundle.portable_schema, "deepseek-api-key-v1");
        assert_eq!(
            bundle.values["api_key"],
            base64::engine::general_purpose::STANDARD.encode("sk-entered")
        );
        for (method, secret, code) in [
            ("api-key", "   ", "invalid_secret"),
            ("api-key", "sk\nother", "invalid_secret"),
            ("api-key", "sk\0", "invalid_secret"),
            ("missing", "sk-entered", "authentication_method_unavailable"),
        ] {
            let error = secret_bundle(&deepseek, method, secret).unwrap_err();
            assert_eq!(error, code);
            assert!(!error.contains("sk"));
        }
        let long = "k".repeat(MAX_SECRET_BYTES + 1);
        assert_eq!(
            secret_bundle(&deepseek, "api-key", &long),
            Err("invalid_secret")
        );
        // A native command login is never a secret entry.
        let native = package(include_str!(
            "../../../../../plugins/claude-code/provider.json"
        ));
        let method = native.manifest.authentication.methods[0].id.clone();
        assert_eq!(
            secret_bundle(&native, &method, "sk-entered"),
            Err("authentication_method_unavailable")
        );
    }

    #[test]
    fn credential_requests_accept_only_method_and_secret() {
        let value = json!({"method":"api-key","secret":"sk"});
        assert!(serde_json::from_value::<Credentials>(value.clone()).is_ok());
        for field in ["account_id", "provider_id", "machine_id", "owner"] {
            let mut extra = value.clone();
            extra[field] = json!("injected");
            assert!(serde_json::from_value::<Credentials>(extra).is_err());
        }
    }

    #[test]
    fn scheduling_needs_a_ready_account_key() {
        let mut status = ProviderAuthenticationStatus {
            provider_id: "codex-deepseek".into(),
            account_id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            recipient_key: None,
            auth_generation: 4,
            authentication_state: crate::provider_service::ServiceAuthenticationState::Ready,
            distribution_state: crate::provider_service::ServiceDistributionState::Current,
            auth_contract_fingerprint: "sha256:x".into(),
            authentication_scope: "deepseek-api-key-v1".into(),
            portable_schema: "deepseek-api-key-v1".into(),
            projection_schema: "codex-deepseek-gateway-v1".into(),
            account_label: None,
            updated_at_ms: 0,
        };
        assert_eq!(scheduling_generation(true, &status), Ok(Some(4)));
        assert_eq!(scheduling_generation(false, &status), Ok(None));
        for state in [
            crate::provider_service::ServiceAuthenticationState::SignedOut,
            crate::provider_service::ServiceAuthenticationState::Expired,
            crate::provider_service::ServiceAuthenticationState::Error,
        ] {
            status.authentication_state = state;
            let error = scheduling_generation(true, &status).unwrap_err();
            assert!(error.contains("Settings → Subscriptions"), "{error}");
        }
        assert!(ready(None).is_err());
    }
}

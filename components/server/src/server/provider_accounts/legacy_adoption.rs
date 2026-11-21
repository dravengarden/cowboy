//! Adopt a legacy default usage card (the Service "default" account of one
//! family) into a named account without signing in again. Service-only: no
//! Machine copies anything.
//!
//! 1. The registry gets an account whose id is the card's browser key, of
//!    the family's primary Provider and named by the legacy label; the
//!    card's browser enablement and share membership move to it.
//! 2. Under the owner's account lock, the Service vault moves the legacy
//!    root credential into the account's vault and wipes the root entries,
//!    atomically under the root vault lock (`move_root_scope_into`), then
//!    delivers both: the account credential and the root wipe.
//! 3. The account becomes the default of every member of its scope.
//!
//! Each stage has its own durable receipt derived from the caller's
//! operation, and every stage is idempotent: a replay resumes and never
//! repeats an effect or overwrites the account's credential.
use super::*;
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::legacy_adoption::Adoption;
use crate::store::provider_accounts::portable::Use;

/// The adopting family's primary Provider. Anthropic's legacy credential
/// has expired, so it is signed in again instead of adopted.
pub(in crate::server) fn primary_provider(card: &str) -> Result<&'static str, &'static str> {
    match card {
        "openai" => Ok("codex"),
        "xai" => Ok("grok"),
        "deepseek" => Ok("claude-deepseek"),
        "anthropic" => Err("legacy_credential_expired"),
        _ => Err("invalid_provider"),
    }
}

fn live(status: Option<crate::provider_service::ProviderAuthenticationStatus>) -> bool {
    status.is_some_and(|status| {
        status.authentication_state == crate::provider_service::ServiceAuthenticationState::Ready
    })
}

/// Each adoptable card's Provider and state for the account read:
/// `available` (a legacy credential to move), `resumable` (its account
/// exists and the legacy credential is still live) or `completed`.
pub(in crate::server) fn cards_value(
    state: &AppState,
    owner: &str,
    registry: &crate::store::provider_accounts::Registry,
) -> Value {
    let mut cards = serde_json::Map::new();
    for card in ["openai", "xai", "deepseek"] {
        let Ok(provider) = primary_provider(card) else {
            continue;
        };
        let root = state
            .provider_catalog
            .portable_account_scope(provider)
            .iter()
            .any(|member| live(state.provider_auth.status(member)));
        let id = crate::store::provider_accounts::legacy_browsers::account(owner, card);
        let adopted = registry.accounts.iter().any(|account| account.id == id);
        // Completed only once the account is the default and the legacy
        // credential is gone; anything short of that stays resumable.
        let defaulted = registry.defaults.get(provider) == Some(&id);
        let adoption = match (adopted, root, defaulted) {
            (false, false, _) => continue,
            (false, true, _) => "available",
            (true, false, true) => "completed",
            (true, _, _) => "resumable",
        };
        cards.insert(
            card.into(),
            json!({"provider_id":provider,"state":adoption,"account_id":id}),
        );
    }
    Value::Object(cards)
}

/// The owner's legacy display name of the card: its primary Provider's,
/// else any Provider of the card's, else the family name.
async fn label(
    state: &AppState,
    owner: &str,
    card: &str,
    provider: &str,
) -> Result<String, &'static str> {
    let preferences = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .preferences(owner, "subscriptions")
        .await
        .map_err(|_| "unavailable")?;
    let labels = preferences
        .as_ref()
        .and_then(|preferences| preferences.values.get("legacy_labels"))
        .and_then(Value::as_object);
    let cards = browser::legacy::bound_cards(state);
    let candidates =
        std::iter::once(provider).chain(cards.get(card).into_iter().flatten().map(String::as_str));
    Ok(candidates
        .filter_map(|provider| labels?.get(provider)?.as_str())
        .find(|label| !label.trim().is_empty())
        .map_or_else(|| format!("Legacy {card}"), str::to_owned))
}

fn applied(
    state: &AppState,
    owner: &str,
    result: crate::store::provider_accounts::MutationResult,
) -> Result<crate::store::provider_accounts::Receipt, &'static str> {
    match result {
        MutationResult::Applied(receipt) => {
            state
                .hub
                .announce_preferences(owner, "provider_accounts", receipt.registry.revision);
            Ok(receipt)
        }
        MutationResult::Invalid(code) => Err(code),
        MutationResult::Conflict(_) => Err("outcome_unknown"),
    }
}

/// Move the credential into the account vault and wipe the legacy one.
async fn move_credential(
    state: &Arc<AppState>,
    owner: &str,
    provider: &str,
    id: &ProviderAccountId,
) -> Result<(), &'static str> {
    let members = state.provider_catalog.portable_account_scope(provider);
    let packages: Vec<_> = members
        .iter()
        .filter_map(|member| state.provider_catalog.latest_package(member))
        .map(|(_, _, package)| package)
        .collect();
    if !packages
        .iter()
        .any(|package| package.manifest.id == provider)
    {
        return Err("named_account_provider_unavailable");
    }
    let vault = state.provider_auth.account(id).map_err(|_| "unavailable")?;
    let mut moved = None;
    // The owner's account lock serializes the move with disabling and
    // revocation and marks the account before its vault first holds a key.
    state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .with_portable_account(
            owner,
            id,
            Use::Commit,
            |account| account.provider_id == provider,
            || {
                moved = Some(
                    state
                        .provider_auth
                        .move_root_scope_into(&vault, &packages, provider),
                );
                std::future::ready(Ok(()))
            },
        )
        .await
        .map_err(|error| {
            if error.to_string() == "account_unavailable" {
                "account_unavailable"
            } else {
                "unavailable"
            }
        })?;
    let outcome = moved.ok_or("unavailable")?.map_err(|error| {
        tracing::warn!(account = id.as_str(), %error, "legacy credential not adopted");
        match error.to_string().as_str() {
            "legacy_adoption_in_progress" => "legacy_adoption_in_progress",
            "account_already_holds_a_credential" => "account_already_holds_a_credential",
            _ => "legacy_credential_unavailable",
        }
    })?;
    distribute_and_mark_scope(state, &vault, id, outcome.account).await;
    if !outcome.wiped.is_empty() {
        distribute_and_mark_provider_auth(state, outcome.wiped).await;
    }
    Ok(())
}

/// Run or resume `owner`'s adoption of `card` under `operation` (already
/// namespaced by its caller).
pub(in crate::server) async fn adopt(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    card: &str,
) -> Result<Value, &'static str> {
    let provider = primary_provider(card)?;
    let store = state.store.as_ref().ok_or("unavailable")?;
    // A finished operation replays its receipt and never repeats an effect.
    if let Some(done) = store
        .provider_account_receipt(owner, &format!("{operation}-final"))
        .await
        .map_err(|_| "unavailable")?
    {
        return Ok(
            json!({"status":"completed","account_id":done.account_id,"card":card,
            "provider_id":provider,"replayed":true}),
        );
    }
    let members = state.provider_catalog.portable_account_scope(provider);
    if members.is_empty() {
        return Err("named_account_provider_unavailable");
    }
    // Refuse what cannot work before the account exists.
    let key = crate::store::provider_accounts::legacy_browsers::account(owner, card);
    let copied =
        live(portable::scope(state, &key, provider).and_then(|vault| vault.status(provider)));
    if !copied
        && !members
            .iter()
            .any(|member| live(state.provider_auth.status(member)))
    {
        return Err("legacy_credential_unavailable");
    }
    let label = label(state, owner, card, provider).await?;
    let adoption = Adoption {
        card,
        provider_id: provider,
        label: &label,
    };
    let created = store
        .adopt_legacy_card(owner, &format!("{operation}-create"), &adoption)
        .await
        .map_err(|_| "outcome_unknown")?;
    let id = applied(state, owner, created)?.account_id;
    move_credential(state, owner, provider, &id).await?;
    let completed = store
        .complete_legacy_adoption(owner, &format!("{operation}-final"), &id, &members)
        .await
        .map_err(|_| "outcome_unknown")?;
    applied(state, owner, completed)?;
    establish_identity_later(state, owner, &format!("{operation}-identity"), &id);
    Ok(
        json!({"status":"completed","account_id":id,"card":card,"provider_id":provider,"label":label,
        "defaults":members}),
    )
}

/// A transient refusal of an adopted account's identity read (the moved
/// credential may not have reached a Machine yet); anything else is final.
fn identity_retryable(code: &str) -> bool {
    !matches!(
        code,
        "account_identity_unsupported"
            | "account_unavailable"
            | "portable_credentials_required"
            | "native_identity_already_bound"
            | "native_identity_mismatch"
            | "native_identity_contract_mismatch"
            | "native_identity_evidence_required"
            | "operation_reused"
            | "invalid_argument"
    )
}

/// Establish an adopted subscription account's identity from the moved
/// credential, in the background and bounded: the account vault reaches its
/// usage Machine asynchronously. A failure leaves the account adopted; the
/// owner can run `accounts.establish_identity` later.
fn establish_identity_later(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    id: &ProviderAccountId,
) {
    let (state, owner, operation, id) = (
        Arc::clone(state),
        owner.to_owned(),
        operation.to_owned(),
        id.clone(),
    );
    tokio::spawn(async move {
        for delay in [5, 30, 120, 600] {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            match super::usage::adopted_identity::establish(&state, &owner, &operation, &id).await {
                Ok(_) => return,
                Err(code) if identity_retryable(code) => {}
                Err(code) => {
                    tracing::warn!(
                        account = id.as_str(),
                        reason = code,
                        "adopted account identity not established"
                    );
                    return;
                }
            }
        }
        tracing::warn!(
            account = id.as_str(),
            "adopted account identity still pending"
        );
    });
}

/// The adoption `operation` started, without starting anything.
pub(in crate::server) async fn receipt(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let Some(created) = store
        .provider_account_receipt(owner, &format!("{operation}-create"))
        .await
        .map_err(|_| "unavailable")?
    else {
        return Ok(json!({"status":"unknown"}));
    };
    let completed = store
        .provider_account_receipt(owner, &format!("{operation}-final"))
        .await
        .map_err(|_| "unavailable")?
        .is_some();
    Ok(
        json!({"status":if completed { "completed" } else { "started" },"account_id":created.account_id}),
    )
}

fn http_operation(operation: &str) -> Result<String, &'static str> {
    if !crate::managed_calls::valid_id(operation) || matches!(operation, "." | "..") {
        return Err("invalid_operation");
    }
    Ok(format!(
        "http-adopt-{}",
        crate::admin::hex_sha256(operation.as_bytes())
    ))
}

/// `POST /api/provider-accounts/legacy/{card}/adopt` `{operation}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct HttpRequest {
    operation: String,
}

pub(in crate::server) async fn http_adopt(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(card): Path<String>,
    body: Result<Json<HttpRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let result = async {
        let Json(request) = body.map_err(|_| "invalid_request")?;
        let operation = http_operation(&request.operation)?;
        adopt(&state, &auth.principal.user_id, &operation, &card).await
    }
    .await;
    let (status, value) = match result {
        Ok(value) => (StatusCode::OK, value),
        Err(code @ ("unavailable" | "outcome_unknown")) => (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error":{"code":code}}),
        ),
        Err(code) => (StatusCode::CONFLICT, json!({"error":{"code":code}})),
    };
    (status, [(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_adoptable_card_maps_to_its_family_primary_provider() {
        use crate::store::provider_accounts::browser_families;
        for (card, provider) in [
            ("openai", "codex"),
            ("xai", "grok"),
            ("deepseek", "claude-deepseek"),
        ] {
            assert_eq!(primary_provider(card), Ok(provider));
            assert_eq!(browser_families::provider(provider), Some(card));
        }
        // Anthropic's legacy credential expired: it signs in again instead.
        assert_eq!(
            primary_provider("anthropic"),
            Err("legacy_credential_expired")
        );
        for card in ["", "default", "future", "OpenAI", "codex"] {
            assert_eq!(primary_provider(card), Err("invalid_provider"));
        }
    }

    #[test]
    fn http_adoptions_are_namespaced_and_accept_only_an_operation() {
        let operation = http_operation("attempt-1").unwrap();
        assert!(operation.starts_with("http-adopt-"));
        assert_eq!(operation, http_operation("attempt-1").unwrap());
        assert!(http_operation("a/b").is_err());
        assert!(
            serde_json::from_value::<HttpRequest>(json!({"operation":"x","machine_id":"ovh"}))
                .is_err()
        );
    }
}

//! Account actions whose input needs its own shape.
use super::actions::{account_owner, document_operation};
use super::*;
use serde::Deserialize;

fn success(value: Value) -> Value {
    json!({"schema":1,"state":"ok","value":value})
}

/// Cancel any discovered owner login by its account and request.
pub(super) async fn cancel_request(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    args: Value,
) -> Result<Value, &'static str> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reference {
        account_id: crate::provider_account::ProviderAccountId,
        request_id: String,
    }
    let input: Reference = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
    let owner = account_owner(state, meta).await?;
    Ok(success(
        super::super::provider_accounts::login::continue_request(
            state,
            &owner,
            &input.account_id,
            &input.request_id,
            true,
        )
        .await?,
    ))
}

/// Adopt a legacy default card into a named account; the same operation
/// resumes it.
pub(super) async fn adopt_legacy(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    args: Value,
    operation: &str,
) -> Result<Value, &'static str> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        card: String,
    }
    let input: Input = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
    let owner = account_owner(state, meta).await?;
    Ok(success(
        super::super::provider_accounts::legacy_adoption::adopt(
            state,
            &owner,
            &document_operation(&meta.id, operation),
            &input.card,
        )
        .await?,
    ))
}

/// Establish an adopted subscription account's identity from its existing
/// vault credential (no sign-in); the same operation replays its result.
pub(super) async fn establish_identity(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    args: Value,
    operation: &str,
) -> Result<Value, &'static str> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        account_id: crate::provider_account::ProviderAccountId,
    }
    let input: Input = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
    let owner = account_owner(state, meta).await?;
    Ok(success(
        super::super::provider_accounts::usage::adopted_identity::establish(
            state,
            &owner,
            &identity_operation(&document_operation(&meta.id, operation)),
            &input.account_id,
        )
        .await?,
    ))
}

/// The receipt of an identity establishment started under `operation`.
pub(super) async fn identity_receipt(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    operation: &str,
) -> Result<Value, &'static str> {
    let owner = account_owner(state, meta).await?;
    let receipt = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .provider_account_receipt(
            &owner,
            &identity_operation(&document_operation(&meta.id, operation)),
        )
        .await
        .map_err(|_| "unavailable")?;
    Ok(success(receipt.map_or_else(
        || json!({"status":"unknown"}),
        |receipt| json!({"status":"bound","account_id":receipt.account_id}),
    )))
}

/// An identity operation's own receipt namespace.
fn identity_operation(operation: &str) -> String {
    format!("{operation}-identity")
}

/// The receipt of an adoption started under `operation`.
pub(super) async fn adoption_receipt(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    operation: &str,
) -> Result<Value, &'static str> {
    let owner = account_owner(state, meta).await?;
    Ok(success(
        super::super::provider_accounts::legacy_adoption::receipt(
            state,
            &owner,
            &document_operation(&meta.id, operation),
        )
        .await?,
    ))
}

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

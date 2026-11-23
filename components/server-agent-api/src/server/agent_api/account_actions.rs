//! Account actions whose input needs its own shape.
use super::actions::account_owner;
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
            &state.accounts,
            &owner,
            &input.account_id,
            &input.request_id,
            true,
        )
        .await?,
    ))
}

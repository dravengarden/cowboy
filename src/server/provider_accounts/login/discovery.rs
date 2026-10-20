//! Read local login metadata without contacting a Provider or starting recovery.
use super::*;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginQuery {
    pub after_machine: Option<String>,
    pub limit: Option<u32>,
}

pub(in crate::server) async fn http_logins(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Query(query): Query<LoginQuery>,
) -> Response {
    operation_response(logins_value(&state, &auth.principal.user_id, &account, query).await)
}

pub(in crate::server) async fn logins_value(
    state: &AppState,
    owner: &str,
    account: &ProviderAccountId,
    query: LoginQuery,
) -> Result<Value, &'static str> {
    let limit = query.limit.unwrap_or(20);
    let after = query.after_machine.as_deref().unwrap_or("");
    if !(1..=50).contains(&limit) || after.len() > 512 || after.chars().any(char::is_control) {
        return Err("invalid_argument");
    }
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let found = registry
        .accounts
        .iter()
        .find(|entry| &entry.id == account)
        .ok_or("account_unavailable")?;
    let mut receipts = store
        .native_login_receipts(owner, account, after, limit + 1)
        .await
        .map_err(|_| "unavailable")?;
    let more = receipts.len() > limit as usize;
    receipts.truncate(limit as usize);
    let next = more
        .then(|| {
            receipts
                .last()
                .map(|receipt| receipt.input.machine_id.clone())
        })
        .flatten();
    let mut logins = Vec::with_capacity(receipts.len());
    for receipt in receipts {
        if receipt.provider_id != found.provider_id {
            return Err("authentication_receipt_mismatch");
        }
        logins.push(operations::observation(state, owner, &receipt).await?);
    }
    Ok(
        json!({"account_id":account,"provider_id":found.provider_id,"logins":logins,"next_after_machine":next}),
    )
}

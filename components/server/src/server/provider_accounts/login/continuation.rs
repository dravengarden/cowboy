//! Owner-checked continuation across GUI, CLI and parent-scoped MCP operations.
//! A request ID locates an existing operation; it cannot dispatch a new login.
use super::*;

async fn resolve(
    store: &crate::store::Store,
    owner: &str,
    account: &ProviderAccountId,
    request: &str,
) -> Result<String, &'static str> {
    if account.is_default() || !crate::managed_calls::valid_id(request) {
        return Err("invalid_login_reference");
    }
    let operation = store
        .provider_login_operation_for_request(owner, request)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")?;
    let receipt = store
        .provider_login_receipt(owner, &operation)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")?;
    if receipt.input.account_id != *account || receipt.request_id != request {
        return Err("login_operation_not_found");
    }
    Ok(operation)
}

pub(in crate::server) async fn http_continue(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((account, request)): Path<(ProviderAccountId, String)>,
) -> Response {
    operation_response(
        continue_request(&state, &auth.principal.user_id, &account, &request, false).await,
    )
}

pub(in crate::server) async fn http_continue_cancel(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((account, request)): Path<(ProviderAccountId, String)>,
) -> Response {
    operation_response(
        continue_request(&state, &auth.principal.user_id, &account, &request, true).await,
    )
}

pub(in crate::server) async fn continue_request(
    state: &Arc<AppState>,
    owner: &str,
    account: &ProviderAccountId,
    request: &str,
    cancel: bool,
) -> Result<Value, &'static str> {
    let operation = resolve(
        state.store.as_ref().ok_or("unavailable")?,
        owner,
        account,
        request,
    )
    .await?;
    if cancel {
        cancel_operation(state, owner, &operation).await
    } else {
        operation_receipt(state, owner, &operation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::provider_accounts::{Change, Mutation, MutationResult, logins::Request};

    #[tokio::test]
    async fn continuation_resolves_existing_namespace_only_for_its_owner_and_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect(
            &format!(
                "sqlite://{}?mode=rwc",
                dir.path().join("login.db").display()
            ),
            dir.path().join("artifacts"),
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let MutationResult::Applied(created) = store
            .mutate_provider_accounts(
                "owner",
                "create",
                &Mutation {
                    expected_revision: 0,
                    change: Change::Create {
                        provider_id: "claude-code".into(),
                        label: "Otter".into(),
                    },
                },
            )
            .await
            .unwrap()
        else {
            panic!("create")
        };
        let account = created.account_id;
        let input = Request {
            account_id: account.clone(),
            machine_id: "machine".into(),
            provider_version: "1".into(),
            generation_digest: "digest".into(),
            method: "native".into(),
        };
        let other = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        for operation in [
            super::super::http_operation("cli").unwrap(),
            format!(
                "agent-{}",
                crate::admin::hex_sha256(b"parent-scoped-operation")
            ),
        ] {
            let receipt = store
                .claim_provider_login("owner", &operation, &input)
                .await
                .unwrap()
                .receipt;
            let request = &receipt.request_id;
            assert_eq!(
                resolve(&store, "owner", &account, request).await.unwrap(),
                operation
            );
            assert_eq!(
                resolve(&store, "other-owner", &account, request).await,
                Err("login_operation_not_found")
            );
            assert_eq!(
                resolve(&store, "owner", &other, request).await,
                Err("login_operation_not_found")
            );
            assert_eq!(
                store
                    .provider_login_receipt("owner", &operation)
                    .await
                    .unwrap()
                    .unwrap(),
                receipt
            );
        }
        assert_eq!(
            resolve(&store, "owner", &account, "missing").await,
            Err("login_operation_not_found")
        );
        assert_eq!(
            resolve(&store, "owner", &account, "../request").await,
            Err("invalid_login_reference")
        );
    }
}

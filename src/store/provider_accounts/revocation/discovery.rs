//! Bounded recovery discovery, not authorization to perform cleanup.
use crate::store::provider_accounts::{Receipt, Registry};
use crate::store::{StorageBackend, Store};
use anyhow::Result;

#[derive(Debug)]
pub(crate) enum LookupError {
    AccountNotFound,
    AmbiguousPlan,
    ScopeMismatch,
}

impl LookupError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::AccountNotFound => "account_not_found",
            Self::AmbiguousPlan => "revocation_plan_ambiguous",
            Self::ScopeMismatch => "revocation_scope_mismatch",
        }
    }
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}
impl std::error::Error for LookupError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub owner: String,
    pub operation: String,
}

pub(crate) struct Page {
    pub operations: Vec<Cursor>,
    /// Advance across examined rows, including historical/inactive operations.
    pub next: Option<Cursor>,
}

impl Store {
    /// Owner-scoped lookup for a different tab/device resuming the same cleanup.
    /// Reject ambiguous historical plans instead of choosing another operation.
    pub(crate) async fn provider_account_revocation(
        &self,
        owner: &str,
        account: &crate::provider_account::ProviderAccountId,
    ) -> Result<Option<(String, Receipt)>> {
        if !self
            .provider_accounts(owner)
            .await?
            .accounts
            .iter()
            .any(|a| &a.id == account)
        {
            return Err(LookupError::AccountNotFound.into());
        }
        macro_rules! read { ($db:expr, $scope:literal, $plan:literal) => {
            sqlx::query_as::<_, (String, String)>(concat!(
                "SELECT operation_id,result FROM provider_account_operations WHERE owner_user_id=$1 AND ",
                $scope, "=$2 AND ", $plan, " ORDER BY operation_id LIMIT 2"))
                .bind(owner).bind(account.as_str()).fetch_all(&$db.pool).await?
        }; }
        let mut rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(
                db,
                "result::jsonb ->> 'account_id'",
                "result::jsonb -> 'revocation' IS NOT NULL AND result::jsonb -> 'revocation' <> 'null'::jsonb"
            ),
            StorageBackend::Sqlite(db) => read!(
                db,
                "json_extract(result, '$.account_id')",
                "json_type(result, '$.revocation') IS NOT NULL AND json_type(result, '$.revocation') <> 'null'"
            ),
        };
        if rows.len() > 1 {
            return Err(LookupError::AmbiguousPlan.into());
        }
        rows.pop()
            .map(|(operation, value)| {
                let receipt: Receipt = serde_json::from_str(&value)?;
                if &receipt.account_id != account || receipt.revocation.is_none() {
                    return Err(LookupError::ScopeMismatch.into());
                }
                Ok((operation, receipt))
            })
            .transpose()
    }

    /// Scan at most 32 journal rows per page. Replay locators carry no vault,
    /// process-exit or remote-dispatch authority. Each stage rechecks admission.
    pub(crate) async fn provider_account_revocations(
        &self,
        after: Option<&Cursor>,
    ) -> Result<Page> {
        let owner = after.map_or("", |cursor| cursor.owner.as_str());
        let operation = after.map_or("", |cursor| cursor.operation.as_str());
        macro_rules! read { ($db:expr, $has_plan:literal) => {
            sqlx::query_as::<_, (String, String, String, String)>(concat!(
                "SELECT o.owner_user_id,o.operation_id,o.result,r.value FROM provider_account_operations o ",
                "JOIN provider_account_registries r ON r.owner_user_id=o.owner_user_id WHERE ", $has_plan,
                " AND (o.owner_user_id > $1 OR (o.owner_user_id=$1 AND o.operation_id > $2)) ",
                "ORDER BY o.owner_user_id,o.operation_id LIMIT 33"))
                .bind(owner).bind(operation).fetch_all(&$db.pool).await?
        }; }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(
                db,
                "o.result::jsonb -> 'revocation' IS NOT NULL AND o.result::jsonb -> 'revocation' <> 'null'::jsonb"
            ),
            StorageBackend::Sqlite(db) => read!(
                db,
                "json_type(o.result, '$.revocation') IS NOT NULL AND json_type(o.result, '$.revocation') <> 'null'"
            ),
        };
        let more = rows.len() > 32;
        let mut operations = Vec::new();
        let mut last = None;
        for (owner, operation, receipt, registry) in rows.into_iter().take(32) {
            let receipt: Receipt = serde_json::from_str(&receipt)?;
            let registry: Registry = serde_json::from_str(&registry)?;
            let cursor = Cursor { owner, operation };
            if receipt.revocation.is_some()
                && !receipt.revocation_completed
                && registry.accounts.iter().any(|account| {
                    account.id == receipt.account_id && account.revoking && !account.enabled
                })
            {
                operations.push(cursor.clone());
            }
            last = Some(cursor);
        }
        Ok(Page {
            operations,
            next: if more { last } else { None },
        })
    }
}

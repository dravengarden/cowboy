//! Commit local cleanup completion without clearing the account admission fence.
use super::{grant_admission, grants, logins, readiness, stops};
use crate::store::{
    StorageBackend, Store,
    provider_accounts::{Receipt, Registry},
};
use anyhow::{Result, ensure};

impl Store {
    /// No remote effects. Evidence and completion are serialized with every
    /// account/login/grant writer. An unknown commit outcome is safe to retry
    /// with the original owner and operation; no cleanup is dispatched here.
    pub async fn complete_provider_revocation(&self, owner: &str, operation: &str) -> Result<bool> {
        macro_rules! complete { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let locked = sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner).execute(&mut *tx).await?;
            ensure!(locked.rows_affected() == 1, "account_unavailable");
            let value: String = sqlx::query_scalar("SELECT result FROM provider_account_operations WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).fetch_one(&mut *tx).await?;
            let mut receipt: Receipt = serde_json::from_str(&value)?;
            let plan = receipt.revocation.as_ref().ok_or_else(|| anyhow::anyhow!("revocation_plan_missing"))?;
            ensure!(!receipt.account_id.is_default(), "named_account_required");
            // Exact historical completion never reacquires dispatch authority.
            if receipt.revocation_completed { return Ok(true); }
            let value: String = sqlx::query_scalar("SELECT value FROM provider_account_registries WHERE owner_user_id=$1")
                .bind(owner).fetch_one(&mut *tx).await?;
            let registry: Registry = serde_json::from_str(&value)?;
            ensure!(registry.accounts.iter().any(|account| account.id == receipt.account_id
                && account.revoking && !account.enabled), "account_not_revoking");
            for target in &plan.sessions {
                let value: Option<String> = sqlx::query_scalar("SELECT value FROM provider_account_stops WHERE owner_user_id=$1 AND operation_id=$2 AND session_id=$3")
                    .bind(owner).bind(operation).bind(&target.session_id).fetch_optional(&mut *tx).await?;
                let stopped: Option<stops::Evidence> = value.map(|value| serde_json::from_str(&value)).transpose()?;
                if !readiness::session_missing(&receipt, target, stopped.as_ref()).is_empty() { return Ok(false); }
            }
            for target in &plan.logins {
                let value: String = sqlx::query_scalar("SELECT value FROM provider_login_operations WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(&target.operation_id).fetch_one(&mut *tx).await?;
                let login: logins::Receipt = serde_json::from_str(&value)?;
                if grant_admission::validate_login_cleanup(&receipt, target, &login, operation).is_err() { return Ok(false); }
            }
            for target in &plan.grants {
                let value: String = sqlx::query_scalar("SELECT value FROM provider_account_grants WHERE owner_user_id=$1 AND operation_id=$2")
                    .bind(owner).bind(&target.operation_id).fetch_one(&mut *tx).await?;
                let grant: grants::Receipt = serde_json::from_str(&value)?;
                ensure!(grant.account_id == receipt.account_id && grant.machine_id == target.machine_id
                    && grant.operation_id == target.operation_id, "grant_scope_mismatch");
                if !readiness::grant_missing(&receipt, &grant).is_empty() { return Ok(false); }
            }
            receipt.revocation_completed = true;
            sqlx::query("UPDATE provider_account_operations SET result=$3 WHERE owner_user_id=$1 AND operation_id=$2")
                .bind(owner).bind(operation).bind(serde_json::to_string(&receipt)?).execute(&mut *tx).await?;
            tx.commit().await?;
            true
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => complete!(db),
            StorageBackend::Sqlite(db) => complete!(db),
        })
    }
}

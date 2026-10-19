//! Machine-scoped native login discovery. Results are not launch authority.
use super::{NativeCompletion, ProviderAccountId, Receipt, Result, StorageBackend, Store};
use anyhow::ensure;

impl Store {
    /// Select only the latest reserved generation for this owner/account/Machine.
    /// Never fall back to an older successful login while a replacement is pending.
    /// The caller must use `with_native_login_binding` immediately around its
    /// scheduling effect; discovery can race cancellation or a new reservation.
    pub(crate) async fn native_login_for_session(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        machine: &str,
    ) -> Result<Option<(String, NativeCompletion)>> {
        self.select_native_login(owner, account, machine, None)
            .await
    }

    /// Discovery for an existing durable session, never a fallback for creation.
    /// Callers must recheck the row under `with_native_session_binding`.
    pub(crate) async fn native_login_for_registered_session(
        &self,
        owner: &str,
        session: &str,
    ) -> Result<Option<(String, NativeCompletion)>> {
        let query = "SELECT provider_account_id,machine_id,provider_auth_generation FROM sessions WHERE id=$1 AND owner_user_id=$2";
        let row: Option<(String, String, Option<i64>)> = match &self.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query_as(query)
                    .bind(session)
                    .bind(owner)
                    .fetch_optional(&db.pool)
                    .await?
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query_as(query)
                    .bind(session)
                    .bind(owner)
                    .fetch_optional(&db.pool)
                    .await?
            }
        };
        let Some((account, machine, Some(generation))) = row else {
            return Ok(None);
        };
        ensure!(generation > 0, "invalid_native_generation");
        self.select_native_login(
            owner,
            &account.parse().map_err(anyhow::Error::msg)?,
            &machine,
            Some(generation),
        )
        .await
    }

    async fn select_native_login(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        machine: &str,
        generation: Option<i64>,
    ) -> Result<Option<(String, NativeCompletion)>> {
        ensure!(!account.is_default(), "named_account_required");
        macro_rules! read {
            ($db:expr, $account:literal, $machine:literal, $generation:literal) => {{
                sqlx::query_as::<_, (String, String)>(concat!(
                    "SELECT l.operation_id,l.value FROM provider_login_operations l ",
                    "JOIN provider_native_generations g ON g.owner_user_id=l.owner_user_id ",
                    "AND g.account_id=",
                    $account,
                    " AND g.machine_id=",
                    $machine,
                    " AND CAST(COALESCE($4,g.generation) AS TEXT)=",
                    $generation,
                    " WHERE g.owner_user_id=$1 AND g.account_id=$2 AND g.machine_id=$3 LIMIT 2"
                ))
                .bind(owner)
                .bind(account.as_str())
                .bind(machine)
                .bind(generation)
                .fetch_all(&$db.pool)
                .await?
            }};
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(
                db,
                "l.value::jsonb #>> '{input,account_id}'",
                "l.value::jsonb #>> '{input,machine_id}'",
                "l.value::jsonb ->> 'native_generation'"
            ),
            StorageBackend::Sqlite(db) => read!(
                db,
                "json_extract(l.value, '$.input.account_id')",
                "json_extract(l.value, '$.input.machine_id')",
                "CAST(json_extract(l.value, '$.native_generation') AS TEXT)"
            ),
        };
        ensure!(rows.len() <= 1, "native_generation_ambiguous");
        let Some((operation, value)) = rows.into_iter().next() else {
            return Ok(None);
        };
        let receipt: Receipt = serde_json::from_str(&value)?;
        if receipt.cancel_requested
            || receipt.dispatch_rejection.is_some()
            || receipt.native_logout.is_some()
            || receipt.native_binding_revision.is_none()
        {
            return Ok(None);
        }
        let Some(completion) = receipt.native_completion else {
            return Ok(None);
        };
        let profile = &completion.profile;
        ensure!(
            receipt.provider_id == "claude-code"
                && profile.provider_id == receipt.provider_id
                && &receipt.input.account_id == account
                && &profile.account_id == account
                && receipt.input.machine_id == machine
                && completion.machine_id == machine
                && receipt.native_generation == Some(profile.generation)
                && profile.generation > 0
                && profile.request_id == receipt.request_id
                && profile.provider_version == receipt.input.provider_version
                && profile.generation_digest == receipt.input.generation_digest
                && profile.method == receipt.input.method
                && receipt.dispatch_recipient_fingerprint.as_ref()
                    == Some(&completion.recipient_fingerprint),
            "native_completion_scope_mismatch"
        );
        Ok(Some((operation, completion)))
    }
}

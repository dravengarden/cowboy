//! Machine-scoped native login discovery. Results are not launch authority.
use super::{NativeCompletion, ProviderAccountId, Receipt, Result, StorageBackend, Store};
use anyhow::ensure;

/// A replacement login that was cancelled or refused before it ever
/// completed. It can never bind (binding requires neither), so it must not
/// disable the account's previous working login.
pub(super) fn abandoned(receipt: &Receipt) -> bool {
    (receipt.cancel_requested || receipt.dispatch_rejection.is_some())
        && receipt.native_completion.is_none()
}

/// The generation that admits new sessions: the latest reserved one, skipping
/// newer abandoned replacements. A pending or unknown replacement still wins,
/// so a login in progress is never raced by an older one.
pub(super) fn effective_generation<'a>(
    receipts: impl IntoIterator<Item = &'a Receipt>,
    account: &ProviderAccountId,
    machine: &str,
    latest: u64,
) -> u64 {
    let scoped: Vec<_> = receipts
        .into_iter()
        .filter(|receipt| {
            &receipt.input.account_id == account && receipt.input.machine_id == machine
        })
        .collect();
    let mut generation = latest;
    while generation > 1
        && scoped
            .iter()
            .any(|receipt| receipt.native_generation == Some(generation) && abandoned(receipt))
        && !scoped
            .iter()
            .any(|receipt| receipt.native_generation == Some(generation) && !abandoned(receipt))
    {
        generation -= 1;
    }
    generation
}

impl Store {
    /// Select the generation that admits new sessions for this owner/account/
    /// Machine: the latest reserved one, or the newest older one when every
    /// newer replacement was abandoned. Never fall back while a replacement is
    /// pending. The caller must use `with_native_login_binding` immediately
    /// around its scheduling effect; discovery can race a new reservation.
    pub async fn native_login_for_session(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        machine: &str,
    ) -> Result<Option<(String, NativeCompletion)>> {
        ensure!(!account.is_default(), "named_account_required");
        let query = "SELECT generation FROM provider_native_generations WHERE owner_user_id=$1 AND account_id=$2 AND machine_id=$3";
        let receipts_query = "SELECT value FROM provider_login_operations WHERE owner_user_id=$1";
        let (latest, values): (Option<i64>, Vec<String>) = match &self.backend {
            StorageBackend::Postgres(db) => (
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(account.as_str())
                    .bind(machine)
                    .fetch_optional(&db.pool)
                    .await?,
                sqlx::query_scalar(receipts_query)
                    .bind(owner)
                    .fetch_all(&db.pool)
                    .await?,
            ),
            StorageBackend::Sqlite(db) => (
                sqlx::query_scalar(query)
                    .bind(owner)
                    .bind(account.as_str())
                    .bind(machine)
                    .fetch_optional(&db.pool)
                    .await?,
                sqlx::query_scalar(receipts_query)
                    .bind(owner)
                    .fetch_all(&db.pool)
                    .await?,
            ),
        };
        let Some(latest) = latest else {
            return Ok(None);
        };
        let receipts: Vec<Receipt> = values
            .iter()
            .filter_map(|value| serde_json::from_str(value).ok())
            .collect();
        let generation = effective_generation(&receipts, account, machine, u64::try_from(latest)?);
        self.select_native_login(owner, account, machine, Some(i64::try_from(generation)?))
            .await
    }

    /// Discovery for an existing durable session, never a fallback for creation.
    /// Callers must recheck the row under `with_native_session_binding`.
    pub async fn native_login_for_registered_session(
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

#[cfg(test)]
mod tests {
    use super::{abandoned, effective_generation};
    use crate::store::provider_accounts::logins::{DispatchRejection, Receipt};

    fn receipt(generation: u64, machine: &str, state: &str) -> Receipt {
        let mut receipt: Receipt = serde_json::from_value(serde_json::json!({
            "request_id": format!("r{generation}"),
            "provider_id": "claude",
            "input": {
                "account_id": "fd376b94-c493-4d27-81da-67a1d154ca17",
                "machine_id": machine,
                "provider_version": "1",
                "generation_digest": "d",
                "method": "m"
            }
        }))
        .unwrap();
        receipt.native_generation = Some(generation);
        match state {
            "cancelled" => receipt.cancel_requested = true,
            "rejected" => receipt.dispatch_rejection = Some(DispatchRejection::StartRejected),
            _ => {}
        }
        receipt
    }

    #[test]
    fn only_abandoned_replacements_fall_back() {
        let account = "fd376b94-c493-4d27-81da-67a1d154ca17".parse().unwrap();
        let receipts = [
            receipt(4, "m", "pending"),
            receipt(5, "m", "cancelled"),
            receipt(6, "m", "rejected"),
            receipt(7, "other", "pending"),
        ];
        assert!(abandoned(&receipts[1]) && abandoned(&receipts[2]) && !abandoned(&receipts[0]));
        assert_eq!(effective_generation(&receipts, &account, "m", 6), 4);
        assert_eq!(effective_generation(&receipts[..2], &account, "m", 5), 4);
        // A pending replacement wins, and a reserved generation without any
        // receipt (unknown) is never skipped.
        assert_eq!(effective_generation(&receipts, &account, "m", 4), 4);
        assert_eq!(effective_generation(&receipts, &account, "m", 8), 8);
        assert_eq!(effective_generation(&receipts, &account, "other", 7), 7);
    }
}

//! Bounded owner/account discovery of the latest native login on each Machine.
use super::{ProviderAccountId, Receipt, Result, StorageBackend, Store};
use anyhow::ensure;

impl Store {
    pub async fn native_login_receipts(
        &self,
        owner: &str,
        account: &ProviderAccountId,
        after_machine: &str,
        limit: u32,
    ) -> Result<Vec<Receipt>> {
        ensure!((1..=51).contains(&limit), "invalid_limit");
        macro_rules! read {
            ($db:expr, $account:literal, $machine:literal, $generation:literal, $collation:literal) => {
                sqlx::query_as::<_, (String, i64, String)>(concat!(
                    "SELECT g.machine_id,g.generation,l.value FROM provider_native_generations g ",
                    "JOIN provider_login_operations l ON l.owner_user_id=g.owner_user_id AND ",
                    $account,
                    "=g.account_id AND ",
                    $machine,
                    "=g.machine_id AND ",
                    $generation,
                    "=CAST(g.generation AS TEXT) ",
                    "WHERE g.owner_user_id=$1 AND g.account_id=$2 AND g.machine_id ",
                    $collation,
                    ">$3 ",
                    "ORDER BY g.machine_id ",
                    $collation,
                    " LIMIT $4"
                ))
                .bind(owner)
                .bind(account.as_str())
                .bind(after_machine)
                .bind(i64::from(limit))
                .fetch_all(&$db.pool)
                .await?
            };
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(
                db,
                "l.value::jsonb #>> '{input,account_id}'",
                "l.value::jsonb #>> '{input,machine_id}'",
                "l.value::jsonb ->> 'native_generation'",
                "COLLATE \"C\""
            ),
            StorageBackend::Sqlite(db) => read!(
                db,
                "json_extract(l.value,'$.input.account_id')",
                "json_extract(l.value,'$.input.machine_id')",
                "CAST(json_extract(l.value,'$.native_generation') AS TEXT)",
                "COLLATE BINARY"
            ),
        };
        let mut seen = std::collections::BTreeSet::new();
        rows.into_iter()
            .map(|(machine, generation, value)| {
                let receipt: Receipt = serde_json::from_str(&value)?;
                ensure!(
                    seen.insert(machine.clone())
                        && &receipt.input.account_id == account
                        && receipt.input.machine_id == machine
                        && receipt.native_generation == Some(u64::try_from(generation)?),
                    "native_login_scope_mismatch"
                );
                Ok(receipt)
            })
            .collect()
    }
}

//! Remove what the retired legacy default cards left in the account tables
//! (`docs/legacy-card-retirement.md`): every `card:<card>` browser share
//! member and every card browser enablement.
//!
//! The Controller runs it at every start, after the credential admission
//! gate and its migrations, before anything is served (never a CLI that may
//! share the database with a Controller from before the retirement). It is
//! idempotent. That covers the first upgrade, a re-upgrade after a rollback
//! (the previous Controller may have added card members again; `SQLx` would not
//! rerun a recorded migration) and a store copy (migrated empty, imported,
//! then opened by a Controller). Each affected owner's share
//! revision and account registry revision (column and serialized value) move
//! in the same transaction, so a share reset or an account mutation captured
//! against the earlier membership fails its compare. Family bindings a card
//! left stay: they keep its family out of the profile until a confirmed reset.
use super::super::{StorageBackend, Store};
use super::Registry;
use anyhow::{Context as _, Result};

macro_rules! retire_rows {
    ($db:expr) => {{
        let mut tx = $db.pool.begin().await?;
        let owners: Vec<String> = sqlx::query_scalar(
            "SELECT owner_user_id FROM provider_browser_share_members WHERE member LIKE 'card:%' \
             UNION SELECT owner_user_id FROM provider_legacy_browsers",
        )
        .fetch_all(&mut *tx)
        .await?;
        for owner in &owners {
            // The account writer's owner lock (`locked_replay`): a write to
            // the registry row first, so a concurrent mutation either
            // commits before this read or waits for this transaction.
            sqlx::query("UPDATE provider_account_registries SET revision=revision WHERE owner_user_id=$1")
                .bind(owner)
                .execute(&mut *tx)
                .await?;
            let value: Option<String> = sqlx::query_scalar(
                "SELECT value FROM provider_account_registries WHERE owner_user_id=$1",
            )
            .bind(owner)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(value) = value {
                let mut registry: Registry =
                    serde_json::from_str(&value).context("decoding an account registry")?;
                registry.revision = registry
                    .revision
                    .checked_add(1)
                    .context("account registry revision exhausted")?;
                sqlx::query(
                    "UPDATE provider_account_registries SET revision=$1,value=$2 WHERE owner_user_id=$3",
                )
                .bind(registry.revision)
                .bind(serde_json::to_string(&registry)?)
                .bind(owner)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "UPDATE provider_browser_shares SET revision=revision+1 WHERE owner_user_id=$1 \
                 AND share_id IN (SELECT share_id FROM provider_browser_share_members \
                 WHERE owner_user_id=$1 AND member LIKE 'card:%')",
            )
            .bind(owner)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "DELETE FROM provider_browser_share_members WHERE owner_user_id=$1 AND member LIKE 'card:%'",
            )
            .bind(owner)
            .execute(&mut *tx)
            .await?;
            // The table stays, empty, for a rolled-back Controller.
            sqlx::query("DELETE FROM provider_legacy_browsers WHERE owner_user_id=$1")
                .bind(owner)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        owners.len()
    }};
}

impl Store {
    /// Remove the retired cards' share members and browser enablements;
    /// returns the number of owners it changed.
    pub async fn retire_legacy_card_rows(&self) -> Result<usize> {
        let owners = match &self.backend {
            StorageBackend::Postgres(db) => retire_rows!(db),
            StorageBackend::Sqlite(db) => retire_rows!(db),
        };
        if owners > 0 {
            tracing::info!(
                owners,
                "removed retired legacy card share members and browsers"
            );
        }
        Ok(owners)
    }

    /// Cancel every pending Service reset of a retired card lane, each with
    /// its audit row in the same transaction, so a rolled-back Controller can
    /// never fire one. Returns the lanes it cancelled. Idempotent.
    pub async fn cancel_retired_resets(
        &self,
        lanes: &[String],
        now_ms: i64,
    ) -> Result<Vec<String>> {
        macro_rules! cancel {
            ($db:expr, $lane:expr) => {{
                let mut tx = $db.pool.begin().await?;
                let deleted = sqlx::query("DELETE FROM scheduled_provider_actions WHERE provider=$1")
                    .bind($lane)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                if deleted > 0 {
                    sqlx::query(
                        "INSERT INTO provider_action_logs (provider,action,trigger,status,phase,message,created_at_ms) \
                         VALUES ($1,'rate_limit_reset','scheduled','cancelled','timer',$2,$3)",
                    )
                    .bind($lane)
                    .bind("Scheduled reset cancelled: the legacy default card is retired")
                    .bind(now_ms)
                    .execute(&mut *tx)
                    .await?;
                }
                tx.commit().await?;
                deleted > 0
            }};
        }
        let mut cancelled = Vec::new();
        for lane in lanes {
            let done = match &self.backend {
                StorageBackend::Postgres(db) => cancel!(db, lane),
                StorageBackend::Sqlite(db) => cancel!(db, lane),
            };
            if done {
                cancelled.push(lane.clone());
            }
        }
        Ok(cancelled)
    }
}

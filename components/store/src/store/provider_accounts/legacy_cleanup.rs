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
            ($db:expr, $lane:expr) => {async {
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
                anyhow::Ok(deleted > 0)
            }.await};
        }
        // Every lane is attempted; any failure fails the whole call.
        let (mut cancelled, mut failed) = (Vec::new(), Vec::new());
        for lane in lanes {
            let done = match &self.backend {
                StorageBackend::Postgres(db) => cancel!(db, lane),
                StorageBackend::Sqlite(db) => cancel!(db, lane),
            };
            match done {
                Ok(true) => cancelled.push(lane.clone()),
                Ok(false) => {}
                Err(error) => failed.push(format!("{lane}: {error:#}")),
            }
        }
        anyhow::ensure!(
            failed.is_empty(),
            "cancelling retired reset schedules failed ({})",
            failed.join("; ")
        );
        Ok(cancelled)
    }

    /// The enrolled, non-revoked Machines a Service credential is delivered
    /// to, each with its enrollment identity (its encryption key's digest): a
    /// placeholder row without enrollment keys is no recipient.
    pub async fn credential_recipients(&self) -> Result<Vec<(String, String)>> {
        let mut recipients = Vec::new();
        for machine in self.list_machines().await? {
            if machine.revoked || machine.fingerprint.is_none() {
                continue;
            }
            if let Some(key) = self.machine_encryption_public_key(&machine.id).await? {
                recipients.push((machine.id, crate::digest::hex_sha256(key.as_bytes())));
            }
        }
        Ok(recipients)
    }

    /// Fence every live (starting, running or busy) session on a retired
    /// card's `default` account as crashed, returning their ids. Only for a
    /// dataset whose recorded retirement admission still matches every wipe
    /// generation (`retirement::admit_store`): such a row is stale, never a
    /// card credential in use. A fenced session refuses every prompt.
    pub async fn fence_live_retired_default_sessions(&self) -> Result<Vec<String>> {
        macro_rules! fence {
            ($db:expr, $updated:literal, $deleted:literal) => {{
                let mut tx = $db.pool.begin().await?;
                let mut fenced = Vec::new();
                for provider in super::legacy_cards::retired_providers() {
                    fenced.extend(
                        sqlx::query_scalar::<_, String>(concat!(
                            "UPDATE sessions SET status='crashed', ",
                            $updated,
                            " WHERE provider=$1 AND provider_account_id='default' AND ",
                            $deleted,
                            " IS NULL AND status IN ('starting','running','busy') RETURNING id",
                        ))
                        .bind(provider)
                        .fetch_all(&mut *tx)
                        .await?,
                    );
                }
                tx.commit().await?;
                fenced.sort();
                fenced
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => fence!(db, "updated_at=now()", "deleted_at"),
            StorageBackend::Sqlite(db) => fence!(
                db,
                "updated_at_ms=CAST(strftime('%s','now') AS INTEGER)*1000",
                "deleted_at_ms"
            ),
        })
    }

    /// Live sessions (starting, running or busy) on a retired card's
    /// `default` account: their workers may still hold a card credential.
    pub async fn live_retired_default_sessions(&self) -> Result<Vec<String>> {
        let providers = super::legacy_cards::retired_providers();
        macro_rules! read {
            ($db:expr, $deleted:literal) => {
                sqlx::query_as::<_, (String, String)>(concat!(
                    "SELECT id, provider FROM sessions WHERE ",
                    $deleted,
                    " IS NULL AND provider_account_id='default' \
                     AND status IN ('starting','running','busy') ORDER BY id"
                ))
                .fetch_all(&$db.pool)
                .await?
            };
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db, "deleted_at"),
            StorageBackend::Sqlite(db) => read!(db, "deleted_at_ms"),
        };
        Ok(rows
            .into_iter()
            .filter(|(_, provider)| providers.contains(&provider.as_str()))
            .map(|(id, _)| id)
            .collect())
    }
}

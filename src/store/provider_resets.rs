//! In-flight and authority state of persisted one-shot reset schedules.
//!
//! A due schedule moves to in flight (`begin_provider_reset`) before any
//! Provider call, keyed by its exact idempotency key. A cancel or a
//! replacement during the attempt therefore never revives the old schedule,
//! and a row still in flight when no attempt runs was interrupted: its outcome
//! is unknown and it is never retried. Only a proven preflight failure rearms
//! it. A named account's schedule records the authority that created it.
use super::{StorageBackend, Store, ensure_reset_provider};
use anyhow::{Context as _, Result};

/// The flight state of one lane's schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetFlight {
    pub idempotency_key: String,
    pub in_flight: bool,
    pub authority: Option<String>,
}

macro_rules! on_backend {
    ($store:expr, |$pool:ident| $body:expr) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => {
                let $pool = &db.pool;
                $body
            }
            StorageBackend::Sqlite(db) => {
                let $pool = &db.pool;
                $body
            }
        }
    };
}

impl Store {
    pub async fn provider_reset_flight(&self, provider: &str) -> Result<Option<ResetFlight>> {
        ensure_reset_provider(provider)?;
        let row: Option<(String, Option<i64>, Option<String>)> = on_backend!(self, |pool| {
            sqlx::query_as(
                "SELECT idempotency_key, in_flight_at_ms, authority \
                 FROM scheduled_provider_actions WHERE provider = $1",
            )
            .bind(provider)
            .fetch_optional(pool)
            .await
        })
        .context("reading scheduled reset flight")?;
        Ok(
            row.map(|(idempotency_key, in_flight, authority)| ResetFlight {
                idempotency_key,
                in_flight: in_flight.is_some(),
                authority,
            }),
        )
    }

    /// Move exactly this pending schedule to in flight. `false`: it was
    /// cancelled, replaced or is already in flight; nothing may be sent.
    pub async fn begin_provider_reset(
        &self,
        provider: &str,
        key: &str,
        now_ms: i64,
    ) -> Result<bool> {
        ensure_reset_provider(provider)?;
        let rows = on_backend!(self, |pool| {
            sqlx::query(
                "UPDATE scheduled_provider_actions SET in_flight_at_ms = $3 \
                 WHERE provider = $1 AND idempotency_key = $2 AND in_flight_at_ms IS NULL",
            )
            .bind(provider)
            .bind(key)
            .bind(now_ms)
            .execute(pool)
            .await
            .map(|result| result.rows_affected())
        })
        .context("starting scheduled reset")?;
        Ok(rows == 1)
    }

    /// Return a proven-unsent attempt of exactly this schedule to pending.
    pub async fn rearm_provider_reset(
        &self,
        provider: &str,
        key: &str,
        next_attempt_at_ms: i64,
    ) -> Result<bool> {
        ensure_reset_provider(provider)?;
        let rows = on_backend!(self, |pool| {
            sqlx::query(
                "UPDATE scheduled_provider_actions SET attempt_count = attempt_count + 1, \
                 next_attempt_at_ms = $3, in_flight_at_ms = NULL \
                 WHERE provider = $1 AND idempotency_key = $2",
            )
            .bind(provider)
            .bind(key)
            .bind(next_attempt_at_ms)
            .execute(pool)
            .await
            .map(|result| result.rows_affected())
        })
        .context("rearming scheduled reset")?;
        Ok(rows == 1)
    }

    /// Bind exactly this schedule to the authority that created it.
    pub async fn bind_provider_reset_authority(
        &self,
        provider: &str,
        key: &str,
        authority: &str,
    ) -> Result<bool> {
        ensure_reset_provider(provider)?;
        anyhow::ensure!(
            (1..=128).contains(&authority.len()),
            "invalid reset authority"
        );
        let rows = on_backend!(self, |pool| {
            sqlx::query(
                "UPDATE scheduled_provider_actions SET authority = $3 \
                 WHERE provider = $1 AND idempotency_key = $2",
            )
            .bind(provider)
            .bind(key)
            .bind(authority)
            .execute(pool)
            .await
            .map(|result| result.rows_affected())
        })
        .context("binding scheduled reset authority")?;
        Ok(rows == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_schedule_flies_once_and_a_replacement_is_a_new_pending_schedule() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let lane = "account-11111111-1111-4111-8111-111111111111";
        store.upsert_provider_reset(lane, 10, "old").await.unwrap();
        assert!(
            store
                .bind_provider_reset_authority(lane, "old", "sha256:a")
                .await
                .unwrap()
        );
        assert!(!store.begin_provider_reset(lane, "other", 11).await.unwrap());
        assert!(store.begin_provider_reset(lane, "old", 11).await.unwrap());
        // In flight: never started twice.
        assert!(!store.begin_provider_reset(lane, "old", 12).await.unwrap());
        let flight = store.provider_reset_flight(lane).await.unwrap().unwrap();
        assert!(flight.in_flight);
        assert_eq!(flight.authority.as_deref(), Some("sha256:a"));
        // A proven-unsent attempt returns to pending, counted.
        assert!(store.rearm_provider_reset(lane, "old", 70).await.unwrap());
        let action = store.load_provider_reset(lane).await.unwrap().unwrap();
        assert_eq!((action.attempt_count, action.next_attempt_at_ms), (1, 70));
        assert!(
            !store
                .provider_reset_flight(lane)
                .await
                .unwrap()
                .unwrap()
                .in_flight
        );
        // A replacement during flight is a fresh pending schedule without the
        // old authority; the old key can no longer start, rearm or claim it.
        assert!(store.begin_provider_reset(lane, "old", 71).await.unwrap());
        store.upsert_provider_reset(lane, 99, "new").await.unwrap();
        let flight = store.provider_reset_flight(lane).await.unwrap().unwrap();
        assert_eq!(
            flight,
            ResetFlight {
                idempotency_key: "new".into(),
                in_flight: false,
                authority: None
            }
        );
        assert!(!store.rearm_provider_reset(lane, "old", 1).await.unwrap());
        assert!(!store.claim_provider_reset(lane, "old").await.unwrap());
    }
}

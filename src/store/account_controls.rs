//! Durable account controls of named-only Providers (`crate::account_controls`):
//! the per-account state, the single active-turn reservation and the Machine
//! fences the Controller owes each account's daemon. Every state change that
//! requires fences writes them in the same transaction, so a fence is never
//! lost between the durable state and the Machine command.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::account_controls::{
    AccountState, Control, Fence, FenceState, Reservation, ReservationState, fence_operation_id,
};
use crate::provider_account::AccountFenceAction;
use anyhow::{Context as _, Result};

macro_rules! dispatch {
    ($store:expr, $method:ident($($arg:expr),* $(,)?)) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => db.$method($($arg),*).await,
            StorageBackend::Sqlite(db) => db.$method($($arg),*).await,
        }
    };
}

/// One state change of an account, with the fences it requires.
#[derive(Clone, Debug)]
pub struct StateChange<'a> {
    pub account_id: &'a str,
    pub owner_user_id: &'a str,
    pub provider_id: &'a str,
    pub state: AccountState,
    pub reason: &'a str,
    pub until_ms: Option<i64>,
    /// `Some(revision)` applies only over exactly that revision (`0`: no
    /// record yet); `None` applies over any (automatic transitions).
    pub expected_revision: Option<i64>,
    /// Replace the account's fence on each of these Machines.
    pub fence: Option<(AccountFenceAction, &'a [String])>,
    pub now_ms: i64,
}

type ControlRow = (
    String,
    String,
    String,
    String,
    String,
    Option<i64>,
    i64,
    i64,
);
type TurnRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    i64,
    i64,
);
type FenceRow = (String, String, String, String, String, String, i64);

fn control(row: ControlRow) -> Result<Control> {
    Ok(Control {
        account_id: row.0,
        owner_user_id: row.1,
        provider_id: row.2,
        state: AccountState::parse(&row.3).context("unknown stored account state")?,
        reason: row.4,
        until_ms: row.5,
        updated_at_ms: row.6,
        revision: row.7,
    })
}

fn turn(row: TurnRow) -> Result<Reservation> {
    Ok(Reservation {
        account_id: row.0,
        owner_user_id: row.1,
        session_id: row.2,
        turn_id: row.3,
        worker_epoch: row.4,
        after_seq: row.5,
        state: ReservationState::parse(&row.6).context("unknown stored reservation state")?,
        created_at_ms: row.7,
        updated_at_ms: row.8,
    })
}

fn fence(row: FenceRow) -> Result<Fence> {
    Ok(Fence {
        account_id: row.0,
        machine_id: row.1,
        action: match row.2.as_str() {
            "stop" => AccountFenceAction::Stop,
            "admit" => AccountFenceAction::Admit,
            _ => anyhow::bail!("unknown stored fence action"),
        },
        operation_id: row.3,
        state: match row.4.as_str() {
            "pending" => FenceState::Pending,
            "confirmed" => FenceState::Confirmed,
            _ => anyhow::bail!("unknown stored fence state"),
        },
        detail: row.5,
        updated_at_ms: row.6,
    })
}

const fn action_name(action: AccountFenceAction) -> &'static str {
    match action {
        AccountFenceAction::Stop => "stop",
        AccountFenceAction::Admit => "admit",
    }
}

fn bounded(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

impl Store {
    pub async fn account_controls(&self) -> Result<Vec<Control>> {
        dispatch!(self, account_controls())
    }

    /// Apply `change`; `None` when its expected revision no longer holds.
    pub async fn set_account_state(&self, change: &StateChange<'_>) -> Result<Option<Control>> {
        dispatch!(self, set_account_state(change))
    }

    pub async fn account_turns(&self) -> Result<Vec<Reservation>> {
        dispatch!(self, account_turns())
    }

    /// Take the account's slot for `turn`, atomically. Returns the holder:
    /// `turn` itself when it was free, else the existing reservation.
    pub async fn reserve_account_turn(&self, turn: &Reservation) -> Result<Reservation> {
        dispatch!(self, reserve_account_turn(turn))
    }

    /// `reserved` -> `may_have_dispatched` for exactly this turn, before its
    /// prompt is enqueued. `false` when the reservation is not this turn's.
    pub async fn mark_account_turn_dispatched(
        &self,
        account_id: &str,
        turn_id: &str,
        worker_epoch: &str,
        now_ms: i64,
    ) -> Result<bool> {
        dispatch!(
            self,
            mark_account_turn_dispatched(account_id, turn_id, worker_epoch, now_ms)
        )
    }

    /// Release exactly this turn's slot; `only` restricts the release to a
    /// reservation still in that state.
    pub async fn release_account_turn(
        &self,
        account_id: &str,
        turn_id: &str,
        only: Option<ReservationState>,
    ) -> Result<bool> {
        dispatch!(self, release_account_turn(account_id, turn_id, only))
    }

    pub async fn account_fences(&self) -> Result<Vec<Fence>> {
        dispatch!(self, account_fences())
    }

    /// Record the outcome of one fence attempt; a confirmation applies only
    /// to the same pending operation.
    pub async fn record_account_fence(
        &self,
        fence: &Fence,
        confirmed: bool,
        detail: &str,
        now_ms: i64,
    ) -> Result<bool> {
        dispatch!(self, record_account_fence(fence, confirmed, detail, now_ms))
    }

    /// Machines holding a native generation of the account.
    pub async fn account_native_machines(&self, account_id: &str) -> Result<Vec<String>> {
        dispatch!(self, account_native_machines(account_id))
    }
}

macro_rules! implementation {
    ($backend:ty, $begin:literal) => {
        impl $backend {
            async fn account_controls(&self) -> Result<Vec<Control>> {
                let rows: Vec<ControlRow> = sqlx::query_as(
                    "SELECT account_id, owner_user_id, provider_id, state, reason, until_ms, \
                     updated_at_ms, revision FROM provider_account_controls ORDER BY account_id",
                )
                .fetch_all(&self.pool)
                .await
                .context("SELECT account controls")?;
                rows.into_iter().map(control).collect()
            }

            async fn set_account_state(&self, change: &StateChange<'_>) -> Result<Option<Control>> {
                let mut tx = self.pool.begin_with($begin).await.context("begin account state")?;
                let current: Option<i64> = sqlx::query_scalar(
                    "SELECT revision FROM provider_account_controls WHERE account_id = $1",
                )
                .bind(change.account_id)
                .fetch_optional(&mut *tx)
                .await
                .context("SELECT account control revision")?;
                if change
                    .expected_revision
                    .is_some_and(|expected| expected != current.unwrap_or(0))
                {
                    return Ok(None);
                }
                let revision = current.unwrap_or(0).saturating_add(1);
                let reason = bounded(change.reason, 256);
                sqlx::query(
                    "INSERT INTO provider_account_controls(account_id, owner_user_id, provider_id, \
                     state, reason, until_ms, updated_at_ms, revision) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                     ON CONFLICT(account_id) DO UPDATE SET state = EXCLUDED.state, \
                     reason = EXCLUDED.reason, until_ms = EXCLUDED.until_ms, \
                     updated_at_ms = EXCLUDED.updated_at_ms, revision = EXCLUDED.revision",
                )
                .bind(change.account_id)
                .bind(change.owner_user_id)
                .bind(change.provider_id)
                .bind(change.state.as_str())
                .bind(&reason)
                .bind(change.until_ms)
                .bind(change.now_ms)
                .bind(revision)
                .execute(&mut *tx)
                .await
                .context("UPSERT account control")?;
                if let Some((action, machines)) = change.fence {
                    for machine in machines {
                        sqlx::query(
                            "INSERT INTO provider_account_fences(account_id, machine_id, action, \
                             operation_id, state, detail, updated_at_ms) \
                             VALUES ($1, $2, $3, $4, 'pending', '', $5) \
                             ON CONFLICT(account_id, machine_id) DO UPDATE SET \
                             action = EXCLUDED.action, operation_id = EXCLUDED.operation_id, \
                             state = 'pending', detail = '', updated_at_ms = EXCLUDED.updated_at_ms",
                        )
                        .bind(change.account_id)
                        .bind(machine)
                        .bind(action_name(action))
                        .bind(fence_operation_id(change.account_id, machine, action, revision))
                        .bind(change.now_ms)
                        .execute(&mut *tx)
                        .await
                        .context("UPSERT account fence")?;
                    }
                }
                tx.commit().await.context("commit account state")?;
                Ok(Some(Control {
                    account_id: change.account_id.to_owned(),
                    owner_user_id: change.owner_user_id.to_owned(),
                    provider_id: change.provider_id.to_owned(),
                    state: change.state,
                    reason,
                    until_ms: change.until_ms,
                    updated_at_ms: change.now_ms,
                    revision,
                }))
            }

            async fn account_turns(&self) -> Result<Vec<Reservation>> {
                let rows: Vec<TurnRow> = sqlx::query_as(
                    "SELECT account_id, owner_user_id, session_id, turn_id, worker_epoch, after_seq, \
                     state, created_at_ms, updated_at_ms FROM provider_account_turns \
                     ORDER BY account_id",
                )
                .fetch_all(&self.pool)
                .await
                .context("SELECT account turns")?;
                rows.into_iter().map(turn).collect()
            }

            async fn reserve_account_turn(&self, wanted: &Reservation) -> Result<Reservation> {
                let mut tx = self.pool.begin_with($begin).await.context("begin account turn")?;
                sqlx::query(
                    "INSERT INTO provider_account_turns(account_id, owner_user_id, session_id, \
                     turn_id, worker_epoch, after_seq, state, created_at_ms, updated_at_ms) \
                     VALUES ($1, $2, $3, $4, $5, $6, 'reserved', $7, $7) \
                     ON CONFLICT(account_id) DO NOTHING",
                )
                .bind(&wanted.account_id)
                .bind(&wanted.owner_user_id)
                .bind(&wanted.session_id)
                .bind(&wanted.turn_id)
                .bind(&wanted.worker_epoch)
                .bind(wanted.after_seq)
                .bind(wanted.created_at_ms)
                .execute(&mut *tx)
                .await
                .context("INSERT account turn")?;
                let row: TurnRow = sqlx::query_as(
                    "SELECT account_id, owner_user_id, session_id, turn_id, worker_epoch, after_seq, \
                     state, created_at_ms, updated_at_ms FROM provider_account_turns \
                     WHERE account_id = $1",
                )
                .bind(&wanted.account_id)
                .fetch_one(&mut *tx)
                .await
                .context("SELECT account turn holder")?;
                tx.commit().await.context("commit account turn")?;
                turn(row)
            }

            async fn mark_account_turn_dispatched(
                &self,
                account_id: &str,
                turn_id: &str,
                worker_epoch: &str,
                now_ms: i64,
            ) -> Result<bool> {
                let result = sqlx::query(
                    "UPDATE provider_account_turns SET state = 'may_have_dispatched', \
                     worker_epoch = $3, updated_at_ms = $4 \
                     WHERE account_id = $1 AND turn_id = $2 AND state = 'reserved'",
                )
                .bind(account_id)
                .bind(turn_id)
                .bind(worker_epoch)
                .bind(now_ms)
                .execute(&self.pool)
                .await
                .context("UPDATE account turn dispatched")?;
                Ok(result.rows_affected() == 1)
            }

            async fn release_account_turn(
                &self,
                account_id: &str,
                turn_id: &str,
                only: Option<ReservationState>,
            ) -> Result<bool> {
                let result = sqlx::query(
                    "DELETE FROM provider_account_turns WHERE account_id = $1 AND turn_id = $2 \
                     AND ($3 IS NULL OR state = $3)",
                )
                .bind(account_id)
                .bind(turn_id)
                .bind(only.map(ReservationState::as_str))
                .execute(&self.pool)
                .await
                .context("DELETE account turn")?;
                Ok(result.rows_affected() == 1)
            }

            async fn account_fences(&self) -> Result<Vec<Fence>> {
                let rows: Vec<FenceRow> = sqlx::query_as(
                    "SELECT account_id, machine_id, action, operation_id, state, detail, \
                     updated_at_ms FROM provider_account_fences ORDER BY account_id, machine_id",
                )
                .fetch_all(&self.pool)
                .await
                .context("SELECT account fences")?;
                rows.into_iter().map(fence).collect()
            }

            async fn record_account_fence(
                &self,
                fence: &Fence,
                confirmed: bool,
                detail: &str,
                now_ms: i64,
            ) -> Result<bool> {
                let result = sqlx::query(
                    "UPDATE provider_account_fences SET state = $4, detail = $5, updated_at_ms = $6 \
                     WHERE account_id = $1 AND machine_id = $2 AND operation_id = $3 \
                     AND state = 'pending'",
                )
                .bind(&fence.account_id)
                .bind(&fence.machine_id)
                .bind(&fence.operation_id)
                .bind(if confirmed { "confirmed" } else { "pending" })
                .bind(bounded(detail, 512))
                .bind(now_ms)
                .execute(&self.pool)
                .await
                .context("UPDATE account fence")?;
                Ok(result.rows_affected() == 1)
            }

            async fn account_native_machines(&self, account_id: &str) -> Result<Vec<String>> {
                sqlx::query_scalar(
                    "SELECT DISTINCT machine_id FROM provider_native_generations \
                     WHERE account_id = $1 ORDER BY machine_id",
                )
                .bind(account_id)
                .fetch_all(&self.pool)
                .await
                .context("SELECT account native Machines")
            }
        }
    };
}

implementation!(PostgresStorage, "BEGIN");
implementation!(SqliteStorage, "BEGIN IMMEDIATE");

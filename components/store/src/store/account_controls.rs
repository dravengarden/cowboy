//! Durable account controls of named-only Providers (`crate::account_controls`):
//! the per-account state and the Machine fences the Controller owes each
//! account's daemon. Every state change that
//! requires fences writes them in the same transaction, so a fence is never
//! lost between the durable state and the Machine command.

use super::{PostgresStorage, SqliteStorage, StorageBackend, Store};
use crate::account_controls::{AccountState, Control, Fence, FenceState, fence_operation_id};
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

/// An automatic transition of an account (a backend signal), applied in one
/// transaction: the state change (never lowering a more severe state, never
/// replacing `blocked`) and its stop fences.
#[derive(Clone, Debug)]
pub struct Raise<'a> {
    pub account_id: &'a str,
    pub owner_user_id: &'a str,
    pub provider_id: &'a str,
    /// The state to raise the account to, with its reason and end.
    pub state: (AccountState, &'a str, Option<i64>),
    /// Stop every unit of the account even when the state is not raised.
    pub stop_units: bool,
    /// The account's Machines, for the stop fences.
    pub machines: &'a [String],
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

    /// The account's current record, read from the store (admission paths
    /// never trust a cached view).
    pub async fn account_control(&self, account_id: &str) -> Result<Option<Control>> {
        dispatch!(self, account_control(account_id))
    }

    /// Apply one automatic transition atomically (see [`Raise`]); `true`
    /// when anything was written.
    pub async fn raise_account_state(&self, raise: &Raise<'_>) -> Result<bool> {
        dispatch!(self, raise_account_state(raise))
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

    /// Queue a pending stop fence of control `revision` on each of
    /// `machines` that has no fence of the account yet (a Machine that
    /// gained the account after its stop was written); the number added.
    pub async fn add_missing_stop_fences(
        &self,
        account_id: &str,
        machines: &[String],
        revision: i64,
        now_ms: i64,
    ) -> Result<u64> {
        dispatch!(
            self,
            add_missing_stop_fences(account_id, machines, revision, now_ms)
        )
    }
}

macro_rules! implementation {
    ($backend:ty, $begin:literal, $db:ty) => {
        async fn write_control(tx: &mut sqlx::Transaction<'_, $db>, control: &Control) -> Result<()> {
            sqlx::query(
                "INSERT INTO provider_account_controls(account_id, owner_user_id, provider_id, \
                 state, reason, until_ms, updated_at_ms, revision) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT(account_id) DO UPDATE SET state = EXCLUDED.state, \
                 reason = EXCLUDED.reason, until_ms = EXCLUDED.until_ms, \
                 updated_at_ms = EXCLUDED.updated_at_ms, revision = EXCLUDED.revision",
            )
            .bind(&control.account_id)
            .bind(&control.owner_user_id)
            .bind(&control.provider_id)
            .bind(control.state.as_str())
            .bind(&control.reason)
            .bind(control.until_ms)
            .bind(control.updated_at_ms)
            .bind(control.revision)
            .execute(&mut **tx)
            .await
            .context("UPSERT account control")?;
            Ok(())
        }

        /// Replace the account's fence on `machine` with a pending `action`.
        async fn write_fence(
            tx: &mut sqlx::Transaction<'_, $db>,
            account_id: &str,
            machine: &str,
            action: AccountFenceAction,
            revision: i64,
            now_ms: i64,
        ) -> Result<()> {
            sqlx::query(
                "INSERT INTO provider_account_fences(account_id, machine_id, action, \
                 operation_id, state, detail, updated_at_ms) \
                 VALUES ($1, $2, $3, $4, 'pending', '', $5) \
                 ON CONFLICT(account_id, machine_id) DO UPDATE SET \
                 action = EXCLUDED.action, operation_id = EXCLUDED.operation_id, \
                 state = 'pending', detail = '', updated_at_ms = EXCLUDED.updated_at_ms",
            )
            .bind(account_id)
            .bind(machine)
            .bind(action_name(action))
            .bind(fence_operation_id(account_id, machine, action, revision))
            .bind(now_ms)
            .execute(&mut **tx)
            .await
            .context("UPSERT account fence")?;
            Ok(())
        }

        impl $backend {
            pub(super) async fn account_controls(&self) -> Result<Vec<Control>> {
                let rows: Vec<ControlRow> = sqlx::query_as(
                    "SELECT account_id, owner_user_id, provider_id, state, reason, until_ms, \
                     updated_at_ms, revision FROM provider_account_controls ORDER BY account_id",
                )
                .fetch_all(&self.pool)
                .await
                .context("SELECT account controls")?;
                rows.into_iter().map(control).collect()
            }

            pub(super) async fn set_account_state(&self, change: &StateChange<'_>) -> Result<Option<Control>> {
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
                let written = Control {
                    account_id: change.account_id.to_owned(),
                    owner_user_id: change.owner_user_id.to_owned(),
                    provider_id: change.provider_id.to_owned(),
                    state: change.state,
                    reason,
                    until_ms: change.until_ms,
                    updated_at_ms: change.now_ms,
                    revision,
                };
                write_control(&mut tx, &written).await?;
                if let Some((action, machines)) = change.fence {
                    for machine in machines {
                        write_fence(&mut tx, change.account_id, machine, action, revision, change.now_ms)
                            .await?;
                    }
                }
                tx.commit().await.context("commit account state")?;
                Ok(Some(written))
            }

            pub(super) async fn account_control(&self, account_id: &str) -> Result<Option<Control>> {
                let row: Option<ControlRow> = sqlx::query_as(
                    "SELECT account_id, owner_user_id, provider_id, state, reason, until_ms, \
                     updated_at_ms, revision FROM provider_account_controls WHERE account_id = $1",
                )
                .bind(account_id)
                .fetch_optional(&self.pool)
                .await
                .context("SELECT account control")?;
                row.map(control).transpose()
            }

            pub(super) async fn raise_account_state(&self, raise: &Raise<'_>) -> Result<bool> {
                let mut tx = self.pool.begin_with($begin).await.context("begin account raise")?;
                let current: Option<(String, String, Option<i64>, i64)> = sqlx::query_as(
                    "SELECT state, reason, until_ms, revision FROM provider_account_controls \
                     WHERE account_id = $1",
                )
                .bind(raise.account_id)
                .fetch_optional(&mut *tx)
                .await
                .context("SELECT account control for raise")?;
                let (current_state, current_reason, current_until, revision) = match current {
                    Some((state, reason, until, revision)) => (
                        AccountState::parse(&state).context("unknown stored account state")?,
                        reason,
                        until,
                        revision,
                    ),
                    None => (AccountState::Ok, String::new(), None, 0),
                };
                // Compared inside the transaction: an owner's block (or any
                // more severe state) written meanwhile is never replaced.
                let (next, reason, until) = raise.state;
                let raised = current_state != AccountState::Blocked
                    && next.severity() >= current_state.severity();
                if !raised && !raise.stop_units {
                    return Ok(false);
                }
                let (state, reason, until) = if raised {
                    (next, bounded(reason, 256), until)
                } else {
                    (current_state, current_reason, current_until)
                };
                let revision = revision.saturating_add(1);
                let written = Control {
                    account_id: raise.account_id.to_owned(),
                    owner_user_id: raise.owner_user_id.to_owned(),
                    provider_id: raise.provider_id.to_owned(),
                    state,
                    reason,
                    until_ms: until,
                    updated_at_ms: raise.now_ms,
                    revision,
                };
                write_control(&mut tx, &written).await?;
                if raise.stop_units || state.stops_units() {
                    for machine in raise.machines {
                        write_fence(
                            &mut tx,
                            raise.account_id,
                            machine,
                            AccountFenceAction::Stop,
                            revision,
                            raise.now_ms,
                        )
                        .await?;
                    }
                }
                tx.commit().await.context("commit account raise")?;
                Ok(true)
            }

            pub(super) async fn account_fences(&self) -> Result<Vec<Fence>> {
                let rows: Vec<FenceRow> = sqlx::query_as(
                    "SELECT account_id, machine_id, action, operation_id, state, detail, \
                     updated_at_ms FROM provider_account_fences ORDER BY account_id, machine_id",
                )
                .fetch_all(&self.pool)
                .await
                .context("SELECT account fences")?;
                rows.into_iter().map(fence).collect()
            }

            pub(super) async fn record_account_fence(
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

            pub(super) async fn add_missing_stop_fences(
                &self,
                account_id: &str,
                machines: &[String],
                revision: i64,
                now_ms: i64,
            ) -> Result<u64> {
                let mut added = 0;
                for machine in machines {
                    let action = AccountFenceAction::Stop;
                    let result = sqlx::query(
                        "INSERT INTO provider_account_fences(account_id, machine_id, action, \
                         operation_id, state, detail, updated_at_ms) \
                         VALUES ($1, $2, $3, $4, 'pending', '', $5) \
                         ON CONFLICT(account_id, machine_id) DO NOTHING",
                    )
                    .bind(account_id)
                    .bind(machine)
                    .bind(action_name(action))
                    .bind(fence_operation_id(account_id, machine, action, revision))
                    .bind(now_ms)
                    .execute(&self.pool)
                    .await
                    .context("INSERT missing account fence")?;
                    added += result.rows_affected();
                }
                Ok(added)
            }

            pub(super) async fn account_native_machines(&self, account_id: &str) -> Result<Vec<String>> {
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

mod postgres {
    use super::*;
    implementation!(PostgresStorage, "BEGIN", sqlx::Postgres);
}
mod sqlite {
    use super::*;
    implementation!(SqliteStorage, "BEGIN IMMEDIATE", sqlx::Sqlite);
}

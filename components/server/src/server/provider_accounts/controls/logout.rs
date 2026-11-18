//! The kill switch's history-preserving native logout
//! (`docs/antigravity-real-accounts-design.md`, "Kill switch and logout").
//!
//! Only a blocked account whose units are confirmed stopped may be logged
//! out. The request is durable (one record per account, one operation ID per
//! control revision) before any Machine is asked; each Machine with a
//! signed-in generation of the account then runs the native logout through
//! the existing Machine path (the daemon's `logout` unit and its
//! `logout_cleanup`, which removes only the vendor token). A Machine that is
//! offline keeps its part pending until it reconnects; the block holds
//! meanwhile. Logout never deletes the account or its home.
use super::*;
use crate::account_controls::{LogoutTargetState, logout_operation_id, logout_state};
use crate::provider_account::{NativeLogoutObservation, NativeLogoutRequest};
use crate::store::provider_accounts::logins::native_logout::Completion;
use serde::{Deserialize, Serialize};

/// The durable record of one account's logout request (an internal Hub
/// setting, `provider_account_logout:<account>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::server) struct Record {
    pub operation_id: String,
    pub owner_user_id: String,
    pub provider_id: String,
    /// The control revision of the block this logout belongs to.
    pub revision: i64,
    pub requested_at_ms: i64,
    pub targets: Vec<Target>,
}

/// One Machine's signed-in generation and its logout outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::server) struct Target {
    pub machine_id: String,
    pub login_operation: String,
    pub login_request_id: String,
    pub generation: u64,
    pub recipient_fingerprint: String,
    pub state: LogoutTargetState,
    /// The request may have reached the Machine: never cancel it then.
    #[serde(default)]
    pub dispatched: bool,
    /// The operation of an earlier block's logout of this generation that
    /// may have reached the Machine (its reply lost): retried as is, since
    /// the Machine refuses another operation for the generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub credential_cleared: Option<bool>,
    #[serde(default)]
    pub history_retained: Option<bool>,
    #[serde(default)]
    pub detail: Option<String>,
    pub updated_at_ms: i64,
}

/// Why the Machine's logout evidence was not recorded.
enum Recording {
    Rejected(String),
    Transient(String),
}

impl Recording {
    fn detail(self) -> String {
        match self {
            Self::Rejected(detail) | Self::Transient(detail) => detail,
        }
    }
}

/// The prefix of a native logout failure that proves nothing was enqueued.
pub(in crate::server) const UNSENT: &str = "unsent: ";

pub(in crate::server) fn logout_key(account: &str) -> String {
    format!(
        "{}{account}",
        crate::core::settings_keys::PROVIDER_ACCOUNT_LOGOUT_PREFIX
    )
}

impl Controls {
    pub(in crate::server) fn logout_record(&self, account: &str) -> Option<Record> {
        self.env
            .setting(&logout_key(account))
            .and_then(|value| serde_json::from_value(value).ok())
    }

    async fn write_logout(&self, account: &str, record: &Record) -> Result<(), &'static str> {
        let key = logout_key(account);
        let value = serde_json::to_value(record).map_err(|_| "unavailable")?;
        self.store
            .put_setting(&key, &value)
            .await
            .map_err(|_| "outcome_unknown")?;
        self.env.set_setting(key, value);
        Ok(())
    }

    /// The owner's logout of a blocked, stopped account. Repeating it for
    /// the same block is idempotent; a later block starts a new operation.
    pub(in crate::server) async fn request_logout(
        &self,
        control: &Control,
        owner: &str,
        confirm: bool,
    ) -> Result<(), &'static str> {
        if !confirm {
            return Err("confirmation_required");
        }
        policy::owner_transition(policy::OwnerAction::Logout, control.state)?;
        if !self.stop_confirmed(&control.account_id) {
            return Err("account_stop_unconfirmed");
        }
        let account = control.account_id.as_str();
        let operation_id = logout_operation_id(account, control.revision);
        let id: ProviderAccountId = account.parse().map_err(|_| "account_unavailable")?;
        // Every Machine's newest dispatched sign-in, not only completed
        // receipts: a token may remain where the Controller recorded none.
        let found = self
            .store
            .native_logout_targets(owner, &id)
            .await
            .map_err(|_| "unavailable")?;
        let now = now_ms();
        let previous = self.logout_record(account);
        let targets = found
            .into_iter()
            .map(|target| {
                // A logout of the same generation an earlier block may have
                // delivered keeps its operation until its receipt arrives.
                let carried = previous.as_ref().and_then(|previous| {
                    previous
                        .targets
                        .iter()
                        .find(|old| {
                            old.machine_id == target.machine_id
                                && old.generation == target.generation
                                && old.dispatched
                                && matches!(
                                    old.state,
                                    LogoutTargetState::Pending | LogoutTargetState::Unconfirmed
                                )
                        })
                        .map(|old| {
                            old.operation_id
                                .clone()
                                .unwrap_or_else(|| previous.operation_id.clone())
                        })
                });
                Target {
                    machine_id: target.machine_id,
                    login_operation: target.login_operation,
                    login_request_id: target.login_request_id,
                    generation: target.generation,
                    recipient_fingerprint: target.recipient_fingerprint,
                    state: LogoutTargetState::Pending,
                    dispatched: carried.is_some(),
                    operation_id: carried,
                    credential_cleared: None,
                    history_retained: None,
                    detail: None,
                    updated_at_ms: now,
                }
            })
            .collect();
        let record = Record {
            operation_id,
            owner_user_id: owner.to_owned(),
            provider_id: control.provider_id.clone(),
            revision: control.revision,
            requested_at_ms: now,
            targets,
        };
        {
            // Decided and written under the writer lock: a repeated request
            // for the same block never rewrites (rolls back) its progress.
            let _write = self.cache.writes.lock().await;
            if self
                .logout_record(account)
                .is_some_and(|current| current.operation_id == record.operation_id)
            {
                drop(_write);
                self.run_logouts();
                return Ok(());
            }
            self.write_logout(account, &record).await?;
        }
        tracing::info!(
            account,
            operation = %record.operation_id,
            machines = record.targets.len(),
            "account logout requested"
        );
        self.run_logouts();
        Ok(())
    }

    /// Deliver every pending logout target whose Machine is connected, one
    /// command per target at a time, always under the same operation ID.
    pub(in crate::server) fn run_logouts(&self) {
        let accounts: Vec<String> = self.cache.controls.read().keys().cloned().collect();
        for account in accounts {
            let Some(record) = self.logout_record(&account) else {
                continue;
            };
            for target in &record.targets {
                let key = format!("{}:{}", record.operation_id, target.machine_id);
                if target.state != LogoutTargetState::Pending
                    || !self.env.machine_connected(&target.machine_id)
                    || !self.cache.fencing.lock().insert(key.clone())
                {
                    continue;
                }
                let controls = self.clone();
                let (account, operation, machine) = (
                    account.clone(),
                    record.operation_id.clone(),
                    target.machine_id.clone(),
                );
                tokio::spawn(async move {
                    controls
                        .deliver_logout(&account, &operation, &machine)
                        .await;
                    controls.cache.fencing.lock().remove(&key);
                });
            }
        }
    }

    /// Cancel every target of the account's logout that no Machine was ever
    /// asked, durably. Called by the unblock under the writer lock, before
    /// the state leaves `blocked`, so an unsent logout of a lifted block can
    /// never fire on a later block.
    pub(super) async fn cancel_undispatched_logouts(
        &self,
        account: &str,
    ) -> Result<(), &'static str> {
        let Some(mut record) = self.logout_record(account) else {
            return Ok(());
        };
        let mut changed = false;
        for target in &mut record.targets {
            if target.state == LogoutTargetState::Pending && !target.dispatched {
                target.state = LogoutTargetState::Cancelled;
                target.detail = Some("the account was unblocked first".into());
                target.updated_at_ms = now_ms();
                changed = true;
            }
        }
        if changed {
            self.write_logout(account, &record).await?;
        }
        Ok(())
    }

    /// Update one target of the current record under the writer lock.
    async fn update_logout_target(
        &self,
        account: &str,
        operation: &str,
        machine: &str,
        update: impl FnOnce(&Self, &Record, &mut Target),
    ) -> Option<(Record, Target)> {
        let _write = self.cache.writes.lock().await;
        let mut record = self
            .logout_record(account)
            .filter(|record| record.operation_id == operation)?;
        let snapshot = record.clone();
        let target = record
            .targets
            .iter_mut()
            .find(|target| target.machine_id == machine)?;
        update(self, &snapshot, target);
        target.updated_at_ms = now_ms();
        let target = target.clone();
        if let Err(error) = self.write_logout(account, &record).await {
            tracing::warn!(
                account,
                machine,
                error,
                "recording an account logout failed"
            );
            return None;
        }
        Some((record, target))
    }

    pub(super) async fn deliver_logout(&self, account: &str, operation: &str, machine: &str) {
        // Decided under the writer lock, which every state write holds until
        // the cache shows it: only the block this logout belongs to (still
        // in force, at or after the requesting revision) dispatches it; an
        // unblock cancelled every target not yet asked. A request that may
        // have reached the Machine keeps its operation.
        let Some((record, target)) = self
            .update_logout_target(account, operation, machine, |controls, record, target| {
                if target.state != LogoutTargetState::Pending || target.dispatched {
                    return;
                }
                // Exactly the block that requested it: a blocked account's
                // revision does not move, so a later revision means it was
                // unblocked meanwhile (even if that unblock's cancellation
                // was interrupted).
                let current = controls.cache.control(account).is_some_and(|control| {
                    control.state == AccountState::Blocked && control.revision == record.revision
                });
                if current {
                    target.dispatched = true;
                } else {
                    target.state = LogoutTargetState::Cancelled;
                    target.detail = Some("the account was unblocked first".into());
                }
            })
            .await
        else {
            return;
        };
        if target.state != LogoutTargetState::Pending {
            return;
        }
        let Ok(account_id) = account.parse::<ProviderAccountId>() else {
            return;
        };
        let request = NativeLogoutRequest {
            account_id,
            provider_id: record.provider_id.clone(),
            generation: target.generation,
            login_request_id: target.login_request_id.clone(),
            operation_id: target
                .operation_id
                .clone()
                .unwrap_or_else(|| operation.to_owned()),
        };
        let observation = match self
            .env
            .native_logout(
                machine.to_owned(),
                target.recipient_fingerprint.clone(),
                request,
            )
            .await
        {
            Ok(observation) => observation,
            Err(detail) => {
                tracing::warn!(account, machine, %detail, "account logout unconfirmed; retrying");
                // Refused before anything was enqueued: the Machine was never
                // asked, so an unblock may still cancel it.
                let unsent = detail.starts_with(UNSENT);
                self.update_logout_target(account, operation, machine, |_, _, target| {
                    if unsent && target.operation_id.is_none() {
                        target.dispatched = false;
                    }
                    target.detail = Some(detail);
                })
                .await;
                return;
            }
        };
        let recorded = self.record_logout(&record, &target, &observation).await;
        self.update_logout_target(account, operation, machine, |_, _, target| {
            target.credential_cleared = observation.credential_cleared;
            target.history_retained = observation.history_retained;
            target.state = match (observation.succeeded, &recorded) {
                // A receipt-store failure is transient: the target stays
                // pending and the same operation is retried; the Machine
                // replays its recorded outcome.
                (Some(_), Err(Recording::Transient(_))) => LogoutTargetState::Pending,
                (_, Err(_)) | (None, _) => LogoutTargetState::Unconfirmed,
                (Some(true), Ok(())) => LogoutTargetState::SignedOut,
                (Some(false), Ok(())) => LogoutTargetState::Failed,
            };
            target.detail = recorded.err().map(Recording::detail);
        })
        .await;
        tracing::info!(
            account,
            machine,
            succeeded = ?observation.succeeded,
            credential_cleared = ?observation.credential_cleared,
            history_retained = ?observation.history_retained,
            "account logout observed"
        );
    }

    /// Record the Machine's evidence on the generation's login receipt, so
    /// that login admits no session any more.
    async fn record_logout(
        &self,
        record: &Record,
        target: &Target,
        observation: &NativeLogoutObservation,
    ) -> Result<(), Recording> {
        if observation.succeeded.is_none() {
            return Err(Recording::Rejected(
                "the Machine could not establish the logout outcome".into(),
            ));
        }
        let completion = Completion {
            revocation_operation: target
                .operation_id
                .clone()
                .unwrap_or_else(|| record.operation_id.clone()),
            machine_id: target.machine_id.clone(),
            recipient_fingerprint: target.recipient_fingerprint.clone(),
            observation: observation.clone(),
        };
        self.store
            .record_control_logout(&record.owner_user_id, &target.login_operation, &completion)
            .await
            .map(drop)
            .map_err(|error| {
                let detail = error.to_string();
                // The store's own refusals are definitive; anything else
                // (a database outage) is retried.
                if detail.starts_with("native_logout_")
                    || detail == "account_unavailable"
                    || detail == "invalid_native_logout_completion"
                {
                    Recording::Rejected(detail)
                } else {
                    Recording::Transient(detail)
                }
            })
    }

    /// The owner-facing projection of an account's logout (no fingerprints
    /// or login operations).
    pub(in crate::server) fn logout_value(&self, account: &str) -> Value {
        let Some(record) = self.logout_record(account) else {
            return Value::Null;
        };
        let states: Vec<LogoutTargetState> =
            record.targets.iter().map(|target| target.state).collect();
        json!({
            "operation_id": record.operation_id,
            "revision": record.revision,
            "requested_at_ms": record.requested_at_ms,
            "state": logout_state(&states),
            "targets": record.targets.iter().map(|target| json!({
                "machine_id": target.machine_id,
                "state": target.state,
                "credential_cleared": target.credential_cleared,
                "history_retained": target.history_retained,
                "detail": target.detail,
                "updated_at_ms": target.updated_at_ms,
            })).collect::<Vec<_>>(),
        })
    }
}

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
    #[serde(default)]
    pub credential_cleared: Option<bool>,
    #[serde(default)]
    pub history_retained: Option<bool>,
    #[serde(default)]
    pub detail: Option<String>,
    pub updated_at_ms: i64,
}

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
        if self
            .logout_record(account)
            .is_some_and(|record| record.operation_id == operation_id)
        {
            self.run_logouts();
            return Ok(());
        }
        let id: ProviderAccountId = account.parse().map_err(|_| "account_unavailable")?;
        let now = now_ms();
        let mut targets = Vec::new();
        for machine in self
            .store
            .account_native_machines(account)
            .await
            .map_err(|_| "unavailable")?
        {
            let Some((login_operation, completion)) = self
                .store
                .native_login_for_session(owner, &id, &machine)
                .await
                .map_err(|_| "unavailable")?
            else {
                continue;
            };
            targets.push(Target {
                machine_id: machine,
                login_operation,
                login_request_id: completion.profile.request_id.clone(),
                generation: completion.profile.generation,
                recipient_fingerprint: completion.recipient_fingerprint.clone(),
                state: LogoutTargetState::Pending,
                dispatched: false,
                credential_cleared: None,
                history_retained: None,
                detail: None,
                updated_at_ms: now,
            });
        }
        let record = Record {
            operation_id,
            owner_user_id: owner.to_owned(),
            provider_id: control.provider_id.clone(),
            revision: control.revision,
            requested_at_ms: now,
            targets,
        };
        {
            let _write = self.cache.writes.lock().await;
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
                let current = controls.cache.control(account).is_some_and(|control| {
                    control.state == AccountState::Blocked && control.revision >= record.revision
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
            operation_id: operation.to_owned(),
        };
        let observation = match self.env.native_logout(machine.to_owned(), request).await {
            Ok(observation) => observation,
            Err(detail) => {
                tracing::warn!(account, machine, %detail, "account logout unconfirmed; retrying");
                self.update_logout_target(account, operation, machine, |_, _, target| {
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
                (_, Err(_)) | (None, _) => LogoutTargetState::Unconfirmed,
                (Some(true), Ok(())) => LogoutTargetState::SignedOut,
                (Some(false), Ok(())) => LogoutTargetState::Failed,
            };
            target.detail = recorded.err();
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
    ) -> Result<(), String> {
        if observation.succeeded.is_none() {
            return Err("the Machine could not establish the logout outcome".into());
        }
        let completion = Completion {
            revocation_operation: record.operation_id.clone(),
            machine_id: target.machine_id.clone(),
            recipient_fingerprint: target.recipient_fingerprint.clone(),
            observation: observation.clone(),
        };
        self.store
            .record_control_logout(&record.owner_user_id, &target.login_operation, &completion)
            .await
            .map(drop)
            .map_err(|error| error.to_string())
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

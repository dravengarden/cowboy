//! Durable evidence for the finite core installer, never a replayable grant.
//! The protocol-seven response is an observed ACK, not a Machine step receipt.

use super::{Actor, bounded, digest};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_INSTALL_INTENT_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InstallIntent {
    pub schema: u16,
    pub operation_id: String,
    pub service_id: String,
    pub actor: Actor,
    pub machine_id: String,
    pub plugin_id: String,
    pub plugin_kind: cowboy_plugin_sdk::PluginKind,
    pub plugin_version: String,
    pub generation_digest: String,
    pub contract_fingerprint: String,
    /// Hash of the complete trusted DesiredPlugin, not its URL or package data.
    pub envelope_digest: String,
    /// Correlates the single live dispatch. It cannot query a durable Machine
    /// receipt, reconstruct a connection, or authorize dispatch after restart.
    pub request_id: String,
    pub expires_at_ms: i64,
}

pub(crate) fn valid_operation_id(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

impl InstallIntent {
    pub(crate) fn validate(&self) -> Result<()> {
        let actor = match &self.actor {
            Actor::Product { user_id } => user_id,
            Actor::Admin { account } => account,
        };
        ensure!(
            self.schema == 1 && valid_operation_id(&self.operation_id),
            "invalid install identity"
        );
        ensure!(
            bounded(&self.service_id, 128) && bounded(actor, 256),
            "invalid install owner"
        );
        ensure!(
            [&self.machine_id, &self.plugin_id]
                .into_iter()
                .all(|value| {
                    bounded(value, 128)
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                }),
            "invalid install target"
        );
        ensure!(
            self.plugin_kind != cowboy_plugin_sdk::PluginKind::AuthenticationProvider
                && self.plugin_version.len() <= 128
                && semver::Version::parse(&self.plugin_version).is_ok()
                && digest(&self.generation_digest)
                && digest(&self.contract_fingerprint)
                && digest(&self.envelope_digest),
            "invalid install release"
        );
        ensure!(
            self.request_id == format!("plugin-install-{}", self.operation_id),
            "invalid install correlation"
        );
        ensure!(
            self.expires_at_ms > 0 && self.expires_at_ms <= 9_007_199_254_740_991,
            "invalid install deadline"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallPhase {
    Prepared,
    SyncingAuthentication,
    Installing,
    MachineAcknowledged,
    Completed,
    AuthenticationPending,
    Aborted,
    NeedsAttention,
}

impl InstallPhase {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::SyncingAuthentication => "syncing_authentication",
            Self::Installing => "installing",
            Self::MachineAcknowledged => "machine_acknowledged",
            Self::Completed => "completed",
            Self::AuthenticationPending => "authentication_pending",
            Self::Aborted => "aborted",
            Self::NeedsAttention => "needs_attention",
        }
    }

    pub(crate) const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::AuthenticationPending | Self::Aborted
        )
    }

    pub(crate) const fn permits(self, next: Self) -> bool {
        if !self.terminal()
            && !matches!(self, Self::NeedsAttention)
            && matches!(next, Self::NeedsAttention)
        {
            return true;
        }
        matches!(
            (self, next),
            (
                Self::Prepared,
                Self::SyncingAuthentication | Self::Installing | Self::Aborted
            ) | (
                Self::SyncingAuthentication,
                Self::Installing | Self::Aborted
            ) | (Self::Installing, Self::MachineAcknowledged | Self::Aborted)
                | (
                    Self::MachineAcknowledged,
                    Self::Completed | Self::AuthenticationPending
                )
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InstallProblem {
    Interrupted,
    PreconditionsChanged,
    AuthenticationSyncFailed,
    TransportNotSent,
    MachineRejected,
    UnknownMachineOutcome,
    StorageFailure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct InstallOperation {
    pub intent: InstallIntent,
    pub phase: InstallPhase,
    pub problem: Option<InstallProblem>,
    pub attention_from: Option<InstallPhase>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl InstallOperation {
    pub(crate) fn validate(&self) -> Result<()> {
        self.intent.validate()?;
        ensure!(
            self.created_at_ms > 0
                && self.created_at_ms <= self.updated_at_ms
                && self.created_at_ms < self.intent.expires_at_ms
                && self.updated_at_ms <= 9_007_199_254_740_991,
            "invalid install evidence time"
        );
        ensure!(
            match self.phase {
                InstallPhase::NeedsAttention => self.problem.is_some(),
                InstallPhase::Aborted => matches!(
                    self.problem,
                    Some(
                        InstallProblem::PreconditionsChanged
                            | InstallProblem::AuthenticationSyncFailed
                            | InstallProblem::TransportNotSent
                    )
                ),
                InstallPhase::AuthenticationPending =>
                    self.problem == Some(InstallProblem::AuthenticationSyncFailed),
                _ => self.problem.is_none(),
            },
            "invalid install phase/problem combination"
        );
        ensure!(
            if self.phase == InstallPhase::NeedsAttention {
                self.problem.is_some()
                    && self.attention_from.is_some_and(|phase| {
                        !phase.terminal() && phase != InstallPhase::NeedsAttention
                    })
            } else {
                self.attention_from.is_none()
            },
            "invalid install interruption evidence"
        );
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn fixture(id: &str) -> InstallIntent {
    let operation_id = format!("installation-{id:0>16}");
    InstallIntent {
        schema: 1,
        request_id: format!("plugin-install-{operation_id}"),
        operation_id,
        service_id: "service-test".into(),
        actor: Actor::Product {
            user_id: "user-test".into(),
        },
        machine_id: "machine-test".into(),
        plugin_id: "victoria".into(),
        plugin_kind: cowboy_plugin_sdk::PluginKind::TelemetryBackend,
        plugin_version: "1.1.0".into(),
        generation_digest: format!("sha256:{}", "a".repeat(64)),
        contract_fingerprint: format!("sha256:{}", "b".repeat(64)),
        envelope_digest: format!("sha256:{}", "c".repeat(64)),
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 300_000,
    }
}

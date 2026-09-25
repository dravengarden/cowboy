//! Durable evidence for the finite core installer, never a replayable grant.
//! Schema one retains protocol-seven ACK evidence. Schema two binds exact
//! Machine installation CAS and typed receipts; neither schema grants replay.

use super::{Actor, bounded, digest};
use crate::machine_protocol::plugin_install::{
    InstallOutcome, InstallReceipt, InstallStep, InstallTarget,
};
use crate::operation_budget::OperationBudget;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_INSTALL_INTENT_BYTES: usize = 4096;
pub(crate) const MAX_INSTALL_STAGING_RESOLUTION_BYTES: usize = 4096;

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
    /// Absent only for retained schema-one attempts. Never inferred from an
    /// empty inventory or populated after a confirmation has been consumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_target: Option<InstallTarget>,
    /// Correlates the single live dispatch. The full schema-two step can query
    /// evidence, but neither identity can authorize dispatch after restart.
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
            matches!(
                (self.schema, &self.machine_target),
                (1, None) | (2, Some(_))
            ) && valid_operation_id(&self.operation_id),
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
        if let Some(target) = &self.machine_target {
            target.validate()?;
        }
        Ok(())
    }

    pub(crate) fn machine_step(&self) -> Result<InstallStep> {
        self.validate()?;
        let expected = self
            .machine_target
            .clone()
            .ok_or_else(|| anyhow::anyhow!("legacy installation has no durable Machine step"))?;
        let step = InstallStep {
            schema: 1,
            operation_id: self.operation_id.clone(),
            service_id: self.service_id.clone(),
            machine_id: self.machine_id.clone(),
            plugin_id: self.plugin_id.clone(),
            plugin_kind: self.plugin_kind,
            plugin_version: self.plugin_version.clone(),
            generation_digest: self.generation_digest.clone(),
            contract_fingerprint: self.contract_fingerprint.clone(),
            envelope_digest: self.envelope_digest.clone(),
            expected,
            expires_at_ms: self.expires_at_ms,
            plan_digest: crate::machine_protocol::plugin_step::digest(&serde_json::to_vec(self)?),
        };
        step.validate()?;
        Ok(step)
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
    pub machine_receipt: Option<InstallReceipt>,
}

impl InstallOperation {
    pub(crate) fn validate(&self) -> Result<()> {
        self.intent.validate()?;
        if let Some(receipt) = &self.machine_receipt {
            ensure!(
                receipt.matches(&self.intent.machine_step()?),
                "install Machine receipt identity mismatch"
            );
        }
        if self.intent.schema == 2 {
            self.validate_machine_progress()?;
        }
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
                InstallPhase::Aborted =>
                    (self.intent.schema == 2
                        && self.problem == Some(InstallProblem::MachineRejected)
                        && matches!(
                            self.machine_receipt.as_ref().map(|r| &r.outcome),
                            Some(InstallOutcome::Rejected { .. })
                        ))
                        || matches!(
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

    fn validate_machine_progress(&self) -> Result<()> {
        let outcome = self.machine_receipt.as_ref().map(|r| &r.outcome);
        ensure!(
            match self.phase {
                InstallPhase::MachineAcknowledged
                | InstallPhase::Completed
                | InstallPhase::AuthenticationPending =>
                    matches!(outcome, Some(InstallOutcome::Applied { .. })),
                InstallPhase::Prepared
                | InstallPhase::SyncingAuthentication
                | InstallPhase::Installing => outcome.is_none(),
                InstallPhase::Aborted =>
                    outcome.is_none()
                        || (matches!(outcome, Some(InstallOutcome::Rejected { .. }))
                            && self.problem == Some(InstallProblem::MachineRejected)),
                InstallPhase::NeedsAttention => match outcome {
                    None => self.attention_from != Some(InstallPhase::MachineAcknowledged),
                    Some(InstallOutcome::Pending { .. } | InstallOutcome::Unknown { .. }) =>
                        self.attention_from == Some(InstallPhase::Installing)
                            && self.problem == Some(InstallProblem::UnknownMachineOutcome),
                    Some(InstallOutcome::Applied { .. }) =>
                        self.attention_from == Some(InstallPhase::MachineAcknowledged)
                            && matches!(
                                self.problem,
                                Some(InstallProblem::Interrupted | InstallProblem::StorageFailure)
                            ),
                    Some(InstallOutcome::Rejected { .. }) => false,
                },
            },
            "install progress lacks matching Machine evidence"
        );
        Ok(())
    }
}

/// Durable proof that a fresh Operator inspected one exact failed staging
/// attempt and the Machine still reported its original installation target.
/// This retires only the Service/Machine slot fence; it is not installation
/// authority and carries no release envelope or credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InstallStagingResolutionIntent {
    pub schema: u16,
    pub resolution_id: String,
    pub operation_id: String,
    pub service_id: String,
    pub actor: Actor,
    pub machine_id: String,
    pub plugin_id: String,
    pub operation_digest: String,
    pub observed_target: InstallTarget,
}

impl InstallStagingResolutionIntent {
    pub(crate) fn validate(&self) -> Result<()> {
        let actor = match &self.actor {
            Actor::Product { user_id } => user_id,
            Actor::Admin { account } => account,
        };
        ensure!(
            self.schema == 1
                && valid_operation_id(&self.resolution_id)
                && valid_operation_id(&self.operation_id),
            "invalid install staging resolution identity"
        );
        ensure!(
            bounded(&self.service_id, 128) && bounded(actor, 256),
            "invalid install staging resolution owner"
        );
        ensure!(
            [&self.machine_id, &self.plugin_id]
                .into_iter()
                .all(|value| {
                    bounded(value, 128)
                        && value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                })
                && digest(&self.operation_digest),
            "invalid install staging resolution target"
        );
        self.observed_target.validate()
    }

    pub(crate) fn matches(
        &self,
        operation: &InstallOperation,
        observed_target: &InstallTarget,
    ) -> Result<bool> {
        self.validate()?;
        operation.validate()?;
        let retryable = operation
            .machine_receipt
            .as_ref()
            .is_some_and(|receipt| receipt.outcome.retryable_staging_failure());
        Ok(retryable
            && operation.intent.schema == 2
            && operation.phase == InstallPhase::NeedsAttention
            && operation.attention_from == Some(InstallPhase::Installing)
            && operation.intent.machine_target.as_ref() == Some(observed_target)
            && &self.observed_target == observed_target
            && self.operation_id == operation.intent.operation_id
            && self.service_id == operation.intent.service_id
            && self.machine_id == operation.intent.machine_id
            && self.plugin_id == operation.intent.plugin_id
            && self.operation_digest
                == crate::machine_protocol::plugin_step::digest(&serde_json::to_vec(operation)?))
    }
}

/// Ephemeral authority for the local durability commit. It cannot be decoded
/// from the serialized intent and retains the original monotonic time budget.
pub(crate) struct InstallStagingResolutionPermit {
    intent: InstallStagingResolutionIntent,
    budget: OperationBudget,
}

impl InstallStagingResolutionPermit {
    pub(crate) fn new(intent: InstallStagingResolutionIntent, budget: OperationBudget) -> Self {
        Self { intent, budget }
    }

    pub(crate) fn intent(&self) -> &InstallStagingResolutionIntent {
        &self.intent
    }

    pub(crate) fn within_budget(&self) -> bool {
        !self.budget.expired()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct InstallStagingResolutionReceipt {
    pub intent: InstallStagingResolutionIntent,
    pub resolved_at_ms: i64,
}

#[cfg(test)]
pub(crate) fn machine_fixture(id: &str) -> InstallIntent {
    InstallIntent {
        schema: 2,
        machine_target: Some(InstallTarget::Vacant {}),
        ..fixture(id)
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
        machine_target: None,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 300_000,
    }
}

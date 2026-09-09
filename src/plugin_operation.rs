//! Versioned Service-side uninstall evidence, not a serialized authorization.
//! Machine protocol 5-9 has no durable step receipt: interrupted remote effects
//! must stay fenced for reconciliation, never be replayed from these records.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub(crate) const MAX_OPERATIONS: i64 = 4096;
pub(crate) const MAX_INTENT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Actor {
    Product { user_id: String },
    Admin { account: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UninstallIntent {
    pub schema: u32,
    pub operation_id: String,
    pub service_id: String,
    pub actor: Actor,
    pub machine_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    pub contract_fingerprint: String,
    pub session_ids: Vec<String>,
    pub active_session_ids: Vec<String>,
    pub live_session_ids: Vec<String>,
    pub purge_after_ms: i64,
    pub expires_at_ms: i64,
}

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

impl UninstallIntent {
    pub(crate) fn machine_step(
        &self,
    ) -> Result<crate::machine_protocol::plugin_step::UninstallStep> {
        use crate::machine_protocol::plugin_step::{UninstallStep, digest};
        self.validate()?;
        let step = UninstallStep {
            schema: 1,
            operation_id: self.operation_id.clone(),
            service_id: self.service_id.clone(),
            machine_id: self.machine_id.clone(),
            plan_digest: digest(&serde_json::to_vec(self)?),
            plugin_id: self.plugin_id.clone(),
            plugin_version: self.plugin_version.clone(),
            generation_digest: self.generation_digest.clone(),
            contract_fingerprint: self.contract_fingerprint.clone(),
            expires_at_ms: self.expires_at_ms,
        };
        step.validate()?;
        Ok(step)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "unsupported uninstall intent schema");
        ensure!(
            (16..=128).contains(&self.operation_id.len())
                && self
                    .operation_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
            "invalid uninstall operation identity"
        );
        let actor = match &self.actor {
            Actor::Product { user_id } => user_id,
            Actor::Admin { account } => account,
        };
        ensure!(
            bounded(actor, 256) && bounded(&self.service_id, 128),
            "invalid uninstall owner"
        );
        ensure!(
            [&self.machine_id, &self.plugin_id].into_iter().all(|id| {
                bounded(id, 128)
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            }),
            "invalid uninstall target"
        );
        ensure!(
            self.plugin_version.len() <= 128
                && semver::Version::parse(&self.plugin_version).is_ok()
                && digest(&self.generation_digest)
                && digest(&self.contract_fingerprint),
            "invalid uninstall release"
        );
        ensure!(
            self.session_ids.len() <= 1024
                && self.session_ids.iter().all(|id| bounded(id, 128))
                && self.session_ids.windows(2).all(|ids| ids[0] < ids[1]),
            "uninstall session set must be bounded, sorted and unique"
        );
        for subset in [&self.active_session_ids, &self.live_session_ids] {
            ensure!(
                subset.len() <= self.session_ids.len()
                    && subset.windows(2).all(|ids| ids[0] < ids[1])
                    && subset
                        .iter()
                        .all(|id| self.session_ids.binary_search(id).is_ok()),
                "uninstall session subset is invalid"
            );
        }
        ensure!(
            self.expires_at_ms > 0
                && self.purge_after_ms > self.expires_at_ms
                && chrono::DateTime::from_timestamp_millis(self.purge_after_ms).is_some(),
            "invalid uninstall deadlines"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Prepared,
    StoppingSessions,
    Uninstalling,
    MachineUninstalled,
    RestoringMachine,
    RestoringSessions,
    Completed,
    Compensated,
    Aborted,
    NeedsAttention,
}

impl Phase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::StoppingSessions => "stopping_sessions",
            Self::Uninstalling => "uninstalling",
            Self::MachineUninstalled => "machine_uninstalled",
            Self::RestoringMachine => "restoring_machine",
            Self::RestoringSessions => "restoring_sessions",
            Self::Completed => "completed",
            Self::Compensated => "compensated",
            Self::Aborted => "aborted",
            Self::NeedsAttention => "needs_attention",
        }
    }

    pub const fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Compensated | Self::Aborted)
    }

    pub const fn permits(self, next: Self) -> bool {
        if !self.terminal()
            && !matches!(self, Self::NeedsAttention)
            && matches!(next, Self::NeedsAttention)
        {
            return true;
        }
        matches!(
            (self, next),
            (Self::Prepared, Self::StoppingSessions | Self::Aborted)
                | (Self::StoppingSessions, Self::Uninstalling)
                | (
                    Self::Uninstalling,
                    Self::MachineUninstalled | Self::RestoringMachine
                )
                | (
                    Self::MachineUninstalled,
                    Self::Completed | Self::RestoringMachine
                )
                | (Self::RestoringMachine, Self::RestoringSessions)
                | (Self::RestoringSessions, Self::Compensated)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Problem {
    Interrupted,
    PreconditionsChanged,
    MachineUnavailable,
    MachineRejected,
    UnknownMachineOutcome,
    StorageFailure,
    CompensationFailed,
    WorkerRecoveryUnverified,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Operation {
    pub intent: UninstallIntent,
    pub phase: Phase,
    pub problem: Option<Problem>,
    pub cause: Option<Problem>,
    pub attention_from: Option<Phase>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[cfg(test)]
pub(crate) fn fixture(id: &str) -> UninstallIntent {
    UninstallIntent {
        schema: 1,
        operation_id: format!("operation-{id:0>16}"),
        service_id: "service-test".to_owned(),
        actor: Actor::Product {
            user_id: "user-test".to_owned(),
        },
        machine_id: "hawk".to_owned(),
        plugin_id: "victoria".to_owned(),
        plugin_version: "1.1.0".to_owned(),
        generation_digest: format!("sha256:{}", "a".repeat(64)),
        contract_fingerprint: format!("sha256:{}", "b".repeat(64)),
        session_ids: Vec::new(),
        active_session_ids: Vec::new(),
        live_session_ids: Vec::new(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 300_000,
        purge_after_ms: chrono::Utc::now().timestamp_millis() + 259_200_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_cannot_resurrect_a_terminal_or_uncertain_operation() {
        for phase in [
            Phase::Completed,
            Phase::Compensated,
            Phase::Aborted,
            Phase::NeedsAttention,
        ] {
            for next in [
                Phase::Prepared,
                Phase::StoppingSessions,
                Phase::Uninstalling,
                Phase::RestoringMachine,
                Phase::Completed,
                Phase::NeedsAttention,
            ] {
                assert!(!phase.permits(next));
            }
        }
        assert!(!Phase::Prepared.permits(Phase::Completed));
        assert!(!Phase::Uninstalling.permits(Phase::Completed));
    }

    #[test]
    fn intent_rejects_unknown_fields_bad_identity_and_expanded_impact() {
        let valid = fixture("1");
        valid.validate().unwrap();
        let mut invalid = valid.clone();
        invalid.active_session_ids.push("unconfirmed".to_owned());
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.generation_digest = "sha256:missing".to_owned();
        assert!(invalid.validate().is_err());
        invalid = valid.clone();
        invalid.session_ids = vec!["duplicate".to_owned(); 2];
        assert!(invalid.validate().is_err());
        let mut value = serde_json::to_value(valid).unwrap();
        value["authorization"] = serde_json::json!(true);
        assert!(serde_json::from_value::<UninstallIntent>(value).is_err());
    }
    #[test]
    fn machine_step_binds_actor_impact_and_every_confirmed_input() {
        let intent = fixture("machine-binding");
        let expected = intent.machine_step().unwrap();
        assert_eq!(intent.machine_step().unwrap(), expected);
        let mut changed = intent.clone();
        changed.actor = Actor::Admin {
            account: "different".into(),
        };
        assert_ne!(
            changed.machine_step().unwrap().plan_digest,
            expected.plan_digest
        );
        let mut changed = intent;
        changed.purge_after_ms += 1;
        assert_ne!(
            changed.machine_step().unwrap().plan_digest,
            expected.plan_digest
        );
    }
}

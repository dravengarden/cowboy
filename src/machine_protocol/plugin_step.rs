//! Closed protocol-ten uninstall evidence. These values are not grants and
//! cannot restore authority by deserialization. No credentials or raw errors.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UninstallStep {
    pub schema: u16,
    pub operation_id: String,
    pub service_id: String,
    pub machine_id: String,
    /// Digest of the complete Service intent, including actor and impact.
    pub plan_digest: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    pub contract_fingerprint: String,
    pub expires_at_ms: i64,
}

fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

impl UninstallStep {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "unsupported Machine step schema");
        ensure!(
            id(&self.operation_id)
                && self.operation_id.len() >= 16
                && id(&self.machine_id)
                && id(&self.plugin_id),
            "invalid Machine step identity"
        );
        ensure!(
            !self.service_id.is_empty()
                && self.service_id.len() <= 128
                && !self.service_id.chars().any(char::is_control),
            "invalid Machine step owner"
        );
        ensure!(
            self.plugin_version.len() <= 128
                && semver::Version::parse(&self.plugin_version).is_ok(),
            "invalid Machine step version"
        );
        for value in [
            &self.plan_digest,
            &self.generation_digest,
            &self.contract_fingerprint,
        ] {
            ensure!(
                value.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64
                        && hex
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }),
                "invalid Machine step digest"
            );
        }
        ensure!(
            self.expires_at_ms > 0 && self.expires_at_ms <= 9_007_199_254_740_991,
            "invalid Machine step deadline"
        );
        Ok(())
    }

    pub(crate) fn request_digest(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(&serde_json::to_vec(self)?))
    }

    #[cfg(feature = "machine-host")]
    pub(crate) fn key(&self) -> Result<String> {
        self.validate()?;
        // Step identity is the closed `uninstall` primitive, not caller argv.
        Ok(digest(&serde_json::to_vec(&(
            &self.service_id,
            &self.operation_id,
            "uninstall",
        ))?)[7..]
            .to_owned())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepRejection {
    Expired,
    TargetChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepUncertainty {
    Interrupted,
    EffectFailure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepOutcome {
    Applied {},
    Rejected { reason: StepRejection },
    Unknown { reason: StepUncertainty },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepReceipt {
    pub step: UninstallStep,
    pub request_digest: String,
    pub outcome: StepOutcome,
}

impl StepReceipt {
    pub(crate) fn matches(&self, step: &UninstallStep) -> bool {
        &self.step == step
            && step
                .request_digest()
                .is_ok_and(|d| d == self.request_digest)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepUnavailable {
    ReaderOnly,
    WrongOwner,
    InvalidRequest,
    IdentityConflict,
    SlotFenced,
    Capacity,
    Storage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepLookup {
    NotFound {},
    Found { receipt: Box<StepReceipt> },
    Unavailable { reason: StepUnavailable },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepObservation {
    pub admission_enabled: bool,
    pub result: StepLookup,
}

#[cfg(test)]
pub(crate) fn fixture() -> UninstallStep {
    UninstallStep {
        schema: 1,
        operation_id: "fixture-operation-0001".into(),
        service_id: "service-test".into(),
        machine_id: "machine-test".into(),
        plan_digest: digest(b"plan"),
        plugin_id: "victoria".into(),
        plugin_version: "1.0.0".into(),
        generation_digest: digest(b"release"),
        contract_fingerprint: digest(b"contract"),
        expires_at_ms: 1_800_000_000_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::{MachineCommand, MachineEvent, PLUGIN_STEP_PROTOCOL_VERSION};

    #[test]
    fn wire_is_closed_typed_and_requires_protocol_ten() {
        let step = fixture();
        step.validate().unwrap();
        for command in [
            MachineCommand::QueryPluginUninstallStep {
                request_id: "query".into(),
                step: Box::new(step.clone()),
            },
            MachineCommand::UninstallPluginStep {
                request_id: "apply".into(),
                step: Box::new(step.clone()),
            },
        ] {
            assert_eq!(command.minimum_protocol(), PLUGIN_STEP_PROTOCOL_VERSION);
            assert_eq!(
                serde_json::from_slice::<MachineCommand>(&serde_json::to_vec(&command).unwrap())
                    .unwrap(),
                command
            );
        }
        let event = MachineEvent::PluginUninstallStep {
            request_id: "query".into(),
            observation: Box::new(StepObservation {
                admission_enabled: false,
                result: StepLookup::Found {
                    receipt: Box::new(StepReceipt {
                        step: step.clone(),
                        request_digest: step.request_digest().unwrap(),
                        outcome: StepOutcome::Applied {},
                    }),
                },
            }),
        };
        assert_eq!(
            serde_json::from_slice::<MachineEvent>(&serde_json::to_vec(&event).unwrap()).unwrap(),
            event
        );
        let mut invalid = serde_json::to_value(&step).unwrap();
        invalid["authorization"] = true.into();
        assert!(serde_json::from_value::<UninstallStep>(invalid).is_err());
        let duplicate = serde_json::to_string(&step)
            .unwrap()
            .replacen('{', "{\"schema\":1,", 1);
        assert!(serde_json::from_str::<UninstallStep>(&duplicate).is_err());
        assert!(serde_json::from_str::<StepOutcome>(r#"{"state":"applied","undo":true}"#).is_err());
        assert!(serde_json::from_str::<StepOutcome>(r#"{"state":"restored"}"#).is_err());
        assert!(
            serde_json::from_str::<StepLookup>(r#"{"state":"not_found","undo":true}"#).is_err()
        );
    }

    #[test]
    fn invalid_step_bounds_never_become_journal_keys() {
        let mut request = fixture();
        request.plugin_id = "../../victoria".into();
        assert!(request.validate().is_err());
        request = fixture();
        request.plan_digest = "sha256:ABC".into();
        assert!(request.validate().is_err());
        request = fixture();
        request.expires_at_ms = i64::MAX;
        assert!(request.validate().is_err());
    }
}

//! Closed Machine installation evidence. A target observation or saved attempt
//! is never execution authority, even when its digest still matches inventory.

use super::DesiredPlugin;
use super::installation_revision::InstallationRevision;
use super::plugin_step::digest;
use anyhow::{Result, ensure};
use cowboy_plugin_sdk::PluginKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallTarget {
    /// No active link AND no prior installation authority. Never inferred from
    /// an empty, failed, stale or untracked inventory response.
    Vacant {},
    Installed {
        revision: InstallationRevision,
        generation_digest: String,
    },
    Removed {
        revision: InstallationRevision,
    },
}

fn canonical_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

impl InstallTarget {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Self::Installed {
            generation_digest, ..
        } = self
        {
            ensure!(
                canonical_digest(generation_digest),
                "invalid install target"
            );
        }
        Ok(())
    }

    pub(crate) fn revision(&self) -> Option<&InstallationRevision> {
        match self {
            Self::Vacant {} => None,
            Self::Installed { revision, .. } | Self::Removed { revision } => Some(revision),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallTargetQuery {
    pub schema: u16,
    pub service_id: String,
    pub machine_id: String,
    pub plugin_id: String,
}

impl InstallTargetQuery {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && !self.service_id.is_empty()
                && self.service_id.len() <= 128
                && !self.service_id.chars().any(char::is_control)
                && id(&self.machine_id)
                && id(&self.plugin_id),
            "invalid installation owner or target"
        );
        Ok(())
    }

    pub(crate) fn digest(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(&serde_json::to_vec(self)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallStep {
    pub schema: u16,
    pub operation_id: String,
    pub service_id: String,
    pub machine_id: String,
    /// The complete Service intent, including original actor and target CAS.
    pub plan_digest: String,
    pub plugin_id: String,
    pub plugin_kind: PluginKind,
    pub plugin_version: String,
    pub generation_digest: String,
    pub contract_fingerprint: String,
    /// Complete `DesiredPlugin`, including signed artifacts and host bundle.
    /// The journal never retains package bytes, download URLs or credentials.
    pub envelope_digest: String,
    pub expected: InstallTarget,
    pub expires_at_ms: i64,
}

impl InstallStep {
    pub(crate) fn target_query(&self) -> InstallTargetQuery {
        InstallTargetQuery {
            schema: 1,
            service_id: self.service_id.clone(),
            machine_id: self.machine_id.clone(),
            plugin_id: self.plugin_id.clone(),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.target_query().validate()?;
        self.expected.validate()?;
        ensure!(
            self.schema == 1 && id(&self.operation_id) && self.operation_id.len() >= 16,
            "invalid installation operation"
        );
        ensure!(
            matches!(
                self.plugin_kind,
                PluginKind::AgentProvider
                    | PluginKind::CodeIntelligence
                    | PluginKind::TelemetryBackend
            ) && self.plugin_version.len() <= 128
                && semver::Version::parse(&self.plugin_version).is_ok(),
            "invalid installation capability or version"
        );
        for value in [
            &self.plan_digest,
            &self.generation_digest,
            &self.contract_fingerprint,
            &self.envelope_digest,
        ] {
            ensure!(canonical_digest(value), "invalid installation digest");
        }
        ensure!(
            self.expires_at_ms > 0 && self.expires_at_ms <= 9_007_199_254_740_991,
            "invalid installation deadline"
        );
        Ok(())
    }

    pub(crate) fn request_digest(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(&serde_json::to_vec(self)?))
    }

    pub(crate) fn matches_envelope(&self, desired: &DesiredPlugin) -> bool {
        self.validate().is_ok()
            && self.plugin_id == desired.release.plugin_id
            && self.plugin_kind == desired.release.plugin_kind
            && self.plugin_version == desired.release.plugin_version
            && self.generation_digest == desired.release.artifact_digest
            && self.contract_fingerprint == desired.release.contract_fingerprint
            && serde_json::to_vec(desired).is_ok_and(|bytes| digest(&bytes) == self.envelope_digest)
    }

    #[cfg(feature = "machine-host")]
    pub(crate) fn key(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(&serde_json::to_vec(&(
            &self.service_id,
            &self.operation_id,
            "install",
        ))?)[7..]
            .to_owned())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPhase {
    Prepared,
    Staging,
    Activating,
    ProjectingAuthentication,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallRejection {
    Expired,
    AuthorizationEnded,
    TargetChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallUncertainty {
    Interrupted,
    EffectFailure,
    AuthorizationEnded,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallOutcome {
    Pending {
        phase: InstallPhase,
    },
    Applied {
        revision: InstallationRevision,
    },
    /// Only valid before Staging. No publisher pin or Plugin effect was begun.
    Rejected {
        reason: InstallRejection,
    },
    Unknown {
        phase: InstallPhase,
        reason: InstallUncertainty,
    },
}

#[cfg(any(feature = "machine-host", test))]
impl InstallOutcome {
    pub(crate) fn fenced(&self) -> bool {
        matches!(self, Self::Pending { .. } | Self::Unknown { .. })
    }

    pub(crate) fn follows(&self, previous: &Self) -> bool {
        use InstallPhase::{Activating, Prepared, ProjectingAuthentication, Staging};
        match (previous, self) {
            (Self::Pending { phase: left }, Self::Unknown { phase: right, .. }) => left == right,
            (Self::Pending { phase: left }, Self::Pending { phase: right }) => matches!(
                (left, right),
                (Prepared, Staging)
                    | (Staging, Activating)
                    | (Activating, ProjectingAuthentication)
            ),
            (Self::Pending { phase: Prepared }, Self::Rejected { .. })
            | (
                Self::Pending {
                    phase: Activating | ProjectingAuthentication,
                },
                Self::Applied { .. },
            ) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallReceipt {
    pub step: InstallStep,
    pub request_digest: String,
    pub outcome: InstallOutcome,
}

impl InstallReceipt {
    pub(crate) fn matches(&self, step: &InstallStep) -> bool {
        &self.step == step
            && step
                .request_digest()
                .is_ok_and(|digest| digest == self.request_digest)
            && match &self.outcome {
                InstallOutcome::Applied { revision } => step.expected.revision() != Some(revision),
                InstallOutcome::Pending { phase } | InstallOutcome::Unknown { phase, .. } => {
                    *phase != InstallPhase::ProjectingAuthentication
                        || step.plugin_kind == PluginKind::AgentProvider
                }
                InstallOutcome::Rejected { .. } => true,
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallUnavailable {
    ReaderOnly,
    WrongOwner,
    InvalidRequest,
    IdentityConflict,
    SlotFenced,
    TargetChanged,
    Untracked,
    Capacity,
    Storage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallLookup {
    NotFound {},
    Found { receipt: Box<InstallReceipt> },
    Unavailable { reason: InstallUnavailable },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallObservation {
    pub admission_enabled: bool,
    pub result: InstallLookup,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallTargetObservation {
    Observed {
        query_digest: String,
        admission_enabled: bool,
        target: InstallTarget,
    },
    Unavailable {
        reason: InstallUnavailable,
    },
}

#[cfg(test)]
pub(crate) fn fixture() -> InstallStep {
    InstallStep {
        schema: 1,
        operation_id: "installation-fixture-0001".into(),
        service_id: "service-test".into(),
        machine_id: "machine-test".into(),
        plan_digest: digest(b"approved installation and target"),
        plugin_id: "victoria".into(),
        plugin_kind: PluginKind::TelemetryBackend,
        plugin_version: "1.0.0".into(),
        generation_digest: digest(b"release"),
        contract_fingerprint: digest(b"contract"),
        envelope_digest: digest(b"complete desired envelope"),
        expected: InstallTarget::Vacant {},
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 300_000,
    }
}

#[cfg(test)]
mod tests;

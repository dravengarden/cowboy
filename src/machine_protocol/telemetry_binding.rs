//! Durable binding evidence, never a serialized egress or recovery grant.
//! Protocol 14 is a reader bridge: it adds a query, not a mutation command.

use super::installation_revision::InstallationRevision;
use super::plugin_step::digest;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

macro_rules! counter {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(u64);

        impl TryFrom<String> for $name {
            type Error = &'static str;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                let parsed = value
                    .parse::<u64>()
                    .map_err(|_| "invalid binding counter")?;
                if value == parsed.to_string() {
                    Ok(Self(parsed))
                } else {
                    Err("noncanonical binding counter")
                }
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0.to_string()
            }
        }
    };
}

// Separate axes, both encoded as canonical decimal strings (including above
// JS's safe integer range). Zero is the unobserved/uninitialized baseline.
counter!(BindingRevision);
counter!(PolicyEpoch);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BindingDigest(String);

impl TryFrom<String> for BindingDigest {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            Ok(Self(value))
        } else {
            Err("invalid binding digest")
        }
    }
}

impl From<BindingDigest> for String {
    fn from(value: BindingDigest) -> Self {
        value.0
    }
}

pub(crate) fn binding_digest(bytes: &[u8]) -> BindingDigest {
    BindingDigest(digest(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingInstallation {
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: BindingDigest,
    pub installation_revision: InstallationRevision,
    pub contract_fingerprint: BindingDigest,
}

fn id(value: &str, minimum: usize) -> bool {
    (minimum..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub(crate) fn valid_service(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

impl BindingInstallation {
    fn validate(&self) -> Result<()> {
        ensure!(id(&self.plugin_id, 1), "invalid binding Plugin identity");
        ensure!(
            self.plugin_version.len() <= 128
                && semver::Version::parse(&self.plugin_version).is_ok(),
            "invalid binding Plugin version"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingSnapshot {
    pub revision: BindingRevision,
    /// This is a policy version, not a policy digest, secret, or grant.
    pub policy_epoch: PolicyEpoch,
    pub selection: Option<BindingInstallation>,
}

impl BindingSnapshot {
    pub(crate) const fn initial() -> Self {
        Self {
            revision: BindingRevision(0),
            policy_epoch: PolicyEpoch(0),
            selection: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.revision.0 == 0 {
            ensure!(self == &Self::initial(), "invalid initial binding snapshot");
        }
        if let Some(selection) = &self.selection {
            ensure!(self.policy_epoch.0 != 0, "binding policy was not observed");
            selection.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BindingChange {
    Select {
        installation: BindingInstallation,
        policy_epoch: PolicyEpoch,
    },
    Revoke {
        policy_epoch: PolicyEpoch,
    },
    /// A separately authorized CAS of managed binding configuration. This
    /// cannot undo HTTP emissions, restore credentials or lower a policy epoch.
    Restore {
        forward_request_digest: BindingDigest,
        selection: Option<BindingInstallation>,
        policy_epoch: PolicyEpoch,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingStep {
    pub schema: u16,
    pub operation_id: String,
    pub service_id: String,
    pub machine_id: String,
    /// Complete independently authorized Service intent, including its actor.
    pub plan_digest: BindingDigest,
    pub expected: BindingSnapshot,
    pub change: BindingChange,
    pub expires_at_ms: i64,
}

impl BindingStep {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "unsupported telemetry binding schema");
        ensure!(
            id(&self.operation_id, 16)
                && id(&self.machine_id, 1)
                && valid_service(&self.service_id),
            "invalid telemetry binding owner or identity"
        );
        ensure!(
            (1..=9_007_199_254_740_991).contains(&self.expires_at_ms),
            "invalid telemetry binding deadline"
        );
        self.expected.validate()?;
        self.after()?.validate()
    }

    pub(crate) fn after(&self) -> Result<BindingSnapshot> {
        let (selection, policy_epoch) = match &self.change {
            BindingChange::Select {
                installation,
                policy_epoch,
            } => (Some(installation.clone()), *policy_epoch),
            BindingChange::Revoke { policy_epoch } => (None, *policy_epoch),
            BindingChange::Restore {
                selection,
                policy_epoch,
                ..
            } => (selection.clone(), *policy_epoch),
        };
        ensure!(
            policy_epoch >= self.expected.policy_epoch,
            "binding cannot lower policy epoch"
        );
        let revision = self
            .expected
            .revision
            .0
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("binding revision exhausted"))?;
        Ok(BindingSnapshot {
            revision: BindingRevision(revision),
            policy_epoch,
            selection,
        })
    }

    pub(crate) fn request_digest(&self) -> Result<BindingDigest> {
        self.validate()?;
        Ok(binding_digest(&serde_json::to_vec(self)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingRejection {
    Expired,
    TargetChanged,
    PolicyChanged,
    AuthorizationEnded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum BindingOutcome {
    Prepared {},
    Applied { after: BindingSnapshot },
    Rejected { reason: BindingRejection },
    Unknown {},
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingReceipt {
    pub step: BindingStep,
    pub request_digest: BindingDigest,
    pub outcome: BindingOutcome,
}

impl BindingReceipt {
    pub(crate) fn matches(&self, step: &BindingStep) -> bool {
        self.step == *step
            && step
                .request_digest()
                .is_ok_and(|digest| digest == self.request_digest)
            && match &self.outcome {
                BindingOutcome::Applied { after } => {
                    step.after().is_ok_and(|expected| expected == *after)
                }
                _ => true,
            }
    }

    pub(crate) const fn unresolved(&self) -> bool {
        matches!(
            self.outcome,
            BindingOutcome::Prepared {} | BindingOutcome::Unknown {}
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingUnavailable {
    WrongOwner,
    InvalidRequest,
    IdentityConflict,
    Storage,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingObservationSnapshot {
    pub request_digest: BindingDigest,
    pub receipt: Option<Box<BindingReceipt>>,
    /// None means no managed namespace, NOT a proven revocation.
    pub current: Option<BindingSnapshot>,
    pub unresolved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum BindingObservation {
    Observed {
        snapshot: Box<BindingObservationSnapshot>,
    },
    Unavailable {
        reason: BindingUnavailable,
    },
}

impl BindingObservation {
    #[cfg(any(feature = "full", test))]
    pub(crate) fn matches(&self, step: &BindingStep) -> bool {
        let Ok(expected) = step.request_digest() else {
            return false;
        };
        let Self::Observed { snapshot } = self else {
            return true;
        };
        snapshot.request_digest == expected
            && snapshot
                .current
                .as_ref()
                .is_none_or(|current| current.validate().is_ok())
            && (snapshot.current.is_some() || (snapshot.receipt.is_none() && !snapshot.unresolved))
            && snapshot.receipt.as_ref().is_none_or(|receipt| {
                receipt.matches(step)
                    && (!receipt.unresolved() || snapshot.unresolved)
                    && (!receipt.unresolved() || snapshot.current.as_ref() == Some(&step.expected))
                    && match &receipt.outcome {
                        BindingOutcome::Applied { after } => {
                            snapshot.current.as_ref().is_some_and(|current| {
                                current.revision >= after.revision
                                    && current.policy_epoch >= after.policy_epoch
                                    && (current.revision != after.revision || current == after)
                            })
                        }
                        _ => true,
                    }
            })
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> BindingStep {
    BindingStep {
        schema: 1,
        operation_id: "binding-operation-0001".into(),
        service_id: "service-test".into(),
        machine_id: "machine-test".into(),
        plan_digest: binding_digest(b"approved fixture plan"),
        expected: BindingSnapshot::initial(),
        change: BindingChange::Select {
            installation: BindingInstallation {
                plugin_id: "victoria".into(),
                plugin_version: "1.1.0".into(),
                generation_digest: binding_digest(b"release"),
                installation_revision: format!("installation-{}", "a".repeat(64))
                    .try_into()
                    .unwrap(),
                contract_fingerprint: binding_digest(b"contract"),
            },
            policy_epoch: PolicyEpoch(1),
        },
        expires_at_ms: 1_900_000_000_000,
    }
}

#[cfg(test)]
mod tests;

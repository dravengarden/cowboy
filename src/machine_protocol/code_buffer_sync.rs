//! Closed core synchronization requests. These are declarations received over
//! the enrolled control connection, never serialized execution grants. Only
//! Machine core can pair one with its original, live connection and deadline.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BufferRef {
    pub(crate) instance: String,
    pub(crate) id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRef {
    pub(crate) instance: String,
    pub(crate) id: String,
}

#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
fn reference(instance: &str, id: &str) -> Result<()> {
    let hex = |value: &str, len| {
        value.len() == len
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    ensure!(
        hex(instance, 32) && hex(id, 16) && id != "0000000000000000",
        "invalid buffer reference"
    );
    Ok(())
}

#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
impl BufferRef {
    pub(crate) fn validate(&self) -> Result<()> {
        reference(&self.instance, &self.id)
    }
}

#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
impl OperationRef {
    pub(crate) fn validate(&self) -> Result<()> {
        reference(&self.instance, &self.id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Content {
    pub(crate) sha256: String,
    pub(crate) utf8_bytes: u32,
}

impl Content {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.utf8_bytes <= 4 * 1024 * 1024
                && self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "invalid buffer content identity"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    RefreshFromDisk,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Prepare {
        lease: BufferRef,
        purpose: Purpose,
        content: Content,
    },
    Apply {
        operation: OperationRef,
    },
    Query {
        operation: OperationRef,
    },
    Retire {
        operation: OperationRef,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub service_id: String,
    pub machine_id: String,
    pub action: Action,
}

#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
impl Request {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            crate::service_identity::valid_service_id(&self.service_id)
                && !self.machine_id.is_empty()
                && self.machine_id.len() <= 256
                && !self.machine_id.chars().any(char::is_control),
            "invalid synchronization Site"
        );
        match &self.action {
            Action::Prepare { lease, content, .. } => {
                lease.validate()?;
                content.validate()
            }
            Action::Apply { operation }
            | Action::Query { operation }
            | Action::Retire { operation } => operation.validate(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionEntry {
    pub(crate) replica_id: u16,
    pub(crate) timestamp: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Changed,
    Source,
    Shared,
    // Exact terminal native refusal, not a local timeout/capacity guess.
    Budget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum State {
    Prepared {},
    Pending {},
    Unknown {},
    Applied {
        content: Content,
        version: Vec<VersionEntry>,
    },
    Refused {
        reason: Reason,
    },
    Retired {},
}

#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
impl State {
    pub(crate) fn terminal(&self) -> bool {
        matches!(self, Self::Applied { .. } | Self::Refused { .. })
    }

    pub(crate) fn validate(&self, expected: &Content) -> Result<()> {
        if let Self::Applied { content, version } = self {
            content.validate()?;
            ensure!(content == expected, "synchronization content changed");
            ensure!(
                version.len() <= 256
                    && version.iter().all(|entry| entry.timestamp > 0)
                    && version
                        .windows(2)
                        .all(|pair| pair[0].replica_id < pair[1].replica_id),
                "invalid synchronization version"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub api_version: u8,
    pub operation: OperationRef,
    pub state: State,
}

#[cfg(test)]
mod tests;

//! Closed navigation declarations, not acquisition authority. Native references
//! stay Machine-private; only the enrolled connection can admit a continuation.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub use super::code_buffer_sync::{BufferRef, Content};

pub(crate) const MAX_LOCATIONS: usize = 256;
pub(crate) const MAX_TARGETS: usize = 32;

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NavigationRef {
    pub(crate) instance: String,
    pub(crate) id: String,
}

#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
impl NavigationRef {
    pub(crate) fn validate(&self) -> Result<()> {
        let id = self.id.strip_prefix("navigation:").unwrap_or("");
        BufferRef {
            instance: self.instance.clone(),
            id: id.to_owned(),
        }
        .validate()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub(crate) row: u32,
    pub(crate) column: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Prepare {
        lease: BufferRef,
        content: Content,
        position: Point,
        query: Kind,
    },
    Execute {
        navigation: NavigationRef,
    },
    Query {
        navigation: NavigationRef,
    },
    Release {
        navigation: NavigationRef,
    },
    PrepareDestination {
        navigation: NavigationRef,
        destination: u32,
        content: Content,
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
            "invalid navigation Site"
        );
        match &self.action {
            Action::Prepare {
                lease,
                content,
                position,
                ..
            } => {
                lease.validate()?;
                content.validate()?;
                ensure!(
                    position.row <= content.utf8_bytes && position.column <= content.utf8_bytes,
                    "navigation position exceeds content bounds"
                );
            }
            Action::Execute { navigation }
            | Action::Query { navigation }
            | Action::Release { navigation } => navigation.validate()?,
            Action::PrepareDestination {
                navigation,
                destination,
                content,
            } => {
                navigation.validate()?;
                content.validate()?;
                ensure!(
                    (*destination as usize) < MAX_LOCATIONS,
                    "invalid navigation destination"
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub(crate) path: String,
    pub(crate) content: Content,
    pub(crate) start: Point,
    pub(crate) end: Point,
}

#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
pub(crate) fn validate_locations(locations: &[Location]) -> Result<()> {
    ensure!(
        locations.len() <= MAX_LOCATIONS,
        "too many navigation locations"
    );
    let mut paths = BTreeMap::new();
    for location in locations {
        location.content.validate()?;
        ensure!(
            !location.path.is_empty()
                && location.path.len() <= 4096
                && !location.path.chars().any(char::is_control)
                && location
                    .path
                    .split('/')
                    .all(|part| !matches!(part, "" | "." | "..")),
            "invalid navigation display path"
        );
        ensure!(
            location.start <= location.end
                && [location.start, location.end]
                    .iter()
                    .all(|point| point.row <= location.content.utf8_bytes
                        && point.column <= location.content.utf8_bytes),
            "invalid navigation range"
        );
        if let Some(content) = paths.insert(&location.path, &location.content) {
            ensure!(
                content == &location.content,
                "navigation target identity changed"
            );
        }
    }
    ensure!(paths.len() <= MAX_TARGETS, "too many navigation targets");
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    Unknown,
    Retained,
    ReleaseUnknown,
    Released,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub(crate) destination: u32,
    pub(crate) lease: BufferRef,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub api_version: u8,
    pub navigation: NavigationRef,
    pub phase: Phase,
    /// Saved observations, never fresh text/coordinates or path lookup grants.
    pub locations: Vec<Location>,
    /// At most one effect-free ordinary reservation per original result index.
    /// Open/read/release use the existing owned-buffer continuation explicitly.
    pub destinations: Vec<Destination>,
}

#[cfg(test)]
mod tests;

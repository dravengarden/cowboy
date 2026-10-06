//! Retiring Agent Provider generations that no recoverable session pins.
//!
//! The Controller owns which generations sessions reference; the Machine owns
//! the files. They exchange one adapter request so older Machines simply refuse
//! an unknown adapter instead of needing a protocol bump.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Machine adapter that retires unreferenced generations of one Plugin.
pub const ADAPTER: &str = "plugin-generation-retention";

/// Bound on referenced digests carried in one request.
pub const MAX_REFERENCED: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub plugin_id: String,
    /// `sha256:<hex>` generations still pinned by a recoverable session.
    pub referenced: BTreeSet<String>,
}

impl Request {
    /// Reject anything but bounded, well-formed digests.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.referenced.len() <= MAX_REFERENCED
            && self.referenced.iter().all(|digest| {
                digest.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64
                        && hex
                            .bytes()
                            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                })
            })
    }
}

/// Machine adapter that retires generations left by uninstalled Plugins.
pub const UNINSTALLED_ADAPTER: &str = "plugin-generation-retention-uninstalled";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UninstalledRequest {
    /// Generations pinned per Plugin by sessions that can still launch,
    /// including soft-deleted sessions until they are purged.
    pub referenced: BTreeMap<String, BTreeSet<String>>,
    /// Plugins whose uninstall may still compensate by reactivation.
    pub skip: BTreeSet<String>,
}

impl UninstalledRequest {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.skip.len() <= MAX_REFERENCED
            && self.referenced.values().map(BTreeSet::len).sum::<usize>() <= MAX_REFERENCED
            && self.referenced.iter().all(|(plugin_id, referenced)| {
                Request {
                    plugin_id: plugin_id.clone(),
                    referenced: referenced.clone(),
                }
                .is_valid()
            })
    }
}

/// What one retention pass kept and removed for a Plugin.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub plugin_id: String,
    pub retired: Vec<String>,
    pub retained: Vec<String>,
    pub freed_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bounded_sha256_digests_are_accepted() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let request = |referenced: Vec<String>| Request {
            plugin_id: "claude-code".to_owned(),
            referenced: referenced.into_iter().collect(),
        };
        assert!(request(vec![digest.clone()]).is_valid());
        assert!(request(Vec::new()).is_valid());
        assert!(!request(vec!["a".repeat(64)]).is_valid());
        assert!(!request(vec![format!("sha256:{}", "A".repeat(64))]).is_valid());
        assert!(!request(vec!["sha256:../escape".to_owned()]).is_valid());
    }
}

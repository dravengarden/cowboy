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

/// Stable phrase a Machine puts in a launch rejection when the session's
/// pinned generation has no directory any more (retired, or never installed
/// there). The Controller recognizes it and moves the session to the
/// installed release instead of failing every later open the same way.
pub const NOT_INSTALLED_MARKER: &str = "is not installed on this Machine";

/// Rejection detail for a session pinned to a generation this Machine lacks.
#[must_use]
pub fn not_installed_detail(plugin_id: &str, digest: &str) -> String {
    format!("Provider {plugin_id} generation {digest} {NOT_INSTALLED_MARKER}")
}

/// Whether a launch rejection means the pinned generation is gone from the
/// Machine. Machines before [`NOT_INSTALLED_MARKER`] surface the generation
/// reader's I/O context instead; its path form is matched for them.
#[must_use]
pub fn is_not_installed_detail(detail: &str) -> bool {
    detail.contains(NOT_INSTALLED_MARKER)
        || [
            "reading legacy Provider generation ",
            "reading Provider generation ",
        ]
        .iter()
        .any(|phrase| {
            // Also behind "fallback after generation launch failed: ".
            detail
                .split_once(phrase)
                .is_some_and(|(_, path)| path.contains("/generations/"))
        })
}

/// Machine adapter that retires the default account's Provider credential
/// generations (`materialized/` and `runtime/`) that no session pins. Each
/// Service credential refresh creates one of each, and a session keeps the
/// generation it launched with for its whole life, including resumes.
pub const AUTH_ADAPTER: &str = "provider-auth-generation-retention";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthRequest {
    pub provider_id: String,
    /// Auth generations still pinned by a session that can launch on this
    /// Machine, including soft-deleted sessions until they are purged.
    pub referenced: BTreeSet<u64>,
}

impl AuthRequest {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.referenced.len() <= MAX_REFERENCED && !self.referenced.contains(&0)
    }
}

/// What one credential retention pass kept and removed for a Provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthOutcome {
    pub provider_id: String,
    pub retired: Vec<u64>,
    pub retained: Vec<u64>,
    pub freed_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_requests_bound_and_reject_generation_zero() {
        let request = |referenced: Vec<u64>| AuthRequest {
            provider_id: "grok".to_owned(),
            referenced: referenced.into_iter().collect(),
        };
        assert!(request(vec![1, 7]).is_valid());
        assert!(request(Vec::new()).is_valid());
        assert!(!request(vec![0, 3]).is_valid());
        let too_many: Vec<u64> = (1..=u64::try_from(MAX_REFERENCED).unwrap() + 1).collect();
        assert!(!request(too_many).is_valid());
    }

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

    #[test]
    fn a_missing_generation_is_recognized_from_current_and_older_machines() {
        let digest = format!("sha256:{}", "a".repeat(64));
        assert!(is_not_installed_detail(&not_installed_detail(
            "claude-code",
            &digest
        )));
        assert!(is_not_installed_detail(&format!(
            "reading legacy Provider generation /state/plugins/claude-code/generations/{}/content/package.cowboy-provider",
            "a".repeat(64)
        )));
        assert!(is_not_installed_detail(&format!(
            "reading Provider generation /state/plugins/codex/generations/{}/content/package.cowboy-provider",
            "b".repeat(64)
        )));
        assert!(is_not_installed_detail(&format!(
            "fallback after generation launch failed: reading legacy Provider generation /state/plugins/codex/generations/{}/content/package.cowboy-provider",
            "c".repeat(64)
        )));
        // Other launch failures keep their session on its generation.
        assert!(!is_not_installed_detail("worker exited before readiness"));
        assert!(!is_not_installed_detail(
            "reading Provider generation manifest"
        ));
    }
}

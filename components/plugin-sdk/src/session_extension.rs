//! Data-only session extensions: memory backends and managed-call profiles.
//! Endpoints, credentials and project mappings stay Machine-private; a signed
//! package only names a closed protocol and bounded, public declarations.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use cowboy_provider_sdk::PlatformTarget;
use serde::{Deserialize, Serialize};

pub const MEMORY_BACKEND_SCHEMA_VERSION: u16 = 1;
pub const CALL_PROFILE_SCHEMA_VERSION: u16 = 1;
/// Plugin SDK release that introduced both session extension kinds.
pub const SESSION_EXTENSION_SDK: (u64, u64, u64) = (1, 12, 0);
const MAX_TOOLS: usize = 16;
const MAX_PURPOSES: usize = 16;
const MAX_GUIDANCE_BYTES: usize = 4096;

/// A memory service a Provider bridge reaches through `cowboy.memory-client`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryBackendContract {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub supported_platforms: Vec<PlatformTarget>,
    pub protocol: MemoryProtocol,
    /// Exact MCP tool names a Provider may expose for this backend.
    pub tools: Vec<String>,
    /// The backend answers bounded context before each user turn.
    pub recall: bool,
    /// The backend accepts completed public turn events.
    pub observe: bool,
}

/// Closed set of drivers implemented by `cowboy.memory-client`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProtocol {
    MatrixHttpV1,
}

/// Purposes and agent-facing guidance for Cowboy managed agent calls. The
/// call mechanism, grants and sandbox remain core; a profile only names what
/// calls are for and tells the agent when to use them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallProfileContract {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub supported_platforms: Vec<PlatformTarget>,
    pub purposes: Vec<CallPurpose>,
    pub guidance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallPurpose {
    pub id: String,
    pub label: String,
}

/// Placeholders a Provider fills when it renders call guidance: the display
/// name of the agent family it calls and that family's `cowboy` subcommand.
pub const CALL_GUIDANCE_PLACEHOLDERS: [&str; 2] = ["{{peer}}", "{{peer_command}}"];

fn guidance_placeholders_known(guidance: &str) -> bool {
    let mut rest = guidance;
    while let Some(start) = rest.find("{{") {
        let tail = &rest[start..];
        let Some(end) = tail.find("}}") else {
            return false;
        };
        if !CALL_GUIDANCE_PLACEHOLDERS.contains(&&tail[..end + 2]) {
            return false;
        }
        rest = &tail[end + 2..];
    }
    true
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn label(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn platforms(targets: &[PlatformTarget]) -> bool {
    !targets.is_empty()
        && targets.len() <= 4
        && targets.iter().collect::<BTreeSet<_>>().len() == targets.len()
}

impl MemoryBackendContract {
    /// Validate the closed, bounded, executable-free memory contract.
    ///
    /// # Errors
    /// Rejects unsupported schemas, malformed text, platforms or tool names.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == MEMORY_BACKEND_SCHEMA_VERSION,
            "unsupported memory backend schema"
        );
        ensure!(
            label(&self.display_name, 128) && label(&self.description, 512),
            "invalid memory backend text"
        );
        ensure!(
            platforms(&self.supported_platforms),
            "invalid memory backend platform matrix"
        );
        ensure!(
            self.tools.len() <= MAX_TOOLS
                && self.tools.iter().all(|tool| identifier(tool))
                && self.tools.iter().collect::<BTreeSet<_>>().len() == self.tools.len(),
            "invalid memory backend tools"
        );
        ensure!(
            self.recall || self.observe || !self.tools.is_empty(),
            "memory backend offers nothing"
        );
        Ok(())
    }
}

impl CallProfileContract {
    /// Validate the bounded purposes and guidance of a call profile.
    ///
    /// # Errors
    /// Rejects unsupported schemas, duplicate or malformed purposes and
    /// oversized guidance.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == CALL_PROFILE_SCHEMA_VERSION,
            "unsupported call profile schema"
        );
        ensure!(
            label(&self.display_name, 128) && label(&self.description, 512),
            "invalid call profile text"
        );
        ensure!(
            platforms(&self.supported_platforms),
            "invalid call profile platform matrix"
        );
        ensure!(
            !self.purposes.is_empty()
                && self.purposes.len() <= MAX_PURPOSES
                && self
                    .purposes
                    .iter()
                    .all(|purpose| identifier(&purpose.id) && label(&purpose.label, 64))
                && self
                    .purposes
                    .iter()
                    .map(|purpose| &purpose.id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    == self.purposes.len(),
            "invalid call profile purposes"
        );
        ensure!(
            guidance_placeholders_known(&self.guidance),
            "call profile guidance uses an unknown placeholder"
        );
        ensure!(
            !self.guidance.trim().is_empty()
                && self.guidance.len() <= MAX_GUIDANCE_BYTES
                && !self.guidance.chars().any(|c| c.is_control() && c != '\n'),
            "invalid call profile guidance"
        );
        Ok(())
    }
}

/// A data-only extension release carries one empty artifact entry per
/// supported platform and nothing executable.
pub(crate) fn validate_data_only_release(
    supported_platforms: &[PlatformTarget],
    release: &crate::PluginRelease,
    kind: &str,
) -> Result<()> {
    ensure!(
        release.host_bundle_digest.is_none(),
        "{kind} cannot contain host code"
    );
    ensure!(
        release.runtime_artifacts.len() == release.supported_platforms.len(),
        "{kind} runtime matrix mismatch"
    );
    let targets = release
        .runtime_artifacts
        .iter()
        .map(|entry| PlatformTarget {
            os: entry.os.clone(),
            architecture: entry.architecture.clone(),
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        targets == supported_platforms.iter().cloned().collect::<BTreeSet<_>>()
            && release
                .runtime_artifacts
                .iter()
                .all(|entry| entry.components.is_empty()),
        "{kind} cannot contain executable artifacts"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn memory() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1, "id": "matrix", "version": "1.0.0",
            "display_name": "Matrix", "description": "Shared memory",
            "supported_platforms": [{"os": "linux", "architecture": "x86_64"}],
            "protocol": "matrix_http_v1",
            "tools": ["memory_search", "memory_get"], "recall": true, "observe": true
        })
    }

    pub(crate) fn profile() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1, "id": "review", "version": "1.0.0",
            "display_name": "Review", "description": "Read-only reviews",
            "supported_platforms": [{"os": "linux", "architecture": "x86_64"}],
            "purposes": [{"id": "review", "label": "Code review"}],
            "guidance": "Ask {{peer}} for a read-only review with `cowboy {{peer_command}}`."
        })
    }

    fn manifest(kind: &str, sdk: &str) -> crate::PluginManifest {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": if kind == "memory_backend" { "matrix" } else { "review" },
            "version": "1.0.0", "component_release": "3.53.0", "publisher": "fixture",
            "kind": kind, "entrypoint": "contract.json",
            "components": [{"id": "cowboy.plugin-contract", "version": "1.11.0"},
                           {"id": "cowboy.plugin-sdk", "version": sdk}]
        }))
        .unwrap()
    }

    #[test]
    fn memory_contract_rejects_secrets_urls_and_bad_tools() {
        let valid = memory();
        serde_json::from_value::<MemoryBackendContract>(valid.clone())
            .unwrap()
            .validate()
            .unwrap();
        for (field, value) in [
            ("endpoint", serde_json::json!("https://matrix.example")),
            ("token", serde_json::json!("secret")),
            ("protocol", serde_json::json!("http")),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(serde_json::from_value::<MemoryBackendContract>(bad).is_err());
        }
        for tools in [
            serde_json::json!(["Memory-Search"]),
            serde_json::json!(["memory_get", "memory_get"]),
            serde_json::json!((0..17).map(|i| format!("tool_{i}")).collect::<Vec<_>>()),
        ] {
            let mut bad = valid.clone();
            bad["tools"] = tools;
            assert!(
                serde_json::from_value::<MemoryBackendContract>(bad)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut empty = valid;
        empty["tools"] = serde_json::json!([]);
        empty["recall"] = false.into();
        empty["observe"] = false.into();
        assert!(
            serde_json::from_value::<MemoryBackendContract>(empty)
                .unwrap()
                .validate()
                .is_err()
        );
    }

    #[test]
    fn call_profile_bounds_purposes_and_guidance() {
        let valid = profile();
        serde_json::from_value::<CallProfileContract>(valid.clone())
            .unwrap()
            .validate()
            .unwrap();
        for (field, value) in [
            ("purposes", serde_json::json!([])),
            (
                "purposes",
                serde_json::json!([{"id": "review", "label": "A"}, {"id": "review", "label": "B"}]),
            ),
            (
                "purposes",
                serde_json::json!([{"id": "Review!", "label": "A"}]),
            ),
            ("guidance", serde_json::json!("x".repeat(4097))),
            ("guidance", serde_json::json!("bell\u{7}")),
            (
                "guidance",
                serde_json::json!("Ask {{secret}} for a review."),
            ),
            ("guidance", serde_json::json!("Ask {{peer for a review.")),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(
                serde_json::from_value::<CallProfileContract>(bad)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut command = valid;
        command["command"] = "codex exec".into();
        assert!(serde_json::from_value::<CallProfileContract>(command).is_err());
    }

    #[test]
    fn session_extensions_are_data_only_plugins_requiring_sdk_one_twelve() {
        for (kind, payload) in [
            (
                "memory_backend",
                crate::PluginPayload::MemoryBackend(serde_json::from_value(memory()).unwrap()),
            ),
            (
                "call_profile",
                crate::PluginPayload::CallProfile(serde_json::from_value(profile()).unwrap()),
            ),
        ] {
            let old = manifest(kind, "1.11.1");
            assert!(
                crate::PluginPackage::new(
                    old.clone(),
                    old.component_release.clone(),
                    payload.clone()
                )
                .is_err()
            );
            let current = manifest(kind, "1.12.0");
            let package = crate::PluginPackage::new(
                current.clone(),
                current.component_release.clone(),
                payload,
            )
            .unwrap();
            assert!(package.agent_provider().is_none());
            assert!(package.authentication_provider().is_none());
            // Pre-existing Catalog readers stop at release schema 4.
            assert_eq!(
                package.minimum_release_schema(),
                crate::SESSION_EXTENSION_RELEASE_SCHEMA_VERSION
            );
            assert!(package.validate_host_contract(None).is_ok());
            assert!(
                package
                    .validate_host_contract(Some(&std::collections::BTreeMap::new()))
                    .is_err()
            );
            let requirements = crate::PluginCompatibilityRequirements {
                plugin_sdk_version: Some("1.12.0".into()),
                manifest_schema: 1,
                package_schema: 1,
                release_schema: crate::SESSION_EXTENSION_RELEASE_SCHEMA_VERSION,
                plugin_kind: current.kind,
                payload_schema: 1,
                host_bundle_schema: None,
                host_schema: None,
            };
            let target: PlatformTarget =
                serde_json::from_value(serde_json::json!({"os":"linux","architecture":"x86_64"}))
                    .unwrap();
            let mut inventory = crate::PluginContractInventory::current_machine(1);
            assert!(
                inventory
                    .compatibility_problem(
                        &requirements,
                        &current.id,
                        "1.0.0",
                        std::slice::from_ref(&target),
                        &target
                    )
                    .is_none()
            );
            inventory.plugin_sdk_version = "1.11.1".into();
            assert!(
                inventory
                    .compatibility_problem(
                        &requirements,
                        &current.id,
                        "1.0.0",
                        std::slice::from_ref(&target),
                        &target
                    )
                    .is_some()
            );
        }
    }
}

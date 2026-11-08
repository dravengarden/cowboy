//! Signed MCP servers use the existing private artifact lifecycle. Installation
//! declares executable identity; a separate session binding grants access.
use crate::{PluginComponentKind, PluginRelease};
use anyhow::{Result, ensure};
use cowboy_provider_sdk::PlatformTarget;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MCP_SERVER_SCHEMA: u16 = 1;
pub const MCP_SERVER_RELEASE_SCHEMA: u16 = 6;
pub const MCP_SERVER_SDK: (u64, u64, u64) = (1, 13, 0);

/// An exact installed generation, not permission to start or call its server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerSelection {
    pub plugin_id: String,
    pub generation_digest: String,
}

impl McpServerSelection {
    /// # Errors
    /// Rejects paths, aliases and non-canonical generation digests.
    pub fn validate(&self) -> Result<()> {
        ensure!(self.plugin_id.len() <= 128, "MCP Plugin id too long");
        crate::validate_id(&self.plugin_id, "MCP Plugin id")?;
        crate::validate_digest(&self.generation_digest, "MCP generation digest")?;
        ensure!(
            self.generation_digest
                .bytes()
                .all(|byte| !byte.is_ascii_uppercase()),
            "MCP generation digest must be canonical"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerContract {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub supported_platforms: Vec<PlatformTarget>,
    pub protocol: McpServerProtocol,
    pub command: String,
    pub arguments: Vec<String>,
    pub tools: Vec<String>,
    /// Named external sources the server may read when a binding grants them.
    /// Names only; credentials stay with the execution Machine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    /// Optional for transport-only servers; native startup requires this metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<Vec<crate::McpToolDefinition>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerProtocol {
    StdioSessionV1,
}

impl McpServerContract {
    /// Validate a bounded declaration; it contains no endpoints or credentials.
    /// # Errors
    /// Rejects unsupported schemas, duplicate tools or unsafe command declarations.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == MCP_SERVER_SCHEMA,
            "unsupported MCP server schema"
        );
        crate::validate_id(&self.id, "MCP server id")?;
        crate::validate_version(&self.version, "MCP server version")?;
        crate::validate_id(&self.command, "MCP server command")?;
        ensure!(
            !self.display_name.trim().is_empty()
                && self.display_name.len() <= 128
                && !self.display_name.chars().any(char::is_control),
            "invalid MCP display name"
        );
        ensure!(
            !self.description.trim().is_empty()
                && self.description.len() <= 512
                && !self.description.chars().any(char::is_control),
            "invalid MCP description"
        );
        ensure!(
            (1..=4).contains(&self.supported_platforms.len())
                && self
                    .supported_platforms
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == self.supported_platforms.len(),
            "invalid MCP platforms"
        );
        ensure!(
            self.arguments.len() <= 32
                && self
                    .arguments
                    .iter()
                    .all(|v| v.len() <= 1024 && !v.contains('\0')),
            "invalid MCP arguments"
        );
        ensure!(
            (1..=16).contains(&self.tools.len())
                && self.tools.iter().collect::<BTreeSet<_>>().len() == self.tools.len(),
            "invalid MCP tools"
        );
        if let Some(discovery) = &self.discovery {
            crate::mcp_discovery::validate_mcp_discovery(discovery, &self.tools)?;
        }
        ensure!(
            self.sources.len() <= 8
                && self.sources.iter().collect::<BTreeSet<_>>().len() == self.sources.len(),
            "invalid MCP sources"
        );
        for name in self.tools.iter().chain(&self.sources) {
            ensure!(
                !name.is_empty()
                    && name.len() <= 64
                    && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid MCP tool or source name"
            );
        }
        Ok(())
    }
}

pub(crate) fn validate_manifest(
    manifest: &crate::PluginManifest,
    contract: &McpServerContract,
) -> Result<()> {
    contract.validate()?;
    ensure!(
        contract.id == manifest.id && contract.version == manifest.version,
        "MCP payload identity mismatch"
    );
    ensure!(
        manifest
            .components
            .iter()
            .any(|component| component.id == "cowboy.plugin-sdk"
                && semver::Version::parse(&component.version).is_ok_and(|version| version
                    >= semver::Version::new(MCP_SERVER_SDK.0, MCP_SERVER_SDK.1, MCP_SERVER_SDK.2))),
        "MCP servers require Plugin SDK 1.13"
    );
    Ok(())
}

pub(crate) fn validate_release(
    contract: &McpServerContract,
    release: &PluginRelease,
) -> Result<()> {
    let mut targets = BTreeSet::new();
    for target in &release.runtime_artifacts {
        ensure!(
            targets.insert(PlatformTarget {
                os: target.os.clone(),
                architecture: target.architecture.clone()
            }),
            "duplicate MCP target"
        );
        ensure!(
            target.components.len() == 1,
            "MCP requires one self-contained server artifact"
        );
        let artifact = &target.components[0];
        ensure!(
            artifact.kind == PluginComponentKind::McpServer
                && artifact.command == contract.command
                && artifact.version == contract.version
                && artifact.dependency == contract.id,
            "MCP artifact identity mismatch"
        );
        crate::code_intelligence::validate_runtime_artifact(artifact)?;
    }
    ensure!(
        targets == contract.supported_platforms.iter().cloned().collect(),
        "incomplete MCP target matrix"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"id":"fixture-mcp","version":"1.0.0","display_name":"Fixture","description":"Fixture MCP server","supported_platforms":[{"os":"linux","architecture":"x86_64"}],"protocol":"stdio_session_v1","command":"fixture-mcp","arguments":["serve"],"tools":["fixture_read"]})
    }
    #[test]
    fn selection_requires_canonical_exact_identity() {
        let selection = McpServerSelection {
            plugin_id: "fixture-mcp".into(),
            generation_digest: format!("sha256:{}", "a".repeat(64)),
        };
        selection.validate().unwrap();
        for digest in [
            "latest".to_owned(),
            format!("sha256:{}", "A".repeat(64)),
            "sha256:a".to_owned(),
        ] {
            assert!(
                McpServerSelection {
                    generation_digest: digest,
                    ..selection.clone()
                }
                .validate()
                .is_err()
            );
        }
        for id in ["../server".to_owned(), "x".repeat(129)] {
            assert!(
                McpServerSelection {
                    plugin_id: id,
                    ..selection.clone()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_open_ended_launch_configuration() {
        let good = fixture();
        serde_json::from_value::<McpServerContract>(good.clone())
            .unwrap()
            .validate()
            .unwrap();
        for key in ["env", "url", "credentials", "shell"] {
            let mut value = good.clone();
            value[key] = serde_json::json!("not admitted");
            assert!(serde_json::from_value::<McpServerContract>(value).is_err());
        }
        for (key, value) in [
            ("command", serde_json::json!("/tmp/arbitrary")),
            ("tools", serde_json::json!(["same", "same"])),
            ("arguments", serde_json::json!(["a\u{0}b"])),
        ] {
            let mut bad = good.clone();
            bad[key] = value;
            assert!(
                serde_json::from_value::<McpServerContract>(bad)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
}

impl McpServerContract {
    /// Project startup metadata onto an explicitly selected signed tool subset.
    /// # Errors
    /// Refuses missing metadata, unknown/duplicate tools and invalid contracts.
    pub fn discovery_for(&self, tools: &[String]) -> Result<Vec<crate::McpToolDefinition>> {
        self.validate()?;
        let definitions = self
            .discovery
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MCP server has no signed startup discovery"))?;
        let selected: BTreeSet<_> = tools.iter().collect();
        ensure!(
            !selected.is_empty()
                && selected.len() == tools.len()
                && selected.iter().all(|name| self.tools.contains(name)),
            "invalid MCP discovery selection"
        );
        Ok(definitions
            .iter()
            .filter(|tool| selected.contains(&tool.name))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use serde_json::json;

    fn contract() -> McpServerContract {
        serde_json::from_value(json!({"schema_version":1,"id":"fixture-mcp","version":"1.0.0","display_name":"Fixture","description":"Fixture MCP server","supported_platforms":[{"os":"linux","architecture":"x86_64"}],"protocol":"stdio_session_v1","command":"fixture-mcp","arguments":["serve"],"tools":["fixture_read","fixture_status"],"discovery":[
            {"name":"fixture_read","description":"Read fixture","inputSchema":{"type":"object","properties":{"code":{"type":"string"}}}},
            {"name":"fixture_status","description":"Get status","inputSchema":{"type":"object","properties":{}}}
        ]})).unwrap()
    }

    #[test]
    fn startup_discovery_is_exact_signed_subset_without_launch_fields() {
        let contract = contract();
        let selected = contract.discovery_for(&["fixture_read".into()]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "fixture_read");
        assert!(contract.discovery_for(&["unsigned".into()]).is_err());
        assert!(
            contract
                .discovery_for(&["fixture_read".into(), "fixture_read".into()])
                .is_err()
        );
        let mut absent = contract.clone();
        absent.discovery = None;
        absent.validate().unwrap();
        assert!(absent.discovery_for(&["fixture_read".into()]).is_err());
        let mut extra = serde_json::to_value(&selected[0]).unwrap();
        extra["command"] = json!("arbitrary");
        assert!(serde_json::from_value::<crate::McpToolDefinition>(extra).is_err());
    }

    #[test]
    fn startup_discovery_rejects_missing_oversized_or_remote_schemas() {
        let mut missing = contract();
        missing.discovery.as_mut().unwrap().pop();
        assert!(missing.validate().is_err());
        for schema in [
            json!({"type":"array"}),
            json!({"type":"object","$ref":"https://fixture.invalid/schema"}),
            json!({"type":"object","$id":"https://fixture.invalid/"}),
            json!({"type":"object","description":"x".repeat(32768)}),
        ] {
            let mut invalid = contract();
            invalid.discovery.as_mut().unwrap()[0].input_schema = schema;
            assert!(invalid.validate().is_err());
        }
    }
}

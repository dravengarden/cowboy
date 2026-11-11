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
    /// MCP Apps declarations (docs/widget-apps-mcp.md §3): signed static views
    /// and per-tool visibility. Absent means every tool is visible to the
    /// agent only and the server has no views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<McpAppsContract>,
}

/// Signed MCP Apps metadata. It names which bytes may ever run as a view and
/// who may call each tool; it grants nothing by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpAppsContract {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui_resources: Vec<McpUiResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<McpAppTool>,
}

/// One static view: a file of the release's signed host bundle and its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpUiResource {
    pub uri: String,
    pub file: String,
    pub sha256: String,
}

/// Visibility and view linkage of one signed tool. Tools without an entry are
/// visible to the agent and to views (the MCP Apps default), are not
/// read-only and have no view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpAppTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub visibility: Vec<McpToolVisibility>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpToolVisibility {
    Model,
    App,
}

pub const MAX_MCP_UI_RESOURCES: usize = 8;

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
        if let Some(apps) = &self.apps {
            apps.validate(&self.tools)?;
        }
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

impl McpServerContract {
    fn app_tool(&self, name: &str) -> Option<&McpAppTool> {
        self.apps
            .as_ref()
            .and_then(|apps| apps.tools.iter().find(|tool| tool.name == name))
    }

    /// Whether the agent may list and call `name` (MCP Apps `"model"`).
    #[must_use]
    pub fn model_visible(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool == name)
            && self
                .app_tool(name)
                .is_none_or(|tool| tool.visibility.contains(&McpToolVisibility::Model))
    }

    /// Whether a view may call `name` (MCP Apps `"app"`).
    #[must_use]
    pub fn app_visible(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool == name)
            && self
                .app_tool(name)
                .is_none_or(|tool| tool.visibility.contains(&McpToolVisibility::App))
    }

    /// The signed `readOnlyHint`; unannotated tools are not read-only.
    #[must_use]
    pub fn read_only(&self, name: &str) -> bool {
        self.app_tool(name).is_some_and(|tool| tool.read_only)
    }

    /// The signed view a tool's results are shown in, if any.
    #[must_use]
    pub fn ui_resource_for(&self, name: &str) -> Option<&McpUiResource> {
        let uri = self.app_tool(name)?.resource.as_deref()?;
        self.apps
            .as_ref()?
            .ui_resources
            .iter()
            .find(|resource| resource.uri == uri)
    }
}

impl McpAppsContract {
    /// # Errors
    /// Rejects unknown tools, duplicate or malformed resources and dangling links.
    pub fn validate(&self, signed_tools: &[String]) -> Result<()> {
        ensure!(
            self.ui_resources.len() <= MAX_MCP_UI_RESOURCES,
            "too many MCP UI resources"
        );
        let mut uris = BTreeSet::new();
        let mut files = BTreeSet::new();
        for resource in &self.ui_resources {
            validate_ui_uri(&resource.uri)?;
            validate_ui_file(&resource.file)?;
            crate::validate_digest(&resource.sha256, "MCP UI resource digest")?;
            ensure!(
                resource
                    .sha256
                    .bytes()
                    .all(|byte| !byte.is_ascii_uppercase()),
                "MCP UI resource digest must be canonical"
            );
            ensure!(
                uris.insert(resource.uri.as_str()) && files.insert(resource.file.as_str()),
                "duplicate MCP UI resource"
            );
        }
        let mut names = BTreeSet::new();
        for tool in &self.tools {
            ensure!(
                signed_tools.contains(&tool.name) && names.insert(tool.name.as_str()),
                "MCP Apps tool is unsigned or duplicated"
            );
            ensure!(
                !tool.visibility.is_empty()
                    && tool.visibility.iter().collect::<BTreeSet<_>>().len()
                        == tool.visibility.len(),
                "invalid MCP tool visibility"
            );
            if let Some(uri) = &tool.resource {
                ensure!(
                    uris.contains(uri.as_str()),
                    "MCP tool names an undeclared UI resource"
                );
            }
        }
        ensure!(
            !self.ui_resources.is_empty() || !self.tools.is_empty(),
            "empty MCP Apps declaration"
        );
        Ok(())
    }
}

fn validate_ui_uri(uri: &str) -> Result<()> {
    let path = uri.strip_prefix("ui://").unwrap_or_default();
    ensure!(
        !path.is_empty()
            && uri.len() <= 256
            && path.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._/".contains(&byte)
            })
            && path
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != ".."),
        "invalid MCP UI resource URI"
    );
    Ok(())
}

fn validate_ui_file(file: &str) -> Result<()> {
    ensure!(
        file.len() <= 128
            && std::path::Path::new(file)
                .extension()
                .is_some_and(|extension| extension == "html")
            && file
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._/".contains(&byte))
            && file
                .split('/')
                .all(|segment| !segment.is_empty() && segment != "." && segment != ".."),
        "invalid MCP UI resource file"
    );
    Ok(())
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

    fn with_apps(apps: serde_json::Value) -> Result<McpServerContract> {
        let mut value = serde_json::to_value(contract())?;
        value["apps"] = apps;
        let contract: McpServerContract = serde_json::from_value(value)?;
        contract.validate()?;
        Ok(contract)
    }

    fn digest() -> String {
        format!("sha256:{}", "b".repeat(64))
    }

    #[test]
    fn apps_declare_signed_views_and_visibility() {
        let contract = with_apps(json!({
            "ui_resources":[{"uri":"ui://fixture/status","file":"ui/status.html","sha256":digest()}],
            "tools":[
                {"name":"fixture_status","resource":"ui://fixture/status","visibility":["model","app"],"read_only":true},
                {"name":"fixture_read","visibility":["app"]}
            ]
        }))
        .unwrap();
        assert!(contract.model_visible("fixture_status") && contract.app_visible("fixture_status"));
        assert!(!contract.model_visible("fixture_read") && contract.app_visible("fixture_read"));
        assert!(!contract.model_visible("unsigned") && !contract.app_visible("unsigned"));
        assert!(contract.read_only("fixture_status") && !contract.read_only("fixture_read"));
        assert_eq!(
            contract
                .ui_resource_for("fixture_status")
                .map(|r| r.file.as_str()),
            Some("ui/status.html")
        );
        assert!(contract.ui_resource_for("fixture_read").is_none());
        // Without a declaration every signed tool keeps today's meaning.
        let plain = contract_without_apps();
        assert!(plain.model_visible("fixture_read") && plain.app_visible("fixture_read"));
        assert!(
            !plain.read_only("fixture_read") && plain.ui_resource_for("fixture_read").is_none()
        );
        // Old contracts serialize exactly as before.
        assert!(serde_json::to_value(&plain).unwrap().get("apps").is_none());
    }

    fn contract_without_apps() -> McpServerContract {
        contract()
    }

    #[test]
    fn apps_reject_unsafe_or_dangling_declarations() {
        let view = |uri: &str, file: &str, sha: String| json!({"uri":uri,"file":file,"sha256":sha});
        for apps in [
            json!({}),
            json!({"ui_resources":[view("https://x.invalid/a", "a.html", digest())]}),
            json!({"ui_resources":[view("ui://fixture/../a", "a.html", digest())]}),
            json!({"ui_resources":[view("ui://Fixture/a", "a.html", digest())]}),
            json!({"ui_resources":[view("ui://fixture/a", "../a.html", digest())]}),
            json!({"ui_resources":[view("ui://fixture/a", "/a.html", digest())]}),
            json!({"ui_resources":[view("ui://fixture/a", "a.js", digest())]}),
            json!({"ui_resources":[view("ui://fixture/a", "a.html", format!("sha256:{}", "B".repeat(64)))]}),
            json!({"ui_resources":[view("ui://fixture/a", "a.html", digest()), view("ui://fixture/a", "b.html", digest())]}),
            json!({"ui_resources":(0..9).map(|n| view(&format!("ui://fixture/{n}"), &format!("{n}.html"), digest())).collect::<Vec<_>>()}),
            json!({"tools":[{"name":"unsigned","visibility":["app"]}]}),
            json!({"tools":[{"name":"fixture_read","visibility":[]}]}),
            json!({"tools":[{"name":"fixture_read","visibility":["app","app"]}]}),
            json!({"tools":[{"name":"fixture_read","visibility":["app"]},{"name":"fixture_read","visibility":["model"]}]}),
            json!({"tools":[{"name":"fixture_read","visibility":["app"],"resource":"ui://fixture/missing"}]}),
            json!({"tools":[{"name":"fixture_read","visibility":["everyone"]}]}),
            json!({"tools":[{"name":"fixture_read","visibility":["app"],"csp":{}}]}),
        ] {
            assert!(with_apps(apps.clone()).is_err(), "{apps}");
        }
    }
}

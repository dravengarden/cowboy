//! Generic, signed Cowboy plugin package and release contracts.
//!
//! Agent Provider data is one typed payload kind. It does not own the Catalog,
//! release, installation, or Machine lifecycle.

#![warn(clippy::pedantic)]

use std::collections::BTreeSet;

use anyhow::{Context as _, Result, bail, ensure};
use cowboy_provider_sdk::{
    AgentRuntimeBinding, PlatformRuntimeArtifacts, PlatformTarget, PrivateComponentKind,
    ProviderArtifactFormat, ProviderArtifactProbe, ProviderPackage,
    RUNTIME_BINDING_SCHEMA_VERSION as PROVIDER_RUNTIME_BINDING_SCHEMA_VERSION,
    ReleasedPrivateComponent,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const PACKAGE_SCHEMA_VERSION: u16 = 1;
pub const RELEASE_SCHEMA_VERSION: u16 = 1;
pub const PLUGIN_RELEASE_SIGNATURE_NAMESPACE: &str = "cowboy-plugin-release-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub publisher: String,
    pub kind: PluginKind,
    pub entrypoint: String,
    pub components: Vec<ComponentDependency>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    AgentProvider,
    CodeIntelligence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentDependency {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPackage {
    pub package_schema: u16,
    pub manifest: PluginManifest,
    pub component_release: String,
    pub payload: PluginPayload,
    pub contract_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "contract", rename_all = "snake_case")]
pub enum PluginPayload {
    AgentProvider(Box<ProviderPackage>),
    CodeIntelligence(CodeIntelligenceContract),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeIntelligenceContract {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub transport: String,
    pub operations: Vec<String>,
    pub states: Vec<String>,
    pub supported_platforms: Vec<PlatformTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRelease {
    pub release_schema: u16,
    pub plugin_id: String,
    pub plugin_version: String,
    pub plugin_kind: PluginKind,
    pub package_digest: String,
    pub artifact_digest: String,
    pub artifact_url: String,
    pub publisher: String,
    pub contract_fingerprint: String,
    pub component_release: String,
    pub signature: String,
    pub supported_platforms: Vec<PlatformTarget>,
    pub runtime_artifacts: Vec<PluginRuntimeArtifacts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRuntimeArtifacts {
    pub os: cowboy_provider_sdk::OperatingSystem,
    pub architecture: cowboy_provider_sdk::Architecture,
    pub components: Vec<ReleasedPluginComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasedPluginComponent {
    pub kind: PluginComponentKind,
    pub slot: String,
    pub dependency: String,
    pub version: String,
    pub command: String,
    pub artifact_url: String,
    pub artifact_digest: String,
    pub artifact_format: PluginArtifactFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    pub probe: PluginArtifactProbe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginComponentKind {
    AgentCli,
    AgentAdapter,
    AgentGateway,
    AcpRuntime,
    CodeIntelligenceAdapter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginArtifactFormat {
    Raw,
    TarGz,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginArtifactProbe {
    #[serde(default)]
    pub args: Vec<String>,
    pub timeout_ms: u64,
}

impl PluginManifest {
    /// Validate the generic identity and exact component dependency graph.
    ///
    /// # Errors
    /// Returns an error when identity, version, entrypoint, or component pins
    /// are invalid or incomplete.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported plugin manifest schema"
        );
        validate_id(&self.id, "plugin id")?;
        validate_version(&self.version, "plugin version")?;
        ensure!(
            !self.publisher.trim().is_empty(),
            "plugin publisher is empty"
        );
        ensure!(
            !self.entrypoint.is_empty()
                && !self.entrypoint.starts_with('/')
                && !self.entrypoint.split('/').any(|part| part == ".."),
            "invalid plugin entrypoint"
        );
        ensure!(!self.components.is_empty(), "plugin has no components");
        let mut ids = BTreeSet::new();
        for component in &self.components {
            let suffix = component
                .id
                .strip_prefix("cowboy.")
                .context("component id must use the cowboy namespace")?;
            validate_id(suffix, "component id")?;
            validate_version(&component.version, "component version")?;
            ensure!(
                ids.insert(component.id.as_str()),
                "duplicate component dependency"
            );
        }
        ensure!(
            ids.contains("cowboy.plugin-contract"),
            "missing plugin contract"
        );
        Ok(())
    }
}

impl PluginPackage {
    /// Construct and fingerprint one validated generic Plugin package.
    ///
    /// # Errors
    /// Returns an error when the manifest, payload binding, or component
    /// release is invalid.
    pub fn new(
        manifest: PluginManifest,
        component_release: String,
        payload: PluginPayload,
    ) -> Result<Self> {
        manifest.validate()?;
        validate_version(&component_release, "component release")?;
        validate_payload(&manifest, &payload)?;
        let contract_fingerprint = fingerprint_json(&serde_json::json!({
            "manifest": &manifest,
            "component_release": &component_release,
            "payload": &payload,
        }))?;
        Ok(Self {
            package_schema: PACKAGE_SCHEMA_VERSION,
            manifest,
            component_release,
            payload,
            contract_fingerprint,
        })
    }

    /// Parse canonical package bytes and recompute their contract fingerprint.
    ///
    /// # Errors
    /// Returns an error for malformed, unsupported, or inconsistent bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let package: Self = serde_json::from_slice(bytes)?;
        ensure!(
            package.package_schema == PACKAGE_SCHEMA_VERSION,
            "unsupported plugin package schema"
        );
        let rebuilt = Self::new(
            package.manifest.clone(),
            package.component_release.clone(),
            package.payload.clone(),
        )?;
        ensure!(
            package.contract_fingerprint == rebuilt.contract_fingerprint,
            "plugin contract fingerprint mismatch"
        );
        Ok(package)
    }

    /// Serialize the package in its stable newline-terminated representation.
    ///
    /// # Errors
    /// Returns an error when the package cannot be serialized.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    #[must_use]
    pub fn artifact_digest(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    #[must_use]
    pub fn agent_provider(&self) -> Option<&ProviderPackage> {
        match &self.payload {
            PluginPayload::AgentProvider(provider) => Some(provider),
            PluginPayload::CodeIntelligence(_) => None,
        }
    }

    fn expected_platforms(&self) -> BTreeSet<PlatformTarget> {
        match &self.payload {
            PluginPayload::AgentProvider(provider) => provider
                .manifest
                .runtime
                .platforms
                .iter()
                .map(|platform| PlatformTarget {
                    os: platform.os.clone(),
                    architecture: platform.architecture.clone(),
                })
                .collect(),
            PluginPayload::CodeIntelligence(contract) => {
                contract.supported_platforms.iter().cloned().collect()
            }
        }
    }
}

impl PluginRelease {
    /// Project an Agent Plugin's generic release binding into the typed
    /// Provider capability validator used by the Machine runtime.
    ///
    /// # Errors
    /// Returns an error for non-Agent plugins or invalid runtime bindings.
    pub fn agent_provider_binding(&self, package: &PluginPackage) -> Result<AgentRuntimeBinding> {
        let provider = package
            .agent_provider()
            .context("Plugin is not an Agent Provider")?;
        let runtime_artifacts = self
            .runtime_artifacts
            .iter()
            .map(PluginRuntimeArtifacts::to_agent_provider)
            .collect::<Result<Vec<_>>>()?;
        let mut release = AgentRuntimeBinding {
            binding_schema: PROVIDER_RUNTIME_BINDING_SCHEMA_VERSION,
            provider_id: self.plugin_id.clone(),
            provider_version: self.plugin_version.clone(),
            package_digest: ProviderPackage::artifact_digest(&provider.canonical_bytes()?),
            artifact_digest: String::new(),
            publisher: self.publisher.clone(),
            contract_fingerprint: provider.contract_fingerprint.clone(),
            supported_platforms: self.supported_platforms.clone(),
            runtime_artifacts,
        };
        release.artifact_digest = release.computed_artifact_digest()?;
        release.validate_for(provider)?;
        Ok(release)
    }

    /// Validate a release against exact package bytes.
    ///
    /// # Errors
    /// Returns an error for invalid package bytes or any release mismatch.
    pub fn validate_bytes(&self, bytes: &[u8]) -> Result<PluginPackage> {
        let package = PluginPackage::from_bytes(bytes)?;
        self.validate_for(&package)?;
        ensure!(
            self.package_digest == PluginPackage::artifact_digest(bytes),
            "plugin release package digest mismatch"
        );
        Ok(package)
    }

    /// Validate the complete release identity and runtime matrix.
    ///
    /// # Errors
    /// Returns an error for identity, signature presence, platform, component,
    /// or digest mismatches.
    pub fn validate_for(&self, package: &PluginPackage) -> Result<()> {
        ensure!(
            self.release_schema == RELEASE_SCHEMA_VERSION,
            "unsupported plugin release schema"
        );
        ensure!(
            self.plugin_id == package.manifest.id,
            "plugin release id mismatch"
        );
        ensure!(
            self.plugin_version == package.manifest.version,
            "plugin release version mismatch"
        );
        ensure!(
            self.plugin_kind == package.manifest.kind,
            "plugin release kind mismatch"
        );
        ensure!(
            self.publisher == package.manifest.publisher,
            "plugin release publisher mismatch"
        );
        ensure!(
            self.contract_fingerprint == package.contract_fingerprint,
            "plugin release contract mismatch"
        );
        ensure!(
            self.component_release == package.component_release,
            "plugin component release mismatch"
        );
        validate_digest(&self.package_digest, "plugin package digest")?;
        validate_digest(&self.artifact_digest, "plugin artifact digest")?;
        ensure!(
            self.artifact_url.starts_with("https://"),
            "plugin artifact URL must use HTTPS"
        );
        ensure!(
            !self.signature.trim().is_empty(),
            "plugin release is unsigned"
        );
        let expected = package.expected_platforms();
        let supported = self
            .supported_platforms
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        ensure!(
            supported.len() == self.supported_platforms.len() && supported == expected,
            "plugin platform matrix mismatch"
        );
        ensure!(
            self.artifact_digest == self.computed_artifact_digest()?,
            "plugin composite artifact digest mismatch"
        );
        if package.agent_provider().is_some() {
            self.agent_provider_binding(package)?;
        }
        Ok(())
    }

    /// Compute the composite package and runtime-artifact identity.
    ///
    /// # Errors
    /// Returns an error when the release identity cannot be serialized.
    pub fn computed_artifact_digest(&self) -> Result<String> {
        fingerprint_json(&serde_json::json!({
            "release_schema": self.release_schema,
            "plugin_id": self.plugin_id,
            "plugin_version": self.plugin_version,
            "plugin_kind": self.plugin_kind,
            "package_digest": self.package_digest,
            "publisher": self.publisher,
            "contract_fingerprint": self.contract_fingerprint,
            "component_release": self.component_release,
            "supported_platforms": self.supported_platforms,
            "runtime_artifacts": self.runtime_artifacts,
        }))
    }

    #[must_use]
    pub fn proof(&self) -> Vec<u8> {
        let fields = [
            self.plugin_id.as_str(),
            self.plugin_version.as_str(),
            self.package_digest.as_str(),
            self.artifact_digest.as_str(),
            self.publisher.as_str(),
            self.contract_fingerprint.as_str(),
            self.component_release.as_str(),
        ];
        let mut proof = format!("{PLUGIN_RELEASE_SIGNATURE_NAMESPACE}\n").into_bytes();
        for field in fields {
            proof.extend_from_slice(field.len().to_string().as_bytes());
            proof.push(b':');
            proof.extend_from_slice(field.as_bytes());
            proof.push(b'\n');
        }
        proof
    }
}

impl PluginRuntimeArtifacts {
    fn to_agent_provider(&self) -> Result<PlatformRuntimeArtifacts> {
        Ok(PlatformRuntimeArtifacts {
            os: self.os.clone(),
            architecture: self.architecture.clone(),
            components: self
                .components
                .iter()
                .map(ReleasedPluginComponent::to_agent_provider)
                .collect::<Result<Vec<_>>>()?,
        })
    }
}

impl ReleasedPluginComponent {
    fn to_agent_provider(&self) -> Result<ReleasedPrivateComponent> {
        let kind = match self.kind {
            PluginComponentKind::AgentCli => PrivateComponentKind::ProviderCli,
            PluginComponentKind::AgentAdapter => PrivateComponentKind::ProviderAdapter,
            PluginComponentKind::AgentGateway => PrivateComponentKind::ProviderGateway,
            PluginComponentKind::AcpRuntime => PrivateComponentKind::AcpRuntime,
            PluginComponentKind::CodeIntelligenceAdapter => {
                bail!("code-intelligence component cannot satisfy an agent Provider payload")
            }
        };
        Ok(ReleasedPrivateComponent {
            kind,
            slot: self.slot.clone(),
            dependency: self.dependency.clone(),
            version: self.version.clone(),
            command: self.command.clone(),
            artifact_url: self.artifact_url.clone(),
            artifact_digest: self.artifact_digest.clone(),
            artifact_format: match self.artifact_format {
                PluginArtifactFormat::Raw => ProviderArtifactFormat::Raw,
                PluginArtifactFormat::TarGz => ProviderArtifactFormat::TarGz,
            },
            entrypoint: self.entrypoint.clone(),
            probe: ProviderArtifactProbe {
                args: self.probe.args.clone(),
                timeout_ms: self.probe.timeout_ms,
            },
        })
    }
}

fn validate_payload(manifest: &PluginManifest, payload: &PluginPayload) -> Result<()> {
    match (manifest.kind, payload) {
        (PluginKind::AgentProvider, PluginPayload::AgentProvider(provider)) => {
            ensure!(
                provider.manifest.id == manifest.id,
                "agent payload id mismatch"
            );
            ensure!(
                provider.manifest.version == manifest.version,
                "agent payload version mismatch"
            );
            ensure!(
                provider.manifest.publisher == manifest.publisher,
                "agent payload publisher mismatch"
            );
        }
        (PluginKind::CodeIntelligence, PluginPayload::CodeIntelligence(contract)) => {
            ensure!(
                contract.schema_version == 1,
                "unsupported code-intelligence contract"
            );
            ensure!(
                contract.id == manifest.id,
                "code-intelligence payload id mismatch"
            );
            ensure!(
                contract.version == manifest.version,
                "code-intelligence payload version mismatch"
            );
            ensure!(
                !contract.operations.is_empty(),
                "code-intelligence operations are empty"
            );
            ensure!(
                !contract.supported_platforms.is_empty(),
                "code-intelligence platforms are empty"
            );
        }
        _ => bail!("plugin kind and payload kind mismatch"),
    }
    Ok(())
}

fn fingerprint_json(value: &serde_json::Value) -> Result<String> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

fn validate_digest(value: &str, label: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        bail!("{label} must use sha256")
    };
    ensure!(
        hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid {label}"
    );
    Ok(())
}

fn validate_id(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            }),
        "invalid {label}"
    );
    Ok(())
}

fn validate_version(value: &str, label: &str) -> Result<()> {
    let version = Version::parse(value).with_context(|| format!("invalid {label}"))?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "{label} must be exact stable SemVer"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_rejects_ranges_and_duplicate_components() {
        let mut manifest = PluginManifest {
            schema_version: 1,
            id: "zed".to_owned(),
            version: "1.0.0".to_owned(),
            publisher: "cowboy-project".to_owned(),
            kind: PluginKind::CodeIntelligence,
            entrypoint: "adapter/Cargo.toml".to_owned(),
            components: vec![ComponentDependency {
                id: "cowboy.plugin-contract".to_owned(),
                version: "1.0.0".to_owned(),
            }],
        };
        manifest.validate().unwrap();
        manifest.components.push(manifest.components[0].clone());
        assert!(manifest.validate().is_err());
        manifest.components.pop();
        manifest.version = "1.x".to_owned();
        assert!(manifest.validate().is_err());
    }
}

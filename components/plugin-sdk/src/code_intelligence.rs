//! The transport belongs to the host; executable names, pins and argv belong
//! to the signed Plugin. No shell expansion or ambient executable lookup.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use anyhow::{Context as _, Result, ensure};
use cowboy_provider_sdk::PlatformTarget;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    CodeIntelligenceContract, PluginArtifactFormat, PluginComponentKind, PluginRelease,
    ReleasedPluginComponent, validate_digest, validate_id, validate_version,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeIntelligenceRuntime {
    pub components: Vec<CodeRuntimeComponent>,
    pub launch: CodeRuntimeCommand,
    pub readiness: CodeRuntimeCommand,
    pub readiness_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeRuntimeComponent {
    pub kind: PluginComponentKind,
    pub slot: String,
    pub dependency: String,
    pub version: String,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeRuntimeCommand {
    pub command: String,
    pub arguments: Vec<CodeRuntimeArgument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodeRuntimeArgument {
    Literal { value: String },
    ComponentCommand { command: String },
    Socket,
    StateDirectory,
}

pub(crate) fn validate_contract(contract: &CodeIntelligenceContract) -> Result<()> {
    ensure!(
        contract.supported_platforms.len() <= 16
            && contract
                .supported_platforms
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == contract.supported_platforms.len(),
        "duplicate or excessive code-intelligence platforms"
    );
    if contract.schema_version == 1 {
        ensure!(
            contract.runtime.is_none(),
            "legacy code contract cannot declare a runtime"
        );
        return Ok(());
    }
    ensure!(
        contract.transport == "unix_socket_json_v1",
        "unsupported code-intelligence runtime transport"
    );
    let runtime = contract
        .runtime
        .as_ref()
        .context("code contract requires an exact runtime")?;
    ensure!(
        (1..=16).contains(&runtime.components.len()),
        "invalid code runtime component count"
    );
    let mut slots = BTreeSet::new();
    let mut commands = BTreeSet::new();
    for component in &runtime.components {
        ensure!(
            is_code_kind(component.kind),
            "invalid code runtime component kind"
        );
        validate_id(&component.slot, "code runtime slot")?;
        validate_id(&component.dependency, "code runtime dependency")?;
        validate_version(&component.version, "code runtime version")?;
        validate_id(&component.command, "code runtime command")?;
        ensure!(slots.insert(&component.slot), "duplicate code runtime slot");
        ensure!(
            commands.insert(&component.command),
            "duplicate code runtime command"
        );
    }
    ensure!(
        runtime.components.iter().any(|component| {
            component.kind == PluginComponentKind::CodeIntelligenceAdapter
                && component.command == runtime.launch.command
        }),
        "code runtime launch must bind its declared adapter"
    );
    for invocation in [&runtime.launch, &runtime.readiness] {
        ensure!(
            commands.contains(&invocation.command),
            "undeclared code runtime command"
        );
        ensure!(
            invocation.arguments.len() <= 64,
            "too many code runtime arguments"
        );
        for argument in &invocation.arguments {
            match argument {
                CodeRuntimeArgument::Literal { value } => ensure!(
                    value.len() <= 4096 && !value.contains('\0'),
                    "invalid code runtime argument"
                ),
                CodeRuntimeArgument::ComponentCommand { command } => ensure!(
                    commands.contains(command),
                    "undeclared code runtime command binding"
                ),
                CodeRuntimeArgument::Socket | CodeRuntimeArgument::StateDirectory => {}
            }
        }
        ensure!(
            invocation.arguments.contains(&CodeRuntimeArgument::Socket),
            "code runtime command must bind its private socket"
        );
    }
    ensure!(
        (100..=120_000).contains(&runtime.readiness_timeout_ms),
        "invalid code runtime readiness timeout"
    );
    Ok(())
}

pub(crate) fn validate_release(
    contract: &CodeIntelligenceContract,
    release: &PluginRelease,
) -> Result<()> {
    let expected: BTreeSet<_> = contract.supported_platforms.iter().cloned().collect();
    let mut targets = BTreeSet::new();
    for artifacts in &release.runtime_artifacts {
        let target = PlatformTarget {
            os: artifacts.os.clone(),
            architecture: artifacts.architecture.clone(),
        };
        ensure!(expected.contains(&target), "undeclared code runtime target");
        ensure!(targets.insert(target), "duplicate code runtime target");
        ensure!(
            (1..=16).contains(&artifacts.components.len()),
            "invalid code runtime artifact count"
        );
        let mut slots = BTreeSet::new();
        let mut commands = BTreeSet::new();
        for artifact in &artifacts.components {
            validate_artifact(artifact)?;
            ensure!(
                slots.insert(&artifact.slot),
                "duplicate code runtime artifact slot"
            );
            ensure!(
                commands.insert(&artifact.command),
                "duplicate code runtime artifact command"
            );
            if let Some(runtime) = &contract.runtime {
                let declared = runtime
                    .components
                    .iter()
                    .find(|component| component.slot == artifact.slot)
                    .context("undeclared code runtime artifact")?;
                ensure!(
                    declared.kind == artifact.kind
                        && declared.dependency == artifact.dependency
                        && declared.version == artifact.version
                        && declared.command == artifact.command,
                    "code runtime artifact does not match its exact dependency pin"
                );
            }
        }
        ensure!(
            artifacts
                .components
                .iter()
                .any(|component| component.kind == PluginComponentKind::CodeIntelligenceAdapter),
            "code runtime artifact matrix has no adapter"
        );
        if let Some(runtime) = &contract.runtime {
            ensure!(
                runtime.components.len() == artifacts.components.len(),
                "incomplete code runtime artifact graph"
            );
        }
    }
    ensure!(
        targets == expected,
        "incomplete code runtime artifact platform matrix"
    );
    Ok(())
}

fn is_code_kind(kind: PluginComponentKind) -> bool {
    matches!(
        kind,
        PluginComponentKind::CodeIntelligenceAdapter | PluginComponentKind::CodeIntelligenceServer
    )
}

fn validate_artifact(artifact: &ReleasedPluginComponent) -> Result<()> {
    ensure!(
        is_code_kind(artifact.kind),
        "invalid code runtime artifact kind"
    );
    for (value, label) in [
        (&artifact.slot, "code runtime artifact slot"),
        (&artifact.dependency, "code runtime artifact dependency"),
        (&artifact.command, "code runtime artifact command"),
    ] {
        validate_id(value, label)?;
    }
    validate_version(&artifact.version, "code runtime artifact version")?;
    validate_digest(&artifact.artifact_digest, "code runtime artifact digest")?;
    let url = Url::parse(&artifact.artifact_url).context("invalid code runtime artifact URL")?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    ensure!(
        (url.scheme() == "https" || (url.scheme() == "http" && loopback))
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && artifact.artifact_url.len() <= 2048
            && !artifact.artifact_url.contains("latest")
            && !artifact
                .artifact_url
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte == 0),
        "invalid code runtime artifact URL"
    );
    ensure!(
        artifact.probe.args.len() <= 64
            && artifact
                .probe
                .args
                .iter()
                .all(|argument| argument.len() <= 4096 && !argument.contains('\0'))
            && (100..=120_000).contains(&artifact.probe.timeout_ms),
        "invalid code runtime artifact probe"
    );
    match artifact.artifact_format {
        PluginArtifactFormat::Raw => ensure!(
            artifact.entrypoint.is_none(),
            "raw code runtime has an entrypoint"
        ),
        PluginArtifactFormat::TarGz => {
            let entrypoint = artifact
                .entrypoint
                .as_deref()
                .context("code runtime archive requires an entrypoint")?;
            ensure!(
                !entrypoint.is_empty()
                    && entrypoint.len() <= 4096
                    && !entrypoint.contains(['\0', '\\'])
                    && Path::new(entrypoint)
                        .components()
                        .all(|component| matches!(component, Component::Normal(_))),
                "unsafe code runtime archive entrypoint"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PluginArtifactProbe, PluginKind, PluginManifest, PluginPackage, PluginPayload};
    use cowboy_provider_sdk::{Architecture, OperatingSystem};

    fn fixture() -> (PluginPackage, PluginRelease) {
        let manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "fixture-code", "version": "1.0.0",
            "component_release": "2.5.0", "publisher": "fixture-publisher",
            "kind": "code_intelligence", "entrypoint": "contract.json",
            "components": [{"id": "cowboy.plugin-contract", "version": "1.6.0"}, {"id": "cowboy.plugin-sdk", "version": "1.6.0"}]
        }))
        .unwrap();
        let runtime: CodeIntelligenceRuntime = serde_json::from_value(serde_json::json!({
            "components": [
                {"kind": "code_intelligence_adapter", "slot": "adapter", "dependency": "adapter", "version": "1.0.0", "command": "adapter"},
                {"kind": "code_intelligence_server", "slot": "server", "dependency": "server", "version": "2.0.0", "command": "server"}
            ],
            "launch": {"command": "adapter", "arguments": [{"kind": "socket"}, {"kind": "component_command", "command": "server"}]},
            "readiness": {"command": "adapter", "arguments": [{"kind": "socket"}]},
            "readiness_timeout_ms": 1000
        })).unwrap();
        let contract = CodeIntelligenceContract {
            schema_version: 2,
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            transport: "unix_socket_json_v1".to_owned(),
            operations: vec!["hover".to_owned()],
            states: vec!["ready".to_owned()],
            supported_platforms: vec![PlatformTarget {
                os: OperatingSystem::Linux,
                architecture: Architecture::X86_64,
            }],
            runtime: Some(runtime.clone()),
        };
        let package = PluginPackage::new(
            manifest,
            "2.5.0".to_owned(),
            PluginPayload::CodeIntelligence(contract.clone()),
        )
        .unwrap();
        let mut release = PluginRelease {
            release_schema: 1,
            plugin_id: package.manifest.id.clone(),
            plugin_version: "1.0.0".to_owned(),
            plugin_kind: PluginKind::CodeIntelligence,
            publisher: package.manifest.publisher.clone(),
            package_digest: PluginPackage::artifact_digest(&package.canonical_bytes().unwrap()),
            artifact_digest: String::new(),
            artifact_url: "https://example.test/fixture.cowboy-plugin".to_owned(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            component_release: package.component_release.clone(),
            host_bundle_digest: None,
            signature: "fixture".to_owned(),
            supported_platforms: contract.supported_platforms,
            runtime_artifacts: vec![crate::PluginRuntimeArtifacts {
                os: OperatingSystem::Linux,
                architecture: Architecture::X86_64,
                components: runtime
                    .components
                    .iter()
                    .map(|component| ReleasedPluginComponent {
                        kind: component.kind,
                        slot: component.slot.clone(),
                        dependency: component.dependency.clone(),
                        version: component.version.clone(),
                        command: component.command.clone(),
                        artifact_url: "https://example.test/pinned.tar.gz".to_owned(),
                        artifact_digest: format!("sha256:{}", "ab".repeat(32)),
                        artifact_format: PluginArtifactFormat::TarGz,
                        entrypoint: Some("bin/runtime".to_owned()),
                        probe: PluginArtifactProbe {
                            args: vec!["--help".to_owned()],
                            timeout_ms: 1000,
                        },
                    })
                    .collect(),
            }],
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        (package, release)
    }

    #[test]
    fn owned_code_runtime_requires_every_exact_pin_and_target() {
        let (package, release) = fixture();
        release.validate_for(&package).unwrap();
        let mutations: Vec<fn(&mut PluginRelease)> = vec![
            |release| {
                release.runtime_artifacts.clear();
            },
            |release| {
                release
                    .runtime_artifacts
                    .push(release.runtime_artifacts[0].clone());
            },
            |release| {
                release.runtime_artifacts[0].components.pop();
            },
            |release| {
                release.runtime_artifacts[0].components[1].version = "2.0.1".to_owned();
            },
            |release| {
                release.runtime_artifacts[0].components[1].command = "ambient".to_owned();
            },
            |release| {
                release.runtime_artifacts[0].components[1].kind = PluginComponentKind::AgentCli;
            },
            |release| {
                release.runtime_artifacts[0].components[1].slot = "adapter".to_owned();
            },
            |release| {
                release.runtime_artifacts[0].components[1].entrypoint =
                    Some("../server".to_owned());
            },
            |release| {
                release.runtime_artifacts[0].components[1].artifact_url =
                    "http://example.test/latest".to_owned();
            },
            |release| {
                release.runtime_artifacts[0].components[1].probe.timeout_ms = 0;
            },
        ];
        for mutate in mutations {
            let mut invalid = release.clone();
            mutate(&mut invalid);
            invalid.artifact_digest = invalid.computed_artifact_digest().unwrap();
            assert!(
                invalid.validate_for(&package).is_err(),
                "accepted malformed code runtime matrix"
            );
        }
    }

    #[test]
    fn owned_code_runtime_has_no_ambient_command_or_schema_downgrade() {
        let (package, _) = fixture();
        let PluginPayload::CodeIntelligence(mut contract) = package.payload else {
            panic!("fixture kind");
        };
        let valid = contract.clone();
        contract.runtime.as_mut().unwrap().launch.arguments.push(
            CodeRuntimeArgument::ComponentCommand {
                command: "ambient".to_owned(),
            },
        );
        assert!(validate_contract(&contract).is_err());
        contract = valid.clone();
        contract.schema_version = 1;
        assert!(validate_contract(&contract).is_err());
        contract.runtime = None;
        validate_contract(&contract).unwrap();
        contract = valid;
        contract.runtime = None;
        assert!(validate_contract(&contract).is_err());
    }

    #[test]
    fn authentication_cannot_smuggle_a_machine_runtime() {
        let manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "fixture-auth", "version": "1.0.0", "component_release": "2.5.0",
            "publisher": "fixture-publisher", "kind": "authentication_provider", "entrypoint": "authentication.json",
            "components": [{"id": "cowboy.plugin-contract", "version": "1.6.0"}, {"id": "cowboy.plugin-sdk", "version": "1.6.0"}]
        })).unwrap();
        let contract = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "fixture-auth", "version": "1.0.0", "display_name": "Fixture", "button_label": "Continue",
            "protocol": {"kind": "open_id_connect", "configuration": {
                "issuer": "https://example.test", "authorization_endpoint": "https://example.test/auth", "token_endpoint": "https://example.test/token",
                "jwks_uri": "https://example.test/jwks", "scopes": ["openid"], "client_authentication_methods": ["client_secret_post"],
                "id_token_signing_algorithms": ["RS256"]
            }}
        })).unwrap();
        let package = PluginPackage::new(
            manifest,
            "2.5.0".to_owned(),
            PluginPayload::AuthenticationProvider(contract),
        )
        .unwrap();
        let (_, mut release) = fixture();
        release.plugin_id.clone_from(&package.manifest.id);
        release.plugin_kind = PluginKind::AuthenticationProvider;
        release
            .contract_fingerprint
            .clone_from(&package.contract_fingerprint);
        release.package_digest =
            PluginPackage::artifact_digest(&package.canonical_bytes().unwrap());
        release.supported_platforms.clear();
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        assert!(
            release
                .validate_for(&package)
                .unwrap_err()
                .to_string()
                .contains("Machine runtime")
        );
    }
}

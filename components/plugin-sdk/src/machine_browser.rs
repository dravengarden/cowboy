//! Machine browser Plugins: a signed browser runtime that core runs on a Linux
//! Machine for owner-operated account sign-in. The Plugin ships bytes and a
//! closed launch contract; admission, isolation, streaming and the UI stay
//! core (`docs/machine-browser.md`). Nothing here names a URL, a credential or
//! an executable outside the signed artifact.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use anyhow::{Context as _, Result, ensure};
use cowboy_provider_sdk::{Architecture, OperatingSystem, PlatformTarget};
use serde::{Deserialize, Serialize};

use crate::{
    PluginArtifactFormat, PluginComponentKind, PluginRelease, ReleasedPluginComponent,
    validate_digest, validate_id, validate_version,
};

pub const MACHINE_BROWSER_SCHEMA_VERSION: u16 = 1;
/// Plugin SDK release that introduced the Machine browser kind.
pub const MACHINE_BROWSER_SDK: (u64, u64, u64) = (1, 13, 0);
/// The only launcher control protocol core speaks.
pub const MACHINE_BROWSER_CONTROL_PROTOCOL: &str = "cowboy-browser-control-v1";
const MAX_HELPERS: usize = 16;
const MAX_MOUNT_POINTS: usize = 8;
const MAX_APPARMOR_FEATURES: usize = 16;
const MAX_RELATIVE_PATH: usize = 1024;
/// Readiness covers namespace setup, snapshot restoration and three process
/// starts; the bounds match the code-intelligence runtime probe.
const READINESS_TIMEOUT_MS: std::ops::RangeInclusive<u64> = 100..=120_000;

/// A browser runtime core may run on a Machine for one Provider account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineBrowserContract {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub supported_platforms: Vec<PlatformTarget>,
    pub host_requirements: MachineBrowserHostRequirements,
    pub runtime: MachineBrowserRuntime,
    pub launch: MachineBrowserLaunch,
}

/// What the host must still provide once the artifact is the instance's
/// whole userland: a kernel, merged-usr links, two mount points and an
/// `AppArmor` with the mediation the sealed policy relies on. The Machine
/// advertises the browser capability only when every requirement holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineBrowserHostRequirements {
    /// Minimum kernel release as `major.minor`.
    pub kernel_min: String,
    /// `/lib64`, `/lib` and `/bin` must resolve below `/usr` so the ELF
    /// interpreter resolves into the artifact.
    pub merged_usr: bool,
    /// Absolute directories the launcher mounts over; they must exist.
    pub mount_points: Vec<String>,
    /// Entries below `/sys/kernel/security/apparmor/features` that must exist.
    pub apparmor_features: Vec<String>,
    /// Provenance of the packaged distribution files; not a host requirement.
    pub built_from: MachineBrowserBuiltFrom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineBrowserBuiltFrom {
    pub id: String,
    pub version_id: String,
}

/// Artifact-relative executables and the browser policy binding. Every path
/// is normalized and relative; core resolves it inside the installed
/// generation before each launch and never falls back to an ambient binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineBrowserRuntime {
    /// Slot of the single released artifact this runtime lives in.
    pub artifact: String,
    pub launcher: String,
    pub browser: String,
    /// Full version of the packaged browser; the profile compatibility fence.
    pub browser_version: String,
    pub display_server: String,
    pub helpers: Vec<String>,
    pub policy: MachineBrowserPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineBrowserPolicy {
    /// Artifact-relative managed policy file.
    pub file: String,
    /// Absolute managed-policy directory of the packaged browser build.
    pub directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineBrowserLaunch {
    pub protocol: String,
    pub readiness_timeout_ms: u64,
}

pub(crate) fn validate_contract(contract: &MachineBrowserContract) -> Result<()> {
    ensure!(
        contract.schema_version == MACHINE_BROWSER_SCHEMA_VERSION,
        "unsupported machine browser schema"
    );
    validate_id(&contract.id, "machine browser id")?;
    validate_version(&contract.version, "machine browser version")?;
    for (value, label, limit) in [
        (&contract.display_name, "machine browser display name", 128),
        (&contract.description, "machine browser description", 512),
    ] {
        ensure!(
            !value.trim().is_empty()
                && value.len() <= limit
                && !value.chars().any(char::is_control),
            "invalid {label}"
        );
    }
    ensure!(
        (1..=4).contains(&contract.supported_platforms.len())
            && contract
                .supported_platforms
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == contract.supported_platforms.len(),
        "duplicate or excessive machine browser platforms"
    );
    ensure!(
        contract
            .supported_platforms
            .iter()
            .all(|platform| platform.os == OperatingSystem::Linux
                && platform.architecture == Architecture::X86_64),
        "machine browser runtimes are Linux x86_64 only"
    );
    validate_host_requirements(&contract.host_requirements)?;
    validate_runtime(&contract.runtime)?;
    ensure!(
        contract.launch.protocol == MACHINE_BROWSER_CONTROL_PROTOCOL,
        "unsupported machine browser control protocol"
    );
    ensure!(
        READINESS_TIMEOUT_MS.contains(&contract.launch.readiness_timeout_ms),
        "invalid machine browser readiness timeout"
    );
    Ok(())
}

fn validate_host_requirements(requirements: &MachineBrowserHostRequirements) -> Result<()> {
    ensure!(
        parse_kernel_release(&requirements.kernel_min).is_some(),
        "invalid machine browser kernel requirement"
    );
    ensure!(
        requirements.merged_usr,
        "machine browser runtimes require a merged-usr host"
    );
    ensure!(
        (1..=MAX_MOUNT_POINTS).contains(&requirements.mount_points.len())
            && requirements
                .mount_points
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == requirements.mount_points.len(),
        "duplicate or excessive machine browser mount points"
    );
    for mount_point in &requirements.mount_points {
        ensure!(
            is_absolute_normal_path(mount_point),
            "invalid machine browser mount point"
        );
    }
    ensure!(
        (1..=MAX_APPARMOR_FEATURES).contains(&requirements.apparmor_features.len())
            && requirements
                .apparmor_features
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == requirements.apparmor_features.len(),
        "duplicate or excessive machine browser AppArmor features"
    );
    for feature in &requirements.apparmor_features {
        ensure!(
            is_relative_normal_path(feature) && feature.len() <= 128,
            "invalid machine browser AppArmor feature"
        );
    }
    ensure!(
        !requirements.built_from.id.is_empty()
            && requirements.built_from.id.len() <= 64
            && requirements
                .built_from
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "invalid machine browser distribution id"
    );
    ensure!(
        !requirements.built_from.version_id.is_empty()
            && requirements.built_from.version_id.len() <= 32
            && requirements
                .built_from
                .version_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.'),
        "invalid machine browser distribution version"
    );
    Ok(())
}

fn validate_runtime(runtime: &MachineBrowserRuntime) -> Result<()> {
    validate_id(&runtime.artifact, "machine browser artifact slot")?;
    ensure!(
        parse_browser_version(&runtime.browser_version).is_some(),
        "invalid machine browser version"
    );
    ensure!(
        runtime.helpers.len() <= MAX_HELPERS,
        "too many machine browser helpers"
    );
    let mut executables = BTreeSet::new();
    for (path, label) in [
        (&runtime.launcher, "machine browser launcher"),
        (&runtime.browser, "machine browser executable"),
        (&runtime.display_server, "machine browser display server"),
    ]
    .into_iter()
    .chain(
        runtime
            .helpers
            .iter()
            .map(|helper| (helper, "machine browser helper")),
    ) {
        ensure!(is_relative_normal_path(path), "invalid {label} path");
        ensure!(executables.insert(path), "duplicate {label} path");
    }
    ensure!(
        is_relative_normal_path(&runtime.policy.file),
        "invalid machine browser policy file"
    );
    ensure!(
        !executables.contains(&runtime.policy.file),
        "machine browser policy file collides with an executable"
    );
    ensure!(
        is_absolute_normal_path(&runtime.policy.directory)
            && runtime.policy.directory.starts_with("/etc/"),
        "invalid machine browser policy directory"
    );
    Ok(())
}

/// One released artifact per declared platform: a `tar.gz` tree whose
/// entrypoint is the declared launcher and whose probe is bounded.
pub(crate) fn validate_release(
    contract: &MachineBrowserContract,
    release: &PluginRelease,
) -> Result<()> {
    ensure!(
        release.host_bundle_digest.is_none(),
        "machine browser cannot contain host code"
    );
    let expected: BTreeSet<_> = contract.supported_platforms.iter().cloned().collect();
    let mut targets = BTreeSet::new();
    for artifacts in &release.runtime_artifacts {
        let target = PlatformTarget {
            os: artifacts.os.clone(),
            architecture: artifacts.architecture.clone(),
        };
        ensure!(
            expected.contains(&target),
            "undeclared machine browser runtime target"
        );
        ensure!(
            targets.insert(target),
            "duplicate machine browser runtime target"
        );
        ensure!(
            artifacts.components.len() == 1,
            "machine browser runtime must be exactly one artifact"
        );
        validate_artifact(contract, &artifacts.components[0])?;
    }
    ensure!(
        targets == expected,
        "incomplete machine browser artifact platform matrix"
    );
    Ok(())
}

fn validate_artifact(
    contract: &MachineBrowserContract,
    artifact: &ReleasedPluginComponent,
) -> Result<()> {
    ensure!(
        artifact.kind == PluginComponentKind::MachineBrowserRuntime,
        "invalid machine browser artifact kind"
    );
    ensure!(
        artifact.slot == contract.runtime.artifact,
        "machine browser artifact slot does not match the contract"
    );
    validate_id(&artifact.dependency, "machine browser artifact dependency")?;
    validate_id(&artifact.command, "machine browser artifact command")?;
    validate_version(&artifact.version, "machine browser artifact version")?;
    ensure!(
        artifact.version == contract.version,
        "machine browser artifact version does not match the contract"
    );
    validate_digest(&artifact.artifact_digest, "machine browser artifact digest")?;
    let url =
        url::Url::parse(&artifact.artifact_url).context("invalid machine browser artifact URL")?;
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
        "invalid machine browser artifact URL"
    );
    ensure!(
        artifact.artifact_format == PluginArtifactFormat::TarGz,
        "machine browser runtime must be a tar.gz tree"
    );
    ensure!(
        artifact.entrypoint.as_deref() == Some(contract.runtime.launcher.as_str()),
        "machine browser archive entrypoint must be the declared launcher"
    );
    ensure!(
        artifact.probe.args.len() <= 8
            && artifact
                .probe
                .args
                .iter()
                .all(|argument| argument.len() <= 256 && !argument.contains('\0'))
            && READINESS_TIMEOUT_MS.contains(&artifact.probe.timeout_ms),
        "invalid machine browser artifact probe"
    );
    Ok(())
}

/// `major.minor` of a kernel release string such as `6.8.0-45-generic`.
#[must_use]
pub fn parse_kernel_release(release: &str) -> Option<(u64, u64)> {
    let mut parts = release.split(['.', '-', '+']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// Dotted numeric browser version, up to four components.
#[must_use]
pub fn parse_browser_version(version: &str) -> Option<Vec<u64>> {
    let parts: Vec<u64> = version
        .split('.')
        .map(|part| {
            (!part.is_empty() && part.len() <= 9 && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        })
        .collect::<Option<_>>()?;
    ((2..=4).contains(&parts.len())).then_some(parts)
}

fn is_relative_normal_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_RELATIVE_PATH
        && !path.contains(['\0', '\\'])
        && !path.ends_with('/')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn is_absolute_normal_path(path: &str) -> bool {
    path.len() > 1
        && path.len() <= MAX_RELATIVE_PATH
        && !path.contains(['\0', '\\'])
        && !path.ends_with('/')
        && {
            let mut components = Path::new(path).components();
            components.next() == Some(Component::RootDir)
                && components.all(|component| matches!(component, Component::Normal(_)))
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PluginArtifactProbe, PluginManifest, PluginPackage, PluginPayload, PluginRuntimeArtifacts,
    };

    pub(crate) fn contract_json() -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "id": "fixture-browser",
            "version": "1.2.3",
            "display_name": "Fixture browser",
            "description": "Owner-operated browser fixture.",
            "supported_platforms": [{"os": "linux", "architecture": "x86_64"}],
            "host_requirements": {
                "kernel_min": "5.15",
                "merged_usr": true,
                "mount_points": ["/opt", "/mnt"],
                "apparmor_features": ["network_v9", "unix", "mount", "namespaces/userns"],
                "built_from": {"id": "ubuntu", "version_id": "26.04"}
            },
            "runtime": {
                "artifact": "runtime",
                "launcher": "usr/libexec/cowboy-browser-launcher",
                "browser": "opt/chromium/chrome",
                "browser_version": "151.0.7922.71",
                "display_server": "usr/bin/Xtigervnc",
                "helpers": ["usr/bin/xrandr", "usr/bin/xkbcomp", "usr/bin/xauth", "usr/bin/matchbox-window-manager"],
                "policy": {"file": "usr/share/cowboy-browser/policy.json", "directory": "/etc/opt/chrome/policies/managed"}
            },
            "launch": {"protocol": "cowboy-browser-control-v1", "readiness_timeout_ms": 20000}
        })
    }

    fn manifest() -> PluginManifest {
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "fixture-browser", "version": "1.2.3",
            "component_release": "3.98.0", "publisher": "fixture-publisher",
            "kind": "machine_browser", "entrypoint": "contract.json",
            "components": [
                {"id": "cowboy.plugin-contract", "version": "1.12.0"},
                {"id": "cowboy.plugin-sdk", "version": "1.13.0"}
            ]
        }))
        .unwrap()
    }

    fn artifact(contract: &MachineBrowserContract) -> ReleasedPluginComponent {
        ReleasedPluginComponent {
            kind: PluginComponentKind::MachineBrowserRuntime,
            slot: contract.runtime.artifact.clone(),
            dependency: "fixture-browser-runtime".to_owned(),
            version: contract.version.clone(),
            command: "cowboy-browser-launcher".to_owned(),
            artifact_url: "https://plugins.example.test/fixture-browser-runtime.tar.gz".to_owned(),
            artifact_digest: format!("sha256:{}", "c".repeat(64)),
            artifact_format: PluginArtifactFormat::TarGz,
            entrypoint: Some(contract.runtime.launcher.clone()),
            probe: PluginArtifactProbe {
                args: vec!["--probe".to_owned()],
                timeout_ms: 5_000,
            },
        }
    }

    fn release(package: &PluginPackage, components: Vec<ReleasedPluginComponent>) -> PluginRelease {
        let bytes = package.canonical_bytes().unwrap();
        let mut release = PluginRelease {
            release_schema: package.minimum_release_schema(),
            plugin_id: package.manifest.id.clone(),
            plugin_version: package.manifest.version.clone(),
            plugin_kind: package.manifest.kind,
            package_digest: PluginPackage::artifact_digest(&bytes),
            artifact_digest: String::new(),
            artifact_url: "https://plugins.example.test/fixture-browser.cowboy-plugin".to_owned(),
            publisher: package.manifest.publisher.clone(),
            contract_fingerprint: package.contract_fingerprint.clone(),
            component_release: package.component_release.clone(),
            host_bundle_digest: None,
            signature: "pending-signature".to_owned(),
            supported_platforms: vec![PlatformTarget {
                os: OperatingSystem::Linux,
                architecture: Architecture::X86_64,
            }],
            runtime_artifacts: vec![PluginRuntimeArtifacts {
                os: OperatingSystem::Linux,
                architecture: Architecture::X86_64,
                components,
            }],
        };
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release
    }

    #[test]
    fn machine_browser_contract_round_trips_and_binds_one_artifact() {
        let contract: MachineBrowserContract = serde_json::from_value(contract_json()).unwrap();
        let package = PluginPackage::new(
            manifest(),
            "3.98.0".to_owned(),
            PluginPayload::MachineBrowser(contract.clone()),
        )
        .unwrap();
        assert_eq!(
            package.minimum_release_schema(),
            crate::MACHINE_BROWSER_RELEASE_SCHEMA_VERSION
        );
        release(&package, vec![artifact(&contract)])
            .validate_for(&package)
            .unwrap();

        // The runtime is exactly one tar.gz tree whose entrypoint is the launcher.
        let mut two = release(&package, vec![artifact(&contract), artifact(&contract)]);
        two.artifact_digest = two.computed_artifact_digest().unwrap();
        assert!(two.validate_for(&package).is_err());
        let mut raw = artifact(&contract);
        raw.artifact_format = PluginArtifactFormat::Raw;
        raw.entrypoint = None;
        assert!(release(&package, vec![raw]).validate_for(&package).is_err());
        let mut other_entry = artifact(&contract);
        other_entry.entrypoint = Some("opt/chromium/chrome".to_owned());
        assert!(
            release(&package, vec![other_entry])
                .validate_for(&package)
                .is_err()
        );
        let mut wrong_kind = artifact(&contract);
        wrong_kind.kind = PluginComponentKind::CodeIntelligenceAdapter;
        assert!(
            release(&package, vec![wrong_kind])
                .validate_for(&package)
                .is_err()
        );
        assert!(
            release(&package, Vec::new())
                .validate_for(&package)
                .is_err()
        );
    }

    #[test]
    fn machine_browser_contract_rejects_unsafe_paths_and_hosts() {
        let base = contract_json();
        let rejected = [
            ("/runtime/launcher", "launcher", "absolute path"),
            ("usr/../bin/launcher", "launcher", "parent component"),
            ("usr/bin/", "launcher", "trailing slash"),
        ];
        for (path, field, _) in rejected {
            let mut json = base.clone();
            json["runtime"][field] = serde_json::Value::String(path.to_owned());
            let contract: MachineBrowserContract = serde_json::from_value(json).unwrap();
            assert!(validate_contract(&contract).is_err(), "{path}");
        }
        let mut relative_dir = base.clone();
        relative_dir["runtime"]["policy"]["directory"] = "etc/opt/chrome".into();
        let contract: MachineBrowserContract = serde_json::from_value(relative_dir).unwrap();
        assert!(validate_contract(&contract).is_err());
        let mut macos = base.clone();
        macos["supported_platforms"] =
            serde_json::json!([{"os": "macos", "architecture": "aarch64"}]);
        let contract: MachineBrowserContract = serde_json::from_value(macos).unwrap();
        assert!(validate_contract(&contract).is_err());
        let mut no_merged_usr = base.clone();
        no_merged_usr["host_requirements"]["merged_usr"] = false.into();
        let contract: MachineBrowserContract = serde_json::from_value(no_merged_usr).unwrap();
        assert!(validate_contract(&contract).is_err());
        let mut bad_protocol = base.clone();
        bad_protocol["launch"]["protocol"] = "cowboy-browser-control-v2".into();
        let contract: MachineBrowserContract = serde_json::from_value(bad_protocol).unwrap();
        assert!(validate_contract(&contract).is_err());
        let mut bad_version = base;
        bad_version["runtime"]["browser_version"] = "151".into();
        let contract: MachineBrowserContract = serde_json::from_value(bad_version).unwrap();
        assert!(validate_contract(&contract).is_err());
    }

    #[test]
    fn kernel_and_browser_versions_parse() {
        assert_eq!(parse_kernel_release("6.8.0-45-generic"), Some((6, 8)));
        assert_eq!(parse_kernel_release("5.15"), Some((5, 15)));
        assert_eq!(parse_kernel_release("six"), None);
        assert_eq!(
            parse_browser_version("151.0.7922.71"),
            Some(vec![151, 0, 7922, 71])
        );
        assert_eq!(parse_browser_version("151"), None);
        assert_eq!(parse_browser_version("151.0.7922.71.1"), None);
    }
}

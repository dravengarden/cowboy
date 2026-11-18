//! Closed host-enforced runtime policies of a signed Provider manifest.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{ProviderManifest, RuntimeCapability};

/// Closed network policies a host must enforce on the Provider process tree,
/// or refuse to launch it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeNetworkPolicy {
    /// Only IPv4, Unix and netlink sockets; a vendor that judges location by
    /// address must see the host's stable IPv4 egress.
    Ipv4OnlyV1,
}

/// Closed process-isolation policies a host must enforce on every Provider
/// process (runtime, login, logout and probe), or refuse the Provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeIsolationPolicy {
    /// Every process of a named account runs as that account's own static
    /// system identity in a confined unit of the host's
    /// `cowboy-account-sandbox` daemon (protocol `cowboy-account-sandbox-v1`).
    /// The Machine never spawns the entrypoint itself and keeps no account
    /// home; a host without a reachable daemon reports the Provider
    /// unavailable.
    AccountSandboxV1,
}

/// Feature floors and companion requirements of the runtime policies.
pub(crate) fn validate(manifest: &ProviderManifest) -> Result<()> {
    let sdk = semver::Version::parse(&manifest.sdk_version)?;
    if manifest.runtime.network.is_some() {
        ensure!(
            sdk >= semver::Version::new(3, 4, 0),
            "runtime network policies require Provider SDK 3.4.0 or newer"
        );
    }
    if let Some(RuntimeIsolationPolicy::AccountSandboxV1) = manifest.runtime.isolation {
        ensure!(
            sdk >= semver::Version::new(3, 6, 0),
            "runtime isolation policies require Provider SDK 3.6.0 or newer"
        );
        ensure!(
            manifest
                .runtime
                .required_capabilities
                .contains(&RuntimeCapability::ProviderNativeCustodyV1),
            "account sandbox isolation requires provider.native-custody.v1"
        );
        ensure!(
            manifest.runtime.network == Some(RuntimeNetworkPolicy::Ipv4OnlyV1),
            "account sandbox isolation requires the ipv4_only_v1 network policy"
        );
        // A sidecar would run on the host as the Machine user, outside the
        // account's unit; nothing of an isolated Provider may.
        ensure!(
            manifest.runtime.sidecars.is_empty(),
            "account sandbox isolation forbids runtime sidecars"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StandardProviderSource, build_package};

    fn antigravity() -> ProviderManifest {
        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../../../plugins/antigravity/provider.json"))
                .unwrap();
        source.compile().unwrap()
    }

    #[test]
    fn account_sandbox_requires_sdk_3_6_custody_and_ipv4() {
        let manifest = antigravity();
        assert_eq!(
            manifest.runtime.isolation,
            Some(RuntimeIsolationPolicy::AccountSandboxV1)
        );
        build_package(manifest.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(RuntimeIsolationPolicy::AccountSandboxV1).unwrap(),
            serde_json::json!("account_sandbox_v1")
        );

        let mut old = manifest.clone();
        old.sdk_version = "3.5.0".into();
        let error = old.validate().unwrap_err().to_string();
        assert!(error.contains("Provider SDK 3.6.0"), "{error}");

        let mut no_custody = manifest.clone();
        no_custody
            .runtime
            .required_capabilities
            .remove(&RuntimeCapability::ProviderNativeCustodyV1);
        let error = no_custody.validate().unwrap_err().to_string();
        assert!(error.contains("native-custody"), "{error}");

        let mut no_network = manifest.clone();
        no_network.runtime.network = None;
        let error = no_network.validate().unwrap_err().to_string();
        assert!(error.contains("ipv4_only_v1"), "{error}");

        let mut sidecar = manifest;
        sidecar.runtime.sidecars.push(
            serde_json::from_value(serde_json::json!({
                "id": "helper",
                "component": {"kind": "provider_adapter", "slot": "antigravity"},
                "transport": {"kind": "loopback_http_v1", "listen_argument": "--listen",
                              "health_path": "/health", "timeout_ms": 1000},
            }))
            .unwrap(),
        );
        let error = sidecar.validate().unwrap_err().to_string();
        assert!(error.contains("forbids runtime sidecars"), "{error}");
    }

    #[test]
    fn absent_isolation_keeps_the_serialized_contract() {
        let mut manifest = antigravity();
        manifest.runtime.isolation = None;
        let value = serde_json::to_value(&manifest.runtime).unwrap();
        assert!(value.get("isolation").is_none());
        let unknown = serde_json::from_value::<RuntimeIsolationPolicy>(serde_json::json!(
            "account_sandbox_v2"
        ));
        assert!(unknown.is_err());
    }
}

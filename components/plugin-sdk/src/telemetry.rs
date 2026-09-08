//! Data-only telemetry destinations. Endpoints and secrets are host policy,
//! never part of a signed, publicly distributable package.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use cowboy_provider_sdk::PlatformTarget;
use serde::{Deserialize, Serialize};

pub const TELEMETRY_BACKEND_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryBackendContract {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub supported_platforms: Vec<PlatformTarget>,
    #[serde(
        default,
        deserialize_with = "present_route",
        skip_serializing_if = "Option::is_none"
    )]
    pub logs: Option<TelemetryRoute>,
    #[serde(
        default,
        deserialize_with = "present_route",
        skip_serializing_if = "Option::is_none"
    )]
    pub metrics: Option<TelemetryRoute>,
}

fn present_route<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<TelemetryRoute>, D::Error> {
    TelemetryRoute::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryRoute {
    pub encoding: TelemetryEncoding,
    /// Absolute path relative to the configured service base, not a URL.
    pub path: String,
    #[serde(default)]
    pub query: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryEncoding {
    JsonLines,
    PrometheusText,
}

impl TelemetryBackendContract {
    /// Validate the closed, bounded, executable-free backend contract.
    ///
    /// # Errors
    /// Rejects unsupported encodings, platforms, routes or private URL policy.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == TELEMETRY_BACKEND_SCHEMA_VERSION,
            "unsupported telemetry schema"
        );
        ensure!(
            !self.display_name.trim().is_empty()
                && self.display_name.len() <= 128
                && !self.display_name.chars().any(char::is_control),
            "invalid telemetry display name"
        );
        ensure!(
            !self.supported_platforms.is_empty()
                && self.supported_platforms.len() <= 4
                && self
                    .supported_platforms
                    .iter()
                    .collect::<BTreeSet<_>>()
                    .len()
                    == self.supported_platforms.len(),
            "invalid telemetry platform matrix"
        );
        ensure!(
            self.logs.is_some() || self.metrics.is_some(),
            "telemetry backend has no lanes"
        );
        for (route, expected) in [
            (&self.logs, TelemetryEncoding::JsonLines),
            (&self.metrics, TelemetryEncoding::PrometheusText),
        ] {
            let Some(route) = route else {
                continue;
            };
            ensure!(
                route.encoding == expected,
                "telemetry lane encoding mismatch"
            );
            ensure!(
                route.path.starts_with('/')
                    && !route.path.starts_with("//")
                    && route.path.len() <= 256
                    && route.path.split('/').skip(1).all(|part| !part.is_empty()
                        && part.bytes().all(
                            |byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                        )),
                "invalid telemetry ingestion path"
            );
            // Only declarative field-name mappings, not credentials or URL
            // fragments. New query semantics need a versioned SDK change.
            ensure!(route.query.len() <= 3, "too many telemetry query fields");
            for (key, value) in &route.query {
                ensure!(expected == TelemetryEncoding::JsonLines && matches!(key.as_str(), "_stream_fields" | "_time_field" | "_msg_field") && !value.is_empty() && value.len() <= 128 && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b',')), "invalid telemetry query mapping");
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_release(
    contract: &TelemetryBackendContract,
    release: &crate::PluginRelease,
) -> Result<()> {
    ensure!(
        release.host_bundle_digest.is_none(),
        "telemetry backend cannot contain host code"
    );
    ensure!(
        release.runtime_artifacts.len() == release.supported_platforms.len(),
        "telemetry runtime matrix mismatch"
    );
    let targets = release
        .runtime_artifacts
        .iter()
        .map(|entry| PlatformTarget {
            os: entry.os.clone(),
            architecture: entry.architecture.clone(),
        })
        .collect::<BTreeSet<_>>();
    let expected = contract
        .supported_platforms
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    ensure!(
        targets == expected
            && release
                .runtime_artifacts
                .iter()
                .all(|entry| entry.components.is_empty()),
        "telemetry backend cannot contain executable artifacts"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"id":"victoria","version":"1.0.0","display_name":"Victoria","supported_platforms":[{"os":"linux","architecture":"x86_64"}],"logs":{"encoding":"json_lines","path":"/insert/jsonline","query":{"_stream_fields":"component,platform"}}})
    }

    #[test]
    fn closed_routes_reject_credentials_urls_and_executables() {
        let valid = contract();
        serde_json::from_value::<TelemetryBackendContract>(valid.clone())
            .unwrap()
            .validate()
            .unwrap();
        for path in [
            "//attacker/insert",
            "https://attacker/insert",
            "/../insert",
            "/insert?token=x",
            "/insert%2fsecret",
            "/insert\\secret",
        ] {
            let mut value = valid.clone();
            value["logs"]["path"] = path.into();
            assert!(
                serde_json::from_value::<TelemetryBackendContract>(value)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        let mut value = valid.clone();
        value["logs"]["query"]["token"] = "private".into();
        assert!(
            serde_json::from_value::<TelemetryBackendContract>(value)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut value = valid;
        value["command"] = "curl".into();
        assert!(serde_json::from_value::<TelemetryBackendContract>(value).is_err());
    }

    #[test]
    fn telemetry_is_a_signed_capability_not_an_agent_or_executable_host() {
        let manifest: crate::PluginManifest = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "id": "victoria", "version": "1.0.0", "component_release": "2.8.0", "publisher": "fixture", "kind": "telemetry_backend", "entrypoint": "telemetry.json",
            "components": [{"id":"cowboy.plugin-contract","version":"1.7.0"},{"id":"cowboy.plugin-sdk","version":"1.7.0"}]
        })).unwrap();
        let payload =
            crate::PluginPayload::TelemetryBackend(serde_json::from_value(contract()).unwrap());
        let package = crate::PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            payload.clone(),
        )
        .unwrap();
        assert!(package.agent_provider().is_none());
        assert!(package.authentication_provider().is_none());
        assert!(package.validate_host_contract(None).is_ok());
        assert!(
            package
                .validate_host_contract(Some(&BTreeMap::new()))
                .is_err()
        );
        let mut too_old = manifest;
        too_old.components[1].version = "1.6.0".into();
        assert!(
            crate::PluginPackage::new(too_old.clone(), too_old.component_release.clone(), payload)
                .is_err()
        );
        let requirements = crate::PluginCompatibilityRequirements {
            plugin_sdk_version: Some("1.7.0".into()),
            manifest_schema: 1,
            package_schema: 1,
            release_schema: 1,
            plugin_kind: crate::PluginKind::TelemetryBackend,
            payload_schema: 1,
            host_bundle_schema: None,
            host_schema: None,
        };
        let mut inventory = crate::PluginContractInventory::current_machine(1);
        let target: PlatformTarget =
            serde_json::from_value(serde_json::json!({"os":"linux","architecture":"x86_64"}))
                .unwrap();
        assert!(
            inventory
                .compatibility_problem(
                    &requirements,
                    "victoria",
                    "1.0.0",
                    std::slice::from_ref(&target),
                    &target
                )
                .is_none()
        );
        inventory.plugin_sdk_version = "1.6.0".into();
        assert!(
            inventory
                .compatibility_problem(
                    &requirements,
                    "victoria",
                    "1.0.0",
                    std::slice::from_ref(&target),
                    &target
                )
                .is_some()
        );
    }
}

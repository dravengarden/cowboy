//! Generic first-party Cowboy plugin metadata.
//!
//! Agent Plugins embed a typed, data-only Provider capability payload and Zed
//! keeps its GPL-isolated adapter process. The generic contract exclusively
//! owns identity and lifecycle for both kinds.

#![warn(clippy::pedantic)]

use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{Result, ensure};
#[cfg(test)]
use cowboy_plugin_sdk::PluginKind;
use cowboy_plugin_sdk::PluginManifest;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentRegistry {
    schema_version: u16,
    active_release: String,
    releases: Vec<ComponentRelease>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentRelease {
    version: String,
    components: Vec<ComponentRecord>,
    plugins: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentRecord {
    id: String,
    version: String,
    publisher: String,
    sources: Vec<String>,
    digest: String,
}

const FIRST_PARTY_PLUGIN_SOURCES: [&str; 7] = [
    include_str!("../plugins/claude-code/plugin.json"),
    include_str!("../plugins/claude-deepseek/plugin.json"),
    include_str!("../plugins/codex/plugin.json"),
    include_str!("../plugins/codex-deepseek/plugin.json"),
    include_str!("../plugins/gemini/plugin.json"),
    include_str!("../plugins/grok/plugin.json"),
    include_str!("../plugins/zed/plugin.json"),
];
const COMPONENT_REGISTRY_SOURCE: &str = include_str!("../components/registry.json");

pub(crate) fn first_party_plugins() -> &'static [PluginManifest] {
    static PLUGINS: OnceLock<Vec<PluginManifest>> = OnceLock::new();
    PLUGINS.get_or_init(|| {
        FIRST_PARTY_PLUGIN_SOURCES
            .iter()
            .map(|source| {
                let manifest: PluginManifest =
                    serde_json::from_str(source).expect("first-party plugin manifest must parse");
                manifest
                    .validate()
                    .expect("first-party plugin manifest must validate");
                validate_against_active_release(&manifest)
                    .expect("first-party plugin must use the active component release");
                manifest
            })
            .collect()
    })
}

pub(crate) fn active_component_release() -> &'static str {
    component_registry().active_release.as_str()
}

fn component_registry() -> &'static ComponentRegistry {
    static REGISTRY: OnceLock<ComponentRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry: ComponentRegistry =
            serde_json::from_str(COMPONENT_REGISTRY_SOURCE).expect("component registry must parse");
        assert_eq!(
            registry.schema_version, 1,
            "component registry schema must be supported"
        );
        let active = registry
            .releases
            .last()
            .expect("component registry must have a release");
        assert_eq!(
            active.version, registry.active_release,
            "active component release must be last"
        );
        registry
    })
}

fn validate_against_active_release(manifest: &PluginManifest) -> Result<()> {
    let registry = component_registry();
    let release = registry.releases.last().expect("component release exists");
    ensure!(
        release.plugins.get(&manifest.id) == Some(&manifest.version),
        "plugin version does not match active component release"
    );
    let components: BTreeMap<_, _> = release
        .components
        .iter()
        .map(|component| {
            ensure!(
                !component.sources.is_empty(),
                "component has no source roots"
            );
            ensure!(
                !component.publisher.is_empty(),
                "component has no publisher"
            );
            ensure!(
                component.digest.starts_with("sha256:"),
                "component has invalid digest"
            );
            Ok((component.id.as_str(), component.version.as_str()))
        })
        .collect::<Result<_>>()?;
    for dependency in &manifest.components {
        ensure!(
            components.get(dependency.id.as_str()) == Some(&dependency.version.as_str()),
            "plugin component dependency does not match active release"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_first_party_integration_is_a_valid_plugin() {
        let plugins = first_party_plugins();
        assert_eq!(plugins.len(), 7);
        assert!(plugins.iter().any(|plugin| plugin.id == "zed"));
        assert_eq!(
            plugins
                .iter()
                .filter(|plugin| plugin.kind == PluginKind::AgentProvider)
                .count(),
            6
        );
    }

    #[test]
    fn component_dependencies_are_exact_and_unique() {
        let mut manifest = first_party_plugins()[0].clone();
        manifest.components.push(manifest.components[0].clone());
        assert!(manifest.validate().is_err());
        manifest.components.pop();
        manifest.components[0].version = "1.x".to_owned();
        assert!(manifest.validate().is_err());
    }
}

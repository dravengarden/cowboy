//! Generic first-party Cowboy plugin metadata.
//!
//! Agent Plugins embed a typed, data-only Provider capability payload and Zed
//! keeps its GPL-isolated adapter process. The generic contract exclusively
//! owns identity and lifecycle for both kinds.

#![warn(clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use anyhow::{Context as _, Result, ensure};
#[cfg(test)]
use cowboy_plugin_sdk::PluginKind;
use cowboy_plugin_sdk::{ComponentDependency, PluginManifest};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentRegistry {
    schema_version: u16,
    active_release: String,
    releases: Vec<ComponentRelease>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentRelease {
    version: String,
    components: Vec<ComponentRecord>,
    plugins: BTreeMap<String, String>,
    closure: Option<ComponentClosure>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentClosure {
    component_dependencies: BTreeMap<String, Vec<String>>,
    plugins: BTreeMap<String, PluginSourceSnapshot>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginSourceSnapshot {
    component_release: String,
    components: Vec<ComponentDependency>,
    source_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentRecord {
    id: String,
    version: String,
    publisher: String,
    sources: Vec<String>,
    digest: String,
    package: Option<ComponentPackage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentPackage {
    kind: ComponentPackageKind,
    name: String,
    manifest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ComponentPackageKind {
    Cargo,
    Npm,
}

const FIRST_PARTY_PLUGIN_SOURCES: &[&str] =
    include!(concat!(env!("OUT_DIR"), "/first_party_plugins.rs"));
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
                    .expect("first-party plugin must have an unchanged exact component closure");
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
        assert!(
            matches!(registry.schema_version, 2 | 3),
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
    validate_against_registry(manifest, component_registry())
}

fn validate_against_registry(
    manifest: &PluginManifest,
    registry: &ComponentRegistry,
) -> Result<()> {
    manifest.validate()?;
    ensure!(
        matches!(registry.schema_version, 2 | 3),
        "unsupported component registry schema"
    );
    let release = registry
        .releases
        .last()
        .context("component release is missing")?;
    ensure!(
        release.version == registry.active_release,
        "active component release must be last"
    );
    ensure!(
        (registry.schema_version == 3) == release.closure.is_some(),
        "registry schema and closure policy disagree"
    );
    let pinned = registry
        .releases
        .iter()
        .find(|candidate| candidate.version == manifest.component_release)
        .context("plugin references an unknown component release")?;
    ensure!(
        release.closure.is_some() || manifest.component_release == release.version,
        "plugin component release does not match active component release"
    );
    let minimum_version = release
        .plugins
        .get(&manifest.id)
        .context("plugin is absent from active component release")?;
    ensure!(
        semver::Version::parse(&manifest.version)? >= semver::Version::parse(minimum_version)?,
        "plugin version predates active component release"
    );
    let components = component_map(release)?;
    let pinned_components = component_map(pinned)?;
    for dependency in &manifest.components {
        ensure!(
            pinned_components
                .get(dependency.id.as_str())
                .map(|component| component.version.as_str())
                == Some(dependency.version.as_str()),
            "plugin component dependency does not match its declared release"
        );
    }
    let mut closure: BTreeSet<&str> = manifest
        .components
        .iter()
        .map(|dependency| dependency.id.as_str())
        .collect();
    if let Some(graph) = &release.closure {
        validate_dependency_graph(release, graph)?;
        let snapshot = graph
            .plugins
            .get(&manifest.id)
            .context("Plugin has no source snapshot")?;
        ensure!(
            valid_source_digest(&snapshot.source_digest),
            "invalid Plugin source digest"
        );
        if manifest.version == *minimum_version {
            ensure!(
                snapshot.component_release == manifest.component_release
                    && snapshot.components == manifest.components,
                "unchanged Plugin version has changed component binding"
            );
        }
        closure.clear();
        for dependency in &manifest.components {
            collect_dependencies(
                &dependency.id,
                &graph.component_dependencies,
                &mut BTreeSet::new(),
                &mut closure,
            )?;
        }
    }
    for id in closure {
        let active = components
            .get(id)
            .context("unknown component in Plugin closure")?;
        ensure!(
            pinned_components.get(id) == Some(active),
            "plugin has a changed component closure: {id}"
        );
    }
    Ok(())
}

fn validate_dependency_graph(release: &ComponentRelease, graph: &ComponentClosure) -> Result<()> {
    ensure!(
        graph
            .component_dependencies
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            == release
                .components
                .iter()
                .map(|component| component.id.as_str())
                .collect(),
        "component closure node set differs from registry"
    );
    ensure!(
        graph.plugins.keys().eq(release.plugins.keys()),
        "Plugin closure node set differs from registry"
    );
    // Validate the whole finite graph, even nodes this Plugin doesn't use.
    let mut checked = BTreeSet::new();
    for id in graph.component_dependencies.keys() {
        collect_dependencies(
            id,
            &graph.component_dependencies,
            &mut BTreeSet::new(),
            &mut checked,
        )?;
    }
    Ok(())
}

fn collect_dependencies<'a>(
    id: &'a str,
    graph: &'a BTreeMap<String, Vec<String>>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> Result<()> {
    ensure!(!visiting.contains(id), "component dependency cycle");
    if visited.contains(id) {
        return Ok(());
    }
    let edges = graph.get(id).context("missing component dependency node")?;
    ensure!(
        edges.iter().collect::<BTreeSet<_>>().len() == edges.len(),
        "duplicate component dependency"
    );
    visiting.insert(id);
    for edge in edges {
        collect_dependencies(edge, graph, visiting, visited)?;
    }
    visiting.remove(id);
    visited.insert(id);
    Ok(())
}

fn valid_source_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn component_map(release: &ComponentRelease) -> Result<BTreeMap<&str, &ComponentRecord>> {
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
                valid_source_digest(&component.digest),
                "component has invalid digest"
            );
            let package = component
                .package
                .as_ref()
                .context("active component has no distributable package")?;
            ensure!(!package.name.is_empty(), "component package name is empty");
            ensure!(
                !package.manifest.is_empty(),
                "component package manifest is empty"
            );
            match package.kind {
                ComponentPackageKind::Cargo | ComponentPackageKind::Npm => {}
            }
            Ok((component.id.as_str(), component))
        })
        .collect::<Result<_>>()?;
    ensure!(
        components.len() == release.components.len(),
        "duplicate component identity"
    );
    Ok(components)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pin migration tests to their immutable matrix. Later SDK/Plugin releases
    // are allowed to adopt the then-active matrix without rewriting these tests.
    fn migration_registry() -> ComponentRegistry {
        let mut registry = component_registry().clone();
        let index = registry
            .releases
            .iter()
            .position(|release| release.version == "3.1.0")
            .unwrap();
        registry.releases.truncate(index + 1);
        registry.active_release = "3.1.0".to_owned();
        registry
    }

    fn migration_manifest(id: &str) -> PluginManifest {
        let registry = migration_registry();
        let active = registry.releases.last().unwrap();
        let snapshot = &active.closure.as_ref().unwrap().plugins[id];
        let mut manifest = first_party_plugins()
            .iter()
            .find(|plugin| plugin.id == id)
            .unwrap()
            .clone();
        manifest.version = active.plugins[id].clone();
        manifest.component_release = snapshot.component_release.clone();
        manifest.components = snapshot.components.clone();
        manifest
    }

    #[test]
    fn every_first_party_integration_is_a_valid_plugin() {
        let plugins = first_party_plugins();
        assert!(plugins.len() >= 7);
        assert!(plugins.iter().any(|plugin| plugin.id == "zed"));
        assert!(
            plugins
                .iter()
                .filter(|plugin| plugin.kind == PluginKind::AgentProvider)
                .count()
                >= 6
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

    #[test]
    fn web_component_release_preserves_historical_plugin_bindings() {
        let registry = migration_registry();
        assert_eq!(registry.schema_version, 3);
        for id in registry.releases.last().unwrap().plugins.keys() {
            let manifest = migration_manifest(id);
            assert_ne!(manifest.component_release, registry.active_release);
            validate_against_registry(&manifest, &registry).unwrap();
        }
    }

    #[test]
    fn component_registry_reader_retains_schema_two_support() {
        let mut registry = migration_registry();
        registry
            .releases
            .retain(|release| release.closure.is_none());
        registry.active_release = registry.releases.last().unwrap().version.clone();
        registry.schema_version = 2;
        for id in registry.releases.last().unwrap().plugins.keys() {
            validate_against_registry(&migration_manifest(id), &registry).unwrap();
        }
    }

    #[test]
    fn component_closure_fences_changed_transitive_sdk_inputs() {
        let manifest = migration_manifest("zed");
        assert!(
            !manifest
                .components
                .iter()
                .any(|pin| pin.id == "cowboy.provider-sdk")
        );
        let mut registry = migration_registry();
        let active = registry.releases.last_mut().unwrap();
        active
            .components
            .iter_mut()
            .find(|component| component.id == "cowboy.provider-sdk")
            .unwrap()
            .digest = format!("sha256:{}", "0".repeat(64));
        assert!(
            validate_against_registry(&manifest, &registry)
                .unwrap_err()
                .to_string()
                .contains("changed component closure")
        );
    }

    #[test]
    fn component_closure_rejects_same_version_relabel_and_unknown_release() {
        let mut manifest = migration_manifest("codex");
        let registry = migration_registry();
        manifest.component_release = registry.active_release.clone();
        assert!(validate_against_registry(&manifest, &registry).is_err());
        manifest.version = "99.0.0".to_owned();
        validate_against_registry(&manifest, &registry).unwrap();
        manifest.component_release = "99.0.0".to_owned();
        assert!(validate_against_registry(&manifest, &registry).is_err());
    }

    #[test]
    fn component_closure_reader_rejects_cycles_missing_nodes_and_wrong_schema() {
        for case in ["cycle", "missing", "schema"] {
            let mut registry = migration_registry();
            let graph = &mut registry
                .releases
                .last_mut()
                .unwrap()
                .closure
                .as_mut()
                .unwrap()
                .component_dependencies;
            match case {
                "cycle" => {
                    graph.insert(
                        "cowboy.provider-sdk".to_owned(),
                        vec!["cowboy.plugin-sdk".to_owned()],
                    );
                }
                "missing" => {
                    graph.remove("cowboy.state-store");
                }
                _ => registry.schema_version = 2,
            }
            assert!(validate_against_registry(&migration_manifest("codex"), &registry).is_err());
        }
    }
}

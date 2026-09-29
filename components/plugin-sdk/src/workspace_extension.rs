//! Data-only repository resource declarations. The Machine owns all I/O.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use cowboy_provider_sdk::PlatformTarget;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceExtensionContract {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub dependencies: Vec<WorkspaceExtensionDependency>,
    pub supported_platforms: Vec<PlatformTarget>,
    pub connection: WorkspaceConnection,
    pub views: Vec<WorkspaceResourceView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceExtensionDependency {
    pub plugin_id: String,
    pub plugin_version: String,
    pub artifact_digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceConnection {
    GithubCli,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceResourceView {
    pub id: String,
    pub label: String,
    pub endpoint: String,
    pub items_pointer: String,
    pub detail_endpoint: String,
    pub filters: Vec<WorkspaceResourceFilter>,
    pub fields: WorkspaceResourceFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_if_present: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceResourceFilter {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceResourceFields {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub metadata: Vec<WorkspaceResourceMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceResourceMetadata {
    pub label: String,
    pub pointer: String,
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn pointer(value: &str) -> bool {
    value.is_empty()
        || (value.starts_with('/')
            && value.len() <= 256
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-~".contains(&b)))
}

fn endpoint(value: &str, detail: bool) -> bool {
    let mut rest = value.to_owned();
    for binding in ["{owner}", "{repo}", "{page}", "{filter}"] {
        rest = rest.replace(binding, "bound");
    }
    if detail {
        rest = rest.replace("{id}", "bound");
    }
    let mut parameters = std::collections::BTreeMap::new();
    if let Some((_, query)) = value.split_once('?') {
        for parameter in query.split('&') {
            let Some((key, value)) = parameter.split_once('=') else {
                return false;
            };
            if key.is_empty() || value.is_empty() || parameters.insert(key, value).is_some() {
                return false;
            }
        }
    }
    value.starts_with("repos/{owner}/{repo}/")
        && value.len() <= 512
        && !value.contains("..")
        && !value.contains("//")
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-?=&.".contains(&b))
        && (detail == value.contains("{id}"))
        && (detail
            || (parameters.get("page") == Some(&"{page}")
                && parameters.get("per_page") == Some(&"50")))
}

impl WorkspaceExtensionContract {
    /// Validate a finite, repository-confined resource declaration.
    ///
    /// # Errors
    /// Refuses unknown schema, invalid identities, unsafe endpoints and unbounded views.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported workspace extension schema"
        );
        crate::validate_id(&self.id, "extension id")?;
        crate::validate_version(&self.version, "extension version")?;
        ensure!(
            text(&self.display_name, 80) && text(&self.description, 512),
            "invalid extension presentation"
        );
        ensure!(
            !self.supported_platforms.is_empty() && self.supported_platforms.len() <= 8,
            "invalid extension platforms"
        );
        ensure!(
            self.supported_platforms
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == self.supported_platforms.len(),
            "duplicate extension platform"
        );
        ensure!(
            self.dependencies.len() <= 16,
            "too many extension dependencies"
        );
        let mut dependencies = BTreeSet::new();
        for dependency in &self.dependencies {
            crate::validate_id(&dependency.plugin_id, "dependency id")?;
            crate::validate_version(&dependency.plugin_version, "dependency version")?;
            crate::validate_digest(&dependency.artifact_digest, "dependency artifact digest")?;
            ensure!(
                dependency.plugin_id != self.id && dependencies.insert(&dependency.plugin_id),
                "cyclic or duplicate extension dependency"
            );
        }
        ensure!(
            !self.views.is_empty() && self.views.len() <= 16,
            "invalid extension views"
        );
        let mut ids = BTreeSet::new();
        for view in &self.views {
            crate::validate_id(&view.id, "view id")?;
            ensure!(
                ids.insert(&view.id) && text(&view.label, 80),
                "invalid resource view"
            );
            ensure!(
                endpoint(&view.endpoint, false) && endpoint(&view.detail_endpoint, true),
                "resource endpoint escapes repository or paging contract"
            );
            ensure!(
                pointer(&view.items_pointer)
                    && view.exclude_if_present.as_ref().is_none_or(|p| pointer(p)),
                "invalid collection projection"
            );
            let fields = &view.fields;
            for p in [&fields.id, &fields.title, &fields.url]
                .into_iter()
                .chain(fields.body.iter())
                .chain(fields.state.iter())
                .chain(fields.updated_at.iter())
            {
                ensure!(
                    !p.is_empty() && pointer(p),
                    "invalid resource field projection"
                );
            }
            ensure!(fields.metadata.len() <= 12, "too much resource metadata");
            for item in &fields.metadata {
                ensure!(
                    text(&item.label, 60) && pointer(&item.pointer),
                    "invalid metadata projection"
                );
            }
            ensure!(view.filters.len() <= 8, "too many resource filters");
            ensure!(
                view.endpoint.contains("{filter}") != view.filters.is_empty(),
                "filter binding mismatch"
            );
            let mut filters = BTreeSet::new();
            for filter in &view.filters {
                crate::validate_id(&filter.value, "filter value")?;
                ensure!(
                    filters.insert(&filter.value) && text(&filter.label, 60),
                    "invalid resource filter"
                );
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_release(
    contract: &WorkspaceExtensionContract,
    release: &crate::PluginRelease,
) -> Result<()> {
    ensure!(
        release.release_schema == crate::WORKSPACE_RELEASE_SCHEMA_VERSION,
        "workspace extensions require release schema 3"
    );
    ensure!(
        release.runtime_artifacts.len() == contract.supported_platforms.len(),
        "incomplete workspace extension platform matrix"
    );
    let targets = release
        .runtime_artifacts
        .iter()
        .map(|a| PlatformTarget {
            os: a.os.clone(),
            architecture: a.architecture.clone(),
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        targets == contract.supported_platforms.iter().cloned().collect()
            && targets.len() == release.runtime_artifacts.len(),
        "invalid workspace extension runtime matrix"
    );
    ensure!(
        release
            .runtime_artifacts
            .iter()
            .all(|a| a.components.is_empty()),
        "workspace extensions cannot ship executables"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_cannot_select_a_host_escape_the_repository_or_run_unbounded_queries() {
        assert!(endpoint(
            "repos/{owner}/{repo}/pulls?state={filter}&per_page=50&page={page}",
            false
        ));
        assert!(endpoint("repos/{owner}/{repo}/pulls/{id}", true));
        for value in [
            "https://evil.example/repos/{owner}/{repo}/pulls",
            "repos/{owner}/{repo}/../secrets/{id}",
            "repos/{owner}/{repo}/pulls/{unknown}",
            "repos/{owner}/{repo}/pulls?per_page=1000",
            "repos/{owner}/{repo}/pulls%2f../{id}",
        ] {
            assert!(!endpoint(value, false));
            assert!(!endpoint(value, true));
        }
        for query in [
            "per_page=500&page={page}",
            "per_page=50&page={page}&per_page=1000",
            "per_page=50&other_page={page}",
        ] {
            assert!(!endpoint(
                &format!("repos/{{owner}}/{{repo}}/issues?{query}"),
                false
            ));
        }
    }

    #[test]
    fn extensions_require_a_capable_reader_and_cannot_claim_executable_hosts() {
        let manifest: crate::PluginManifest = serde_json::from_value(serde_json::json!({
            "schema_version":1, "id":"fixture", "version":"0.1.0", "component_release":"3.30.0", "publisher":"fixture", "kind":"workspace_extension", "entrypoint":"contract.json",
            "components":[{"id":"cowboy.plugin-contract","version":"1.9.0"},{"id":"cowboy.plugin-sdk","version":"1.9.0"}]
        })).unwrap();
        let contract: WorkspaceExtensionContract = serde_json::from_value(serde_json::json!({
            "schema_version":1,"id":"fixture","version":"0.1.0","display_name":"Resources","description":"Fixture resources","dependencies":[],
            "supported_platforms":[{"os":"linux","architecture":"x86_64"}],"connection":"github_cli",
            "views":[{"id":"issues","label":"Issues","endpoint":"repos/{owner}/{repo}/issues?per_page=50&page={page}","detail_endpoint":"repos/{owner}/{repo}/issues/{id}","items_pointer":"","filters":[],"fields":{"id":"/number","title":"/title","url":"/html_url","metadata":[]}}]
        })).unwrap();
        let payload = crate::PluginPayload::WorkspaceExtension(contract.clone());
        let package = crate::PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            payload.clone(),
        )
        .unwrap();
        assert!(package.agent_provider().is_none());
        assert!(
            package
                .validate_host_contract(Some(&std::collections::BTreeMap::new()))
                .is_err()
        );
        let mut old = manifest;
        old.components[1].version = "1.8.1".into();
        assert!(
            crate::PluginPackage::new(old.clone(), old.component_release.clone(), payload).is_err()
        );
        let requirements = crate::PluginCompatibilityRequirements {
            plugin_sdk_version: Some("1.9.0".into()),
            manifest_schema: 1,
            package_schema: 1,
            release_schema: 3,
            plugin_kind: crate::PluginKind::WorkspaceExtension,
            payload_schema: 1,
            host_bundle_schema: None,
            host_schema: None,
        };
        requirements.validate().unwrap();
        let mut inventory = crate::PluginContractInventory::current_machine(1);
        let target = &contract.supported_platforms[0];
        assert!(
            inventory
                .compatibility_problem(
                    &requirements,
                    "fixture",
                    "0.1.0",
                    std::slice::from_ref(target),
                    target
                )
                .is_none()
        );
        inventory.max_release_schema = 2;
        assert!(
            inventory
                .compatibility_problem(
                    &requirements,
                    "fixture",
                    "0.1.0",
                    std::slice::from_ref(target),
                    target
                )
                .is_some()
        );
        let mut invalid = requirements;
        invalid.host_bundle_schema = Some(1);
        invalid.host_schema = Some(1);
        assert!(invalid.validate().is_err());
    }
}

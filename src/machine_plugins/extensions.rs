//! Exact installed extension/dependency resolution on the workspace's Machine.

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::MetadataExt as _;

use anyhow::{Context as _, Result, ensure};
use cowboy_plugin_sdk::{PluginKind, PluginPayload, WorkspaceExtensionContract};

use super::{MachinePluginStore, PluginInstallationState, PluginInventory};
use crate::workspace_extensions::{
    Extension, Failure, Identity, Operation, Request, Response, runtime,
};

static READS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

#[derive(Debug, PartialEq, Eq)]
struct InstalledDependency {
    inventory: PluginInventory,
    // Legacy installation slots have no durable operation revision. Preserve
    // their active-link identity as well, so remove/reactivate is still fenced.
    active_link: (u64, u64, i64, i64),
}

impl MachinePluginStore {
    fn extension_dependency_stamp(
        &self,
        inventory: PluginInventory,
    ) -> Result<InstalledDependency> {
        let metadata = self
            .plugin_root(&inventory.plugin_id)
            .join("active")
            .symlink_metadata()?;
        Ok(InstalledDependency {
            inventory,
            active_link: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
    fn extension_dependencies(
        &self,
        contract: &WorkspaceExtensionContract,
    ) -> Result<BTreeMap<String, InstalledDependency>> {
        fn visit(
            store: &MachinePluginStore,
            contract: &WorkspaceExtensionContract,
            visiting: &mut BTreeSet<String>,
            resolved: &mut BTreeMap<String, InstalledDependency>,
        ) -> Result<()> {
            ensure!(
                visiting.insert(contract.id.clone()),
                "extension dependency cycle"
            );
            ensure!(
                visiting.len() + resolved.len() <= 64,
                "extension dependency budget exceeded"
            );
            for dependency in &contract.dependencies {
                ensure!(
                    !visiting.contains(&dependency.plugin_id),
                    "extension dependency cycle"
                );
                store.operations.ensure_unfenced(&dependency.plugin_id)?;
                let installed = store
                    .inventory_one(&dependency.plugin_id)?
                    .context("extension dependency is not installed")?;
                ensure!(
                    installed.state == PluginInstallationState::Active
                        && installed.plugin_version == dependency.plugin_version
                        && installed.generation_digest == dependency.artifact_digest,
                    "extension dependency release unavailable"
                );
                if let Some(previous) = resolved.get(&dependency.plugin_id) {
                    ensure!(
                        previous.inventory == installed,
                        "inconsistent extension dependency"
                    );
                    continue;
                }
                let (package, _, _) = store.verified_plugin_generation(
                    &installed.plugin_id,
                    &installed.generation_digest,
                )?;
                if let PluginPayload::WorkspaceExtension(next) = &package.payload {
                    visit(store, next, visiting, resolved)?;
                }
                resolved.insert(
                    dependency.plugin_id.clone(),
                    store.extension_dependency_stamp(installed)?,
                );
            }
            visiting.remove(&contract.id);
            Ok(())
        }
        let mut resolved = BTreeMap::new();
        visit(self, contract, &mut BTreeSet::new(), &mut resolved)?;
        Ok(resolved)
    }

    pub(crate) fn validate_extension_dependencies(
        &self,
        package: &cowboy_plugin_sdk::PluginPackage,
    ) -> Result<()> {
        if let PluginPayload::WorkspaceExtension(contract) = &package.payload {
            self.extension_dependencies(contract)?;
        }
        Ok(())
    }

    fn extension_contract(
        &self,
        identity: &Identity,
    ) -> Result<(
        WorkspaceExtensionContract,
        BTreeMap<String, InstalledDependency>,
    )> {
        super::validate_plugin_id(&identity.plugin_id)?;
        self.operations.ensure_unfenced(&identity.plugin_id)?;
        let installed = self
            .inventory_one(&identity.plugin_id)?
            .context("extension is not installed")?;
        ensure!(
            installed.plugin_version == identity.plugin_version
                && installed.generation_digest == identity.generation_digest
                && installed.state == PluginInstallationState::Active,
            "extension generation changed"
        );
        let (package, _, _) =
            self.verified_plugin_generation(&identity.plugin_id, &identity.generation_digest)?;
        let PluginPayload::WorkspaceExtension(contract) = package.payload else {
            anyhow::bail!("not a workspace extension");
        };
        let mut dependencies = self.extension_dependencies(&contract)?;
        dependencies.insert(
            identity.plugin_id.clone(),
            self.extension_dependency_stamp(installed)?,
        );
        Ok((contract, dependencies))
    }

    pub(crate) async fn extension_request(&self, request: Request) -> Response {
        let Ok(_permit) = READS.try_acquire() else {
            return Response::Unavailable {
                code: Failure::Busy,
            };
        };
        match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.extension_request_inner(request),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(code)) => Response::Unavailable { code },
            Err(_) => Response::Unavailable {
                code: Failure::RequestFailed,
            },
        }
    }

    async fn extension_request_inner(&self, request: Request) -> Result<Response, Failure> {
        let remotes = runtime::remotes(std::path::Path::new(&request.root))
            .await
            .unwrap_or_default();
        match request.operation {
            Operation::Inventory => {
                let _guard = self.lifecycle.lock().await;
                let inventory = self.inventory().map_err(|_| Failure::ExtensionChanged)?;
                let mut extensions = Vec::new();
                for installed in inventory
                    .into_iter()
                    .filter(|i| i.plugin_kind == PluginKind::WorkspaceExtension)
                    .take(128)
                {
                    let identity = Identity {
                        plugin_id: installed.plugin_id,
                        plugin_version: installed.plugin_version,
                        generation_digest: installed.generation_digest,
                    };
                    let (package, _, _) = self
                        .verified_plugin_generation(
                            &identity.plugin_id,
                            &identity.generation_digest,
                        )
                        .map_err(|_| Failure::ExtensionChanged)?;
                    if let PluginPayload::WorkspaceExtension(contract) = package.payload {
                        let available = self.extension_contract(&identity).is_ok();
                        extensions.push(Extension::from_contract(identity, &contract, available));
                    }
                }
                Ok(Response::Inventory {
                    extensions,
                    remotes,
                })
            }
            Operation::Read {
                identity,
                remote,
                view,
                item,
                filter,
                page,
                review,
            } => {
                let remote = remotes
                    .into_iter()
                    .find(|r| r.name == remote)
                    .ok_or(Failure::RepositoryUnavailable)?;
                let (contract, dependencies) = {
                    let _guard = self.lifecycle.lock().await;
                    self.extension_contract(&identity)
                        .map_err(|_| Failure::DependencyUnavailable)?
                };
                let view = contract
                    .views
                    .iter()
                    .find(|v| v.id == view)
                    .ok_or(Failure::InvalidRequest)?;
                let response = if let Some(review) = review {
                    if view.review.is_none() || filter.is_some() {
                        return Err(Failure::InvalidRequest);
                    }
                    runtime::read_review(
                        &remote,
                        item.as_deref().ok_or(Failure::InvalidRequest)?,
                        &review,
                        page,
                    )
                    .await?
                } else {
                    runtime::read(&remote, view, item.as_deref(), filter.as_deref(), page).await?
                };
                // Do not hold the installation lock across a Git subprocess.
                let current_remotes = runtime::remotes(std::path::Path::new(&request.root)).await?;
                let _guard = self.lifecycle.lock().await;
                let (_, current) = self
                    .extension_contract(&identity)
                    .map_err(|_| Failure::ExtensionChanged)?;
                if current != dependencies {
                    return Err(Failure::ExtensionChanged);
                }
                // Git remotes are mutable independently of the Plugin graph.
                if !current_remotes.contains(&remote) {
                    return Err(Failure::RepositoryUnavailable);
                }
                Ok(response)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::{DesiredPlugin, Platform};
    use base64::Engine as _;
    use cowboy_plugin_sdk::{PluginPackage, WorkspaceExtensionDependency};

    fn release(
        publisher: &crate::machine_auth::MachineIdentity,
        id: &str,
        version: &str,
        dependencies: Vec<WorkspaceExtensionDependency>,
    ) -> DesiredPlugin {
        let mut manifest: cowboy_plugin_sdk::PluginManifest =
            serde_json::from_str(include_str!("../../plugins/github/plugin.json")).unwrap();
        let mut contract: WorkspaceExtensionContract =
            serde_json::from_str(include_str!("../../plugins/github/contract.json")).unwrap();
        manifest.id = id.into();
        manifest.version = version.into();
        contract.id = id.into();
        contract.version = version.into();
        contract.dependencies = dependencies;
        let package = PluginPackage::new(
            manifest.clone(),
            manifest.component_release.clone(),
            PluginPayload::WorkspaceExtension(contract),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let mut desired = super::super::tests::telemetry_release(publisher, "1.0.0");
        desired.release.release_schema = 3;
        desired.release.plugin_id = id.into();
        desired.release.plugin_version = version.into();
        desired.release.plugin_kind = PluginKind::WorkspaceExtension;
        desired.release.component_release = manifest.component_release;
        desired.release.package_digest = PluginPackage::artifact_digest(&bytes);
        desired.release.contract_fingerprint = package.contract_fingerprint;
        desired.release.artifact_digest = desired.release.computed_artifact_digest().unwrap();
        desired.release.signature = publisher
            .sign_namespaced(
                cowboy_plugin_sdk::PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
                &desired.release.proof(),
            )
            .unwrap();
        desired.package_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        desired
    }

    #[tokio::test]
    async fn signed_extensions_use_normal_lifecycle_and_fence_exact_dependency_changes() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-extension-lifecycle-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher")).unwrap();
        let store =
            MachinePluginStore::new(&root.join("machine"), Platform::Linux, "x86_64".into())
                .unwrap();
        let dependency = release(&publisher, "dependency", "0.1.0", vec![]);
        let edge = WorkspaceExtensionDependency {
            plugin_id: "dependency".into(),
            plugin_version: "0.1.0".into(),
            artifact_digest: dependency.release.artifact_digest.clone(),
        };
        let extension = release(&publisher, "github", "0.1.0", vec![edge.clone()]);
        assert!(store.install(&extension).await.is_err());
        assert!(store.inventory().unwrap().is_empty());
        store.install(&dependency).await.unwrap();
        let installed = store.install(&extension).await.unwrap();
        assert_eq!(installed.plugin_kind, PluginKind::WorkspaceExtension);
        assert!(installed.auth_generation.is_none());
        let identity = Identity {
            plugin_id: "github".into(),
            plugin_version: "0.1.0".into(),
            generation_digest: installed.generation_digest.clone(),
        };
        let (_, original) = store.extension_contract(&identity).unwrap();
        // Same-byte reinstall changes the installation incarnation (ABA).
        store.install(&dependency).await.unwrap();
        let (_, current) = store.extension_contract(&identity).unwrap();
        assert_ne!(original, current);
        let wrong = release(
            &publisher,
            "github",
            "0.1.1",
            vec![WorkspaceExtensionDependency {
                artifact_digest: format!("sha256:{}", "f".repeat(64)),
                ..edge
            }],
        );
        assert!(store.install(&wrong).await.is_err());
        assert!(store.extension_contract(&identity).is_ok());
        // An update introducing a transitive cycle is refused before activation.
        let cycle = release(
            &publisher,
            "dependency",
            "0.1.1",
            vec![WorkspaceExtensionDependency {
                plugin_id: "github".into(),
                plugin_version: "0.1.0".into(),
                artifact_digest: installed.generation_digest.clone(),
            }],
        );
        assert!(store.install(&cycle).await.is_err());
        store
            .uninstall("dependency", &dependency.release.artifact_digest)
            .await
            .unwrap();
        assert!(store.extension_contract(&identity).is_err());
        store
            .uninstall("github", &installed.generation_digest)
            .await
            .unwrap();
        assert!(
            store
                .reactivate("github", &installed.generation_digest)
                .await
                .is_err()
        );
        store
            .reactivate("dependency", &dependency.release.artifact_digest)
            .await
            .unwrap();
        store
            .reactivate("github", &installed.generation_digest)
            .await
            .unwrap();
        assert!(store.extension_contract(&identity).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}

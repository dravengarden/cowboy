//! An accepted release's continuous lifetime, not a serializable grant.
//! Successful refreshes preserve identity only for an unchanged full signed
//! envelope. Removing then restoring identical bytes creates a new lifetime.

use super::{CatalogArtifact, DesiredPlugin, PluginCatalog};
use anyhow::{Context as _, Result};
use std::sync::{Arc, Weak};

pub(super) struct ReleaseObservation {
    key: (String, String, String),
    incarnation: Weak<()>,
}

impl ReleaseObservation {
    pub(super) fn capture(artifact: &CatalogArtifact) -> Self {
        let release = &artifact.desired.release;
        Self {
            key: (
                release.plugin_id.clone(),
                release.plugin_version.clone(),
                release.artifact_digest.clone(),
            ),
            incarnation: Arc::downgrade(&artifact.incarnation),
        }
    }

    pub(super) fn current(&self, catalog: &PluginCatalog) -> bool {
        catalog
            .state
            .read()
            .external
            .get(&self.key)
            .is_some_and(|artifact| {
                // A different Catalog owner, an intervening removal, or a changed
                // envelope cannot regain this identity. Retained runtime/snapshot
                // Arcs do not keep an old observation current in the live Catalog.
                self.incarnation
                    .ptr_eq(&Arc::downgrade(&artifact.incarnation))
            })
    }
}

/// Only exact, signature-checked Catalog lookup can construct this result.
/// No Clone/serde: wire data and durable receipts cannot mint a live lease.
/// Callers still need purpose-bound authority, budget, connection and CAS.
pub(crate) struct VerifiedPluginRelease {
    desired: DesiredPlugin,
    observation: ReleaseObservation,
}

impl VerifiedPluginRelease {
    pub(crate) fn desired(&self) -> &DesiredPlugin {
        &self.desired
    }

    pub(crate) fn current(&self, catalog: &PluginCatalog) -> bool {
        self.observation.current(catalog)
    }
}

impl PluginCatalog {
    pub(crate) fn resolve_verified_exact(
        &self,
        plugin_id: &str,
        version: &str,
        digest: &str,
    ) -> Result<VerifiedPluginRelease> {
        let snapshot = self.state.read();
        let artifact = snapshot
            .external
            .get(&(plugin_id.to_owned(), version.to_owned(), digest.to_owned()))
            .context("exact Plugin release is not in the trusted Catalog")?;
        Ok(VerifiedPluginRelease {
            desired: artifact.desired.clone(),
            observation: ReleaseObservation::capture(artifact),
        })
    }
}

//! Read-only validation for Provider releases created before the generic
//! Plugin envelope became Cowboy's sole installable extension format.
//!
//! This compatibility contract exists only so already-installed Provider
//! generations remain schedulable during the Plugin rollout. Legacy releases
//! are never offered to the generic install lifecycle.

use anyhow::{Result, ensure};
use cowboy_provider_sdk::{
    AgentRuntimeBinding, PlatformRuntimeArtifacts, PlatformTarget, ProviderPackage,
    RUNTIME_BINDING_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::machine_auth::LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacyProviderRelease {
    pub release_schema: u16,
    pub provider_id: String,
    pub provider_version: String,
    pub package_digest: String,
    pub artifact_digest: String,
    pub artifact_url: String,
    pub publisher: String,
    pub contract_fingerprint: String,
    pub signature: String,
    pub supported_platforms: Vec<PlatformTarget>,
    pub runtime_artifacts: Vec<PlatformRuntimeArtifacts>,
}

impl LegacyProviderRelease {
    pub(crate) fn proof(&self) -> Result<Vec<u8>> {
        let runtime_artifacts = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&(
                &self.supported_platforms,
                &self.runtime_artifacts,
            ))?)
        );
        let fields = [
            self.provider_id.as_str(),
            self.provider_version.as_str(),
            self.package_digest.as_str(),
            self.artifact_digest.as_str(),
            self.publisher.as_str(),
            self.contract_fingerprint.as_str(),
            runtime_artifacts.as_str(),
        ];
        let mut proof = format!("{LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE}\n").into_bytes();
        for field in fields {
            proof.extend_from_slice(field.len().to_string().as_bytes());
            proof.push(b':');
            proof.extend_from_slice(field.as_bytes());
            proof.push(b'\n');
        }
        Ok(proof)
    }

    pub(crate) fn validate_and_project(
        &self,
        bytes: &[u8],
    ) -> Result<(ProviderPackage, AgentRuntimeBinding)> {
        ensure!(
            self.release_schema == 2,
            "unsupported legacy Provider release schema"
        );
        ensure!(
            self.package_digest == ProviderPackage::artifact_digest(bytes),
            "legacy Provider release package digest mismatch"
        );
        ensure!(
            self.artifact_url.starts_with("https://"),
            "legacy Provider artifact URL must use HTTPS"
        );
        ensure!(
            !self.signature.trim().is_empty(),
            "legacy Provider release is unsigned"
        );
        let package = ProviderPackage::from_historical_bytes(bytes)?;
        ensure!(
            self.provider_id == package.manifest.id,
            "legacy Provider release id mismatch"
        );
        ensure!(
            self.provider_version == package.manifest.version,
            "legacy Provider release version mismatch"
        );
        ensure!(
            self.publisher == package.manifest.publisher,
            "legacy Provider publisher mismatch"
        );
        ensure!(
            self.contract_fingerprint == package.contract_fingerprint,
            "legacy Provider contract mismatch"
        );
        let expected_digest = cowboy_provider_sdk::fingerprint_json(&serde_json::json!({
            "release_schema": self.release_schema,
            "provider_id": self.provider_id,
            "provider_version": self.provider_version,
            "package_digest": self.package_digest,
            "publisher": self.publisher,
            "contract_fingerprint": self.contract_fingerprint,
            "supported_platforms": self.supported_platforms,
            "runtime_artifacts": self.runtime_artifacts,
        }))?;
        ensure!(
            self.artifact_digest == expected_digest,
            "legacy Provider release composite artifact digest mismatch"
        );
        let mut binding = AgentRuntimeBinding {
            binding_schema: RUNTIME_BINDING_SCHEMA_VERSION,
            provider_id: self.provider_id.clone(),
            provider_version: self.provider_version.clone(),
            package_digest: self.package_digest.clone(),
            artifact_digest: String::new(),
            publisher: self.publisher.clone(),
            contract_fingerprint: self.contract_fingerprint.clone(),
            supported_platforms: self.supported_platforms.clone(),
            runtime_artifacts: self.runtime_artifacts.clone(),
        };
        binding.artifact_digest = binding.computed_artifact_digest()?;
        binding.validate_for(&package)?;
        Ok((package, binding))
    }
}

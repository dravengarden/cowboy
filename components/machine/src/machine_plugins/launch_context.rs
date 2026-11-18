//! The resolved launch inputs of one Provider package for a worker.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use super::{
    MachinePluginStore, ProviderPackage, account_sandbox, ensure_within,
    set_directory_chain_permissions,
};

#[derive(Debug, Clone)]
pub struct ProviderLaunchContext {
    pub package_path: PathBuf,
    pub command: String,
    pub version: String,
    pub behavior: cowboy_provider_sdk::ProviderBehaviorContract,
    pub environment: BTreeMap<String, String>,
    pub remove_environment: BTreeSet<String>,
    pub remove_environment_prefixes: BTreeSet<String>,
    pub home: Option<PathBuf>,
    pub execution_jsonrpc: bool,
    pub mcp_bindings: bool,
    pub network: Option<cowboy_provider_sdk::RuntimeNetworkPolicy>,
    /// The daemon launch of a Provider that runs only in the account sandbox.
    pub account_sandbox: Option<crate::account_sandbox::Launch>,
}

impl MachinePluginStore {
    /// The account a launch runs on: the daemon launch of an account-sandboxed
    /// Provider, or else the authenticated home of a Provider that requires one.
    pub(super) fn launch_account(
        &self,
        package: &ProviderPackage,
        auth_generation: Option<u64>,
        generation_digest: &str,
    ) -> Result<(Option<crate::account_sandbox::Launch>, Option<PathBuf>)> {
        if account_sandbox::sandboxed(package) {
            let launch =
                self.account_sandbox_launch(package, generation_digest, auth_generation)?;
            return Ok((Some(launch), None));
        }
        if !package.manifest.authentication.required {
            return Ok((None, None));
        }
        let generation = auth_generation.context("session has no Provider auth generation")?;
        let home = self.prepare_launch_auth_home(
            &package.manifest.id,
            package,
            generation,
            generation_digest,
        )?;
        Ok((None, Some(home)))
    }

    /// Directory of each projected credential file in the one runtime
    /// projection that owns the credential bytes. Historical generations link
    /// their copies to it, so this is the single directory a Provider CLI may
    /// use to serialize its own token refresh across live sessions.
    pub(super) fn credential_directories(
        &self,
        package: &ProviderPackage,
        native_home: Option<&Path>,
    ) -> Result<BTreeMap<String, String>> {
        let auth = &package.manifest.authentication;
        // A sandboxed Provider's credentials live only in the daemon.
        if !auth.required || auth.credential_files.is_empty() || account_sandbox::sandboxed(package)
        {
            return Ok(BTreeMap::new());
        }
        if let Some(native_home) = native_home {
            return Self::native_credential_directories(package, native_home);
        }
        let Some(canonical) = self.canonical_runtime_projection(package)? else {
            return Ok(BTreeMap::new());
        };
        let home = canonical.join("home");
        let mut directories = BTreeMap::new();
        for credential in &auth.credential_files {
            let path = home.join(&credential.relative_path);
            ensure_within(&home, &path)?;
            let directory = path
                .parent()
                .context("projected credential has no directory")?;
            fs::create_dir_all(directory)?;
            set_directory_chain_permissions(&home, directory)?;
            directories.insert(
                credential.bundle_key.clone(),
                directory.to_string_lossy().into_owned(),
            );
        }
        Ok(directories)
    }
}

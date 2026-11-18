//! Service sign-in candidates: the credentials a temporary login (or an
//! entered secret) produced, exported once for the Cowboy Service to commit.
//! The Machine is never the source of auth generation truth.
//!
//! The default account signs in with the legacy portable contract. A named
//! account signs in only with a native identity method: the Machine reads
//! the identity from the fresh home with the signed recipe and stores the
//! credential under the method's portable twin, so the account's vault
//! holds exactly the portable credential the default account would.
use super::*;

/// The method a login runs and the portable method its bundle is stored as.
fn sign_in_method(
    account: &crate::provider_account::ProviderAccountId,
    package: &ProviderPackage,
    method_id: &str,
) -> Result<(cowboy_provider_sdk::AuthMethod, String)> {
    if account.is_default() {
        crate::provider_account::portable_credentials_allowed(account, &package.manifest.id)
            .map_err(anyhow::Error::msg)?;
        let method = portable_contract::contract(package)
            .methods
            .iter()
            .find(|method| method.id == method_id)
            .cloned()
            .context("authentication method is not declared by the active Provider")?;
        let id = method.id.clone();
        return Ok((method, id));
    }
    crate::provider_account::portable_credentials_allowed(account, &package.manifest.id)
        .map_err(anyhow::Error::msg)?;
    let auth = &package.manifest.authentication;
    let method = auth
        .methods
        .iter()
        .find(|method| method.id == method_id)
        .context("authentication method is not declared by the active Provider")?;
    let twin = portable_contract::portable_twin(auth, method)
        .context("named sign-in requires a native identity method with a portable twin")?;
    Ok((method.clone(), twin.id.clone()))
}

/// Read the credential files `method` requires from `home` into a bundle of
/// the portable method `portable_method`, validated against the contract.
pub(super) fn read_portable_bundle(
    package: &ProviderPackage,
    method: &cowboy_provider_sdk::AuthMethod,
    portable_method: &str,
    home: &Path,
) -> Result<PortableCredentialBundle> {
    let portable_auth = portable_contract::contract(package);
    let auth = &*portable_auth;
    let mut values = BTreeMap::new();
    let mut total = 0_usize;
    for credential in &auth.credential_files {
        if !method.required_bundle_keys.contains(&credential.bundle_key) {
            continue;
        }
        match native_identity::read_login_file(
            home,
            &credential.relative_path,
            MAX_CREDENTIAL_VALUE_BYTES.min(MAX_CREDENTIAL_BUNDLE_BYTES.saturating_sub(total)),
        ) {
            Ok(bytes) => {
                ensure!(
                    bytes.len() <= MAX_CREDENTIAL_VALUE_BYTES,
                    "credential value is too large"
                );
                total = total.saturating_add(bytes.len());
                ensure!(
                    total <= MAX_CREDENTIAL_BUNDLE_BYTES,
                    "credential bundle is too large"
                );
                values.insert(
                    credential.bundle_key.clone(),
                    base64::engine::general_purpose::STANDARD.encode(bytes),
                );
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                ensure!(
                    !credential.required,
                    "login completed without required credential {}",
                    credential.bundle_key
                );
            }
            Err(error) => {
                return Err(error).context("reading native login credentials");
            }
        }
    }
    // Native login exports only its isolated output files. Environment
    // credentials enter through the explicit SecretInput executor instead
    // of being borrowed from the Machine process after a login succeeds.
    let bundle = PortableCredentialBundle {
        portable_schema: auth.portable_schema.clone(),
        method_id: portable_method.to_owned(),
        values,
    };
    validate_portable_bundle(auth, &bundle)?;
    Ok(bundle)
}

impl MachinePluginStore {
    pub fn authentication_method(
        &self,
        provider_id: &str,
        method_id: &str,
    ) -> Result<cowboy_provider_sdk::AuthMethod> {
        let (package, _) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        sign_in_method(&self.account_id, &package, method_id).map(|(method, _)| method)
    }

    pub fn authentication_component_command(
        &self,
        provider_id: &str,
        component: &cowboy_provider_sdk::AuthComponent,
    ) -> Result<PathBuf> {
        let (package, digest) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        let payload = matching_payload(&package, &self.platform, &self.architecture)?;
        let requirement = payload
            .private_components
            .iter()
            .find(|candidate| candidate.kind == component.kind && candidate.slot == component.slot)
            .context("authentication component is not exported by the active Provider")?;
        let (_, package_path) = self.package_for_generation(provider_id, &digest)?;
        runtime_command(&package_path, &requirement.command)
    }

    /// Export credentials created by a temporary login executor into the
    /// Provider-declared portable bundle. The caller sends this once to the
    /// Cowboy Service.
    pub fn export_auth_candidate(
        &self,
        provider_id: &str,
        method_id: &str,
        request_id: &str,
    ) -> Result<ExportedAuthCandidate> {
        crate::provider_account::portable_credentials_allowed(&self.account_id, provider_id)
            .map_err(anyhow::Error::msg)?;
        validate_auth_candidate_id(request_id)?;
        let home = self
            .auth_candidate_root(provider_id)
            .join(request_id)
            .join("home");
        let (package, generation_digest) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        let (method, portable_method) = sign_in_method(&self.account_id, &package, method_id)?;
        ensure_within(&self.auth_candidate_root(provider_id), &home)?;
        ensure!(
            self.account_id.is_default() || method.native_identity.is_some(),
            "named sign-in requires a signed identity recipe"
        );
        let native_identity = method
            .native_identity
            .as_ref()
            .map(|recipe| native_identity::read(&home, recipe, &self.account_id))
            .transpose()?;
        let bundle = read_portable_bundle(&package, &method, &portable_method, &home)?;
        Ok(ExportedAuthCandidate {
            provider_version: package.manifest.version.clone(),
            native_identity,
            generation_digest,
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            bundle,
        })
    }

    pub fn auth_candidate_from_secret(
        &self,
        provider_id: &str,
        method_id: &str,
        secret: &str,
    ) -> Result<ExportedAuthCandidate> {
        crate::provider_account::portable_credentials_allowed(&self.account_id, provider_id)
            .map_err(anyhow::Error::msg)?;
        let secret = secret.trim();
        ensure!(!secret.is_empty(), "authentication secret is empty");
        ensure!(
            secret.len() <= MAX_CREDENTIAL_VALUE_BYTES && !secret.contains(['\0', '\r', '\n']),
            "authentication secret is invalid"
        );
        let (package, generation_digest) = self
            .active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        let method = package
            .manifest
            .authentication
            .methods
            .iter()
            .find(|method| method.id == method_id)
            .context("authentication method is not declared by the active Provider")?;
        let cowboy_provider_sdk::AuthExecutor::SecretInputV1 { bundle_key, .. } = &method.executor
        else {
            bail!("authentication method does not accept a secret")
        };
        let bundle = PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: method_id.to_owned(),
            values: BTreeMap::from([(
                bundle_key.clone(),
                base64::engine::general_purpose::STANDARD.encode(secret.as_bytes()),
            )]),
        };
        validate_portable_bundle(&portable_contract::contract(&package), &bundle)?;
        Ok(ExportedAuthCandidate {
            provider_version: package.manifest.version.clone(),
            native_identity: None,
            generation_digest,
            auth_contract_fingerprint: package
                .manifest
                .compatibility
                .auth_contract_fingerprint
                .clone(),
            bundle,
        })
    }

    pub fn prepare_auth_candidate_home(
        &self,
        provider_id: &str,
        request_id: &str,
    ) -> Result<PathBuf> {
        crate::provider_account::portable_credentials_allowed(&self.account_id, provider_id)
            .map_err(anyhow::Error::msg)?;
        self.active_package(provider_id)?
            .context("Provider must be installed before Service authentication")?;
        self.create_auth_candidate_home(provider_id, request_id)
    }
}

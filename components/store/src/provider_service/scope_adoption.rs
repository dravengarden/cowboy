//! A Provider installed after its scope's secret was entered inherits it.
use super::*;

impl ProviderAuthService {
    /// Extend the current shared credential of one public authentication scope
    /// to scope members that have never held Service authentication, such as a
    /// Provider installed after the user entered the scope's secret.
    ///
    /// The default Service vault and named portable account vaults
    /// participate; Machine-bound native grants never do. The source is the
    /// scope's newest generation and must be a ready credential: a newer
    /// logout or failure tombstone means there is nothing to inherit. A
    /// member with any durable state, including a tombstone, is never
    /// overwritten. Adopted entries keep the source generation, so shared
    /// refresh and logout stay one lineage.
    pub fn adopt_shared_scope(
        &self,
        packages: &[ProviderPackage],
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        let scope = self.root_write_scope(packages, None)?;
        let packages = &*scope;
        if self.recipient_key.is_some() || packages.len() < 2 {
            return Ok(Vec::new());
        }
        let mut providers = self.providers.write();
        if packages
            .iter()
            .all(|package| providers.contains_key(&package.manifest.id))
        {
            return Ok(Vec::new());
        }
        let generation = maximum_auth_generation(packages, &providers);
        let Some(source) = packages
            .iter()
            .filter_map(|package| providers.get(&package.manifest.id))
            .find(|stored| stored.auth_generation == generation)
            .filter(|stored| {
                generation > 0
                    && stored.action == ProviderAuthAction::Apply
                    && stored.authentication_state == ServiceAuthenticationState::Ready
                    && stored.promotion_commitment.is_none()
            })
            .cloned()
        else {
            return Ok(Vec::new());
        };
        // Every member at this generation must agree; a partially written
        // scope is not a single credential to extend.
        ensure!(
            packages
                .iter()
                .filter_map(|package| providers.get(&package.manifest.id))
                .filter(|stored| stored.auth_generation == generation)
                .all(|stored| stored.action == ProviderAuthAction::Apply
                    && stored.authentication_state == ServiceAuthenticationState::Ready),
            "shared authentication scope is inconsistent at its newest generation"
        );
        let plaintext = self.open_vault(&source)?;
        let bundle: PortableCredentialBundle =
            serde_json::from_slice(&plaintext).context("decoding shared credential bundle")?;
        let mut stored_values = Vec::new();
        for package in packages
            .iter()
            .filter(|package| !providers.contains_key(&package.manifest.id))
        {
            let provider_id = &package.manifest.id;
            let auth = &package.manifest.authentication;
            if auth.portable_schema != source.portable_schema {
                continue;
            }
            self.require_portable_packages(std::slice::from_ref(package))?;
            if let Err(error) = validate_bundle(&portable_contract::contract(package), &bundle) {
                tracing::warn!(
                    %error,
                    provider_id,
                    source_provider_id = source.provider_id,
                    "shared credential does not satisfy this Provider's contract"
                );
                continue;
            }
            let fingerprint = portable_contract::fingerprint(package)?;
            let (nonce, ciphertext) =
                self.seal_vault(provider_id, generation, &fingerprint, &plaintext, None)?;
            stored_values.push(StoredProviderAuthentication {
                promotion_commitment: None,
                vault_schema: self.vault_schema(),
                recipient_key: self.recipient_key.clone(),
                provider_id: provider_id.clone(),
                account_id: self.account_id.clone(),
                auth_generation: generation,
                authentication_state: ServiceAuthenticationState::Ready,
                distribution_state: ServiceDistributionState::Pending,
                auth_contract_fingerprint: fingerprint,
                portable_schema: auth.portable_schema.clone(),
                projection_schema: auth.projection_schema.clone(),
                action: ProviderAuthAction::Apply,
                nonce,
                ciphertext,
                account_label: source.account_label.clone(),
                updated_at_ms: now_ms(),
            });
        }
        let mut statuses = Vec::with_capacity(stored_values.len());
        for stored in stored_values {
            tracing::info!(
                provider_id = stored.provider_id,
                source_provider_id = source.provider_id,
                auth_generation = generation,
                scope = source.portable_schema,
                "Provider adopted its scope's shared Service credential"
            );
            statuses.push(self.replace_locked(&mut providers, stored)?);
        }
        Ok(statuses)
    }
}

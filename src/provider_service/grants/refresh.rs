//! Refresh within the original verified grant lineage. Never share native
//! refresh tokens with another Provider, account, or Machine.
use super::*;
use crate::provider_account::NativeIdentity;
use crate::store::provider_accounts::grants::Admission;

impl ProviderAuthService {
    pub(crate) async fn refresh_current_grant(
        &self,
        store: &crate::store::Store,
        machine: &str,
        package: &ProviderPackage,
        bundle: &PortableCredentialBundle,
        expected_generation: u64,
        identity: &NativeIdentity,
    ) -> Result<ProviderAuthRefreshResult> {
        let provider = &package.manifest.id;
        let commitment = self
            .providers
            .read()
            .get(provider)
            .and_then(|stored| stored.promotion_commitment.clone())
            .context("Provider refresh has no operation-bound credentials")?;
        let (owner, operation) = store
            .provider_grant_route(&commitment)
            .await?
            .context("Provider refresh has no committed route")?;
        store
            .with_provider_grant_admission(&owner, &operation, machine, |admission| {
                ensure!(
                    admission.receipt().provider_id == *provider
                        && admission.receipt().identity_fingerprint
                            == identity.fingerprint(provider).map_err(anyhow::Error::msg)?,
                    "Provider refresh native identity mismatch"
                );
                self.refresh_admitted_grant(admission, package, bundle, expected_generation)
            })
            .await
    }

    fn refresh_admitted_grant(
        &self,
        admission: &Admission,
        package: &ProviderPackage,
        bundle: &PortableCredentialBundle,
        expected_generation: u64,
    ) -> Result<ProviderAuthRefreshResult> {
        let provider = &package.manifest.id;
        let (_, plaintext) =
            validate_refresh_request(std::slice::from_ref(package), bundle, provider)?;
        let mut intent = admission.receipt().clone();
        let Phase::Committed {
            generation: initial_generation,
        } = intent.phase
        else {
            anyhow::bail!("Provider refresh is not committed");
        };
        intent.phase = Phase::Prepared;
        let hash = self.validate_grant_intent(&intent)?;
        let mut providers = self.providers.write();
        let current = providers
            .get(provider)
            .context("Provider refresh state missing")?;
        ensure!(
            current.promotion_commitment.as_ref() == Some(&hash)
                && current.auth_generation >= initial_generation,
            "Provider refresh lineage changed"
        );
        if current.auth_generation != expected_generation {
            return Ok(ProviderAuthRefreshResult::Stale {
                current_generation: current.auth_generation,
            });
        }
        ensure!(
            current.action == ProviderAuthAction::Apply
                && matches!(
                    current.authentication_state,
                    ServiceAuthenticationState::Ready | ServiceAuthenticationState::Expired
                ),
            "Provider refresh is not ready"
        );
        ensure!(
            current.auth_contract_fingerprint
                == package.manifest.compatibility.auth_contract_fingerprint
                && current.portable_schema == package.manifest.authentication.portable_schema
                && current.projection_schema == package.manifest.authentication.projection_schema,
            "Provider refresh contract changed"
        );
        let previous: PortableCredentialBundle =
            serde_json::from_slice(&self.open_vault(current)?)?;
        ensure!(
            previous.method_id == bundle.method_id,
            "Provider refresh authentication method changed"
        );
        if previous == *bundle {
            return Ok(ProviderAuthRefreshResult::Unchanged(vec![current.status()]));
        }
        let mut updated = current.clone();
        updated.auth_generation = expected_generation
            .checked_add(1)
            .filter(|value| *value != u64::MAX)
            .context("Provider auth generation exhausted")?;
        let (nonce, ciphertext) = self.seal_vault(
            provider,
            updated.auth_generation,
            &current.auth_contract_fingerprint,
            &plaintext,
            Some(&hash),
        )?;
        updated.nonce = nonce;
        updated.ciphertext = ciphertext;
        updated.authentication_state = ServiceAuthenticationState::Ready;
        updated.distribution_state = ServiceDistributionState::Pending;
        updated.updated_at_ms = now_ms();
        let status = self.replace_locked(&mut providers, updated)?;
        Ok(ProviderAuthRefreshResult::Updated(vec![status]))
    }
}

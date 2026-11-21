//! Read-only subscription identity of a named portable account whose vault
//! credential was adopted without a sign-in (`ReadPortableIdentity`).
//!
//! The account's replica at exactly the admitted vault generation is staged
//! in a private credential home, and the exact release's signed native
//! identity recipe (the one a sign-in runs on its fresh login home) reads the
//! identity from the credential file the portable bundle carries. No
//! collector or Provider CLI starts, no token is redeemed or refreshed, and
//! nothing contacts the Provider. Only the identity and the credential's
//! portable method leave the Machine.
use super::*;
use crate::machine_protocol::portable_usage::{PortableIdentityReply, PortableIdentityRequest};

impl MachinePluginStore {
    /// Read the identity of `request`'s account from its replica.
    ///
    /// # Errors
    /// A closed code ([`crate::machine_protocol::portable_usage::portable_identity_error_code`]).
    pub async fn read_portable_identity(
        &self,
        request: PortableIdentityRequest,
        connected: Arc<AtomicBool>,
    ) -> Result<PortableIdentityReply> {
        let admission = HostAdmission::new(connected);
        portable_account(
            &request.account_id,
            &request.plugin_id,
            request.auth_generation,
        )?;
        let scoped = self
            .for_account(&request.account_id)
            .context("portable_usage_account_required")?;
        // The account's credential guard: no apply or wipe overlaps the read.
        let _guard =
            tokio::time::timeout(admission.remaining()?, scoped.native_observation_guard())
                .await
                .map_err(|_| anyhow::anyhow!("portable_usage_busy"))?;
        admission.remaining()?;
        // The lifecycle lock uninstall holds: the exact release is checked
        // and the private copy staged and removed under it, so an uninstall
        // either finished first or runs after the copy is gone.
        let _lifecycle = tokio::time::timeout(admission.remaining()?, self.lifecycle.lock())
            .await
            .map_err(|_| anyhow::anyhow!("portable_usage_busy"))?;
        let (package, digest) = scoped
            .active_package(&request.plugin_id)?
            .context("portable_usage_collector_unavailable")?;
        ensure!(
            package.manifest.version == request.plugin_version
                && digest == request.generation_digest,
            "native_usage_installation_changed"
        );
        let envelope = scoped
            .latest_auth_envelope(&request.plugin_id)?
            .filter(|replica| {
                replica.auth_generation == request.auth_generation
                    && replica.action == ProviderAuthAction::Apply
            })
            .context("portable_usage_replica_pending")?;
        let PreparedProviderAuth::Apply(bundle) =
            scoped.prepare_auth_inner(&envelope, Some(&package))?
        else {
            anyhow::bail!("portable_usage_replica_pending");
        };
        let auth = &package.manifest.authentication;
        let recipe = portable_contract::native_method_for(auth, &bundle.method_id)
            .and_then(|method| method.native_identity.as_ref())
            .filter(|recipe| portable_contract::identity_in_credentials(auth, recipe))
            .context("portable_identity_unsupported")?;
        let (private, _) = scoped
            .private_credential_home(&request.plugin_id, request.auth_generation)?
            .context("portable_identity_unsupported")?;
        let identity = crate::machine_plugins::native_identity::read(
            &private.path().join("home"),
            recipe,
            &request.account_id,
        );
        private.remove()?;
        Ok(PortableIdentityReply {
            method_id: bundle.method_id,
            identity: identity.map_err(|_| anyhow::anyhow!("portable_identity_unreadable"))?,
        })
    }
}

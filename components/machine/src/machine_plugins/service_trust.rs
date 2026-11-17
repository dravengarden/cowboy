//! The Machine's single pinned Service signing key, shared by every scope.
use super::{MachinePluginStore, Result, SealedProviderAuth, ensure, pin_key};

impl MachinePluginStore {
    /// A named scope never establishes the trust root by itself.
    pub(super) fn pin_service_key(&self, public_key: &str) -> Result<()> {
        let pin = self.auth_base.join("service.pub");
        ensure!(
            self.account_id.is_default() || pin.is_file(),
            "named account requires an established Service trust root"
        );
        pin_key(&pin, public_key)
    }

    /// Pin the signer of a verified envelope. The default scope and a portable
    /// account (a Service-held key, applied or wiped) may establish the root on
    /// first use, over the same authenticated Controller connection as a
    /// default apply. A native named scope requires an existing root.
    pub(super) fn pin_envelope_service_key(&self, envelope: &SealedProviderAuth) -> Result<()> {
        if self.account_id.is_default()
            || crate::provider_account::portable_credentials_allowed(
                &self.account_id,
                &envelope.provider_id,
            )
            .is_ok()
        {
            pin_key(
                &self.auth_base.join("service.pub"),
                &envelope.service_public_key,
            )
        } else {
            self.pin_service_key(&envelope.service_public_key)
        }
    }
}

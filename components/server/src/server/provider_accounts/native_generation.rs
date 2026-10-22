//! Resolve native authentication without consulting default-account replicas.
use crate::machine_protocol::PluginInventory;
use crate::provider_account::NativeProfileReceipt;
use cowboy_provider_sdk::{AuthExecutor, ProviderManifest};

pub(in crate::server) fn resolve(
    profile: &NativeProfileReceipt,
    installed: &PluginInventory,
    manifest: &ProviderManifest,
    signed_in: Option<&ProviderManifest>,
) -> Result<u64, String> {
    if profile.account_id.is_default()
        || profile.provider_id != "claude-code"
        || profile.generation == 0
    {
        return Err("native_account_required".into());
    }
    if profile.provider_id != installed.plugin_id
        || profile.provider_id != manifest.id
        || installed.plugin_version != manifest.version
    {
        return Err("native_provider_generation_mismatch".into());
    }
    let exact = profile.provider_version == installed.plugin_version
        && profile.generation_digest == installed.generation_digest;
    // A later release keeps the login only when its signed authentication is
    // identical to the release used to sign in (Catalog-verified). The Machine
    // repeats this comparison against the retained release before every use.
    if !exact
        && !signed_in.is_some_and(|signed_in| {
            signed_in.version == profile.provider_version
                && crate::provider_account::native_release_compatible(signed_in, manifest)
        })
    {
        return Err("native_provider_generation_mismatch".into());
    }
    if profile.auth_contract_fingerprint != manifest.compatibility.auth_contract_fingerprint {
        return Err("authentication_contract_mismatch".into());
    }
    let method = manifest
        .authentication
        .methods
        .iter()
        .find(|method| method.id == profile.method)
        .ok_or("authentication_method_unavailable")?;
    if method.native_identity.is_none()
        || !matches!(method.executor, AuthExecutor::CommandV1 { .. })
    {
        return Err("native_identity_verification_required".into());
    }
    super::validate_native_evidence(
        &profile.provider_id,
        method.native_identity.as_ref(),
        Some(&profile.identity),
    )
    .map_err(str::to_owned)?;
    Ok(profile.generation)
}

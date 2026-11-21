//! Establish the subscription identity of a named portable account whose
//! vault credential was adopted from a legacy card without a sign-in (so it
//! has no identity fingerprint, and resets refuse it).
//!
//! The evidence path is a sign-in's: on the Machine the account's usage runs
//! on, the exact selected release's signed native identity recipe reads the
//! identity from the account's replica at exactly the vault's current
//! generation (`ReadPortableIdentity`, read-only: no collector or Provider CLI
//! starts, no token is redeemed or refreshed). The Service checks the
//! evidence against the same recipe of its own copy of that release, then
//! binds it under the owner lock with the sign-in rules: a matching bound
//! identity is a no-op, another is refused, and an identity already bound to
//! another account of the Provider is `native_identity_already_bound`. The
//! bind is refused unless the vault still holds the generation that was read.
use super::*;
use crate::machine_protocol::portable_usage::{
    PORTABLE_IDENTITY_PROTOCOL_VERSION, PortableIdentityReply, PortableIdentityRequest,
};
use crate::provider_account::portable_contract;

/// Establish `id`'s identity once under `operation` (already namespaced by
/// its caller); a replay returns the first result.
pub(in crate::server) async fn establish(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    id: &ProviderAccountId,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    if let Some(done) = store
        .provider_account_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?
    {
        return Ok(json!({"status":"bound","account_id":done.account_id,"replayed":true}));
    }
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let account = registry
        .accounts
        .iter()
        .find(|account| &account.id == id && account.available())
        .cloned()
        .ok_or("account_unavailable")?;
    if account.identity_fingerprint.is_some() {
        return Ok(json!({"status":"bound","account_id":id,"already_bound":true}));
    }
    if !account.portable_credentials {
        return Err("portable_credentials_required");
    }
    let target =
        portable::target(state, &account, None).ok_or("account_identity_unsupported")??;
    let (machine, installed, _) = portable::placement(state, owner, id, &target)?;
    if !state
        .machine_control
        .machine_supports(&machine, PORTABLE_IDENTITY_PROTOCOL_VERSION)
    {
        return Err("machine_usage_update_required");
    }
    let package = state
        .provider_catalog
        .package(
            &installed.plugin_id,
            &installed.plugin_version,
            &installed.generation_digest,
        )
        .filter(|package| {
            crate::provider_catalog::subscription_account_contract(&package.manifest.authentication)
        })
        .ok_or("account_identity_unsupported")?;
    let request = PortableIdentityRequest {
        account_id: id.clone(),
        plugin_id: installed.plugin_id.clone(),
        plugin_version: installed.plugin_version.clone(),
        generation_digest: installed.generation_digest.clone(),
        auth_generation: target.auth_generation,
    };
    let (pending, code) = portable::under_account_lock(state, owner, id, &target, || {
        state
            .machine_control
            .begin_portable_identity(&machine, &installed, request)
            .map_err(|error| crate::usage::portable::dispatch_error(&error))
    })
    .await;
    let reply = pending.ok_or(code)?.await?;
    let identity = verified(&package, id, &reply)?;
    let current = |current: &crate::store::provider_accounts::Account| {
        current.provider_id == account.provider_id
            && super::super::portable::scope(state, id, &target.plugin_id)
                .and_then(|vault| vault.status(&target.plugin_id))
                .is_some_and(|status| status.auth_generation == target.auth_generation)
    };
    match store
        .bind_adopted_identity(owner, operation, id, identity, current)
        .await
        .map_err(|_| "outcome_unknown")?
    {
        crate::store::provider_accounts::MutationResult::Applied(_) => {
            state.hub.broadcast_account_usage_changed(owner);
            Ok(json!({"status":"bound","account_id":id}))
        }
        crate::store::provider_accounts::MutationResult::Invalid(code) => Err(code),
        crate::store::provider_accounts::MutationResult::Conflict(_) => Err("unavailable"),
    }
}

/// The Machine's evidence must be the identity the signed recipe of the
/// credential's native sign-in method reads from a carried credential file.
fn verified<'a>(
    package: &cowboy_provider_sdk::ProviderPackage,
    id: &ProviderAccountId,
    reply: &'a PortableIdentityReply,
) -> Result<&'a crate::provider_account::NativeIdentity, &'static str> {
    let auth = &package.manifest.authentication;
    let recipe = portable_contract::native_method_for(auth, &reply.method_id)
        .and_then(|method| method.native_identity.as_ref())
        .filter(|recipe| portable_contract::identity_in_credentials(auth, recipe))
        .ok_or("account_identity_unsupported")?;
    super::super::validate_native_evidence(
        &package.manifest.id,
        id,
        Some(recipe),
        Some(&reply.identity),
    )?;
    Ok(&reply.identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(plugin: &str) -> cowboy_provider_sdk::ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(
            &std::fs::read_to_string(format!(
                "{}/../../plugins/{plugin}/provider.json",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap(),
        )
        .unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    /// Codex and Grok adopted credentials prove their identity with the
    /// signed recipe of their own sign-in method; nothing else is accepted.
    #[test]
    fn only_the_credential_methods_recipe_identity_is_accepted() {
        let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        for (plugin, method, authority) in [
            ("codex", "chatgpt", "chatgpt"),
            ("grok", "xai-account", "x-ai"),
        ] {
            let package = package(plugin);
            let reply = PortableIdentityReply {
                method_id: method.into(),
                identity: crate::provider_account::NativeIdentity {
                    authority: authority.into(),
                    subject: "user".into(),
                    tenant: Some("team".into()),
                },
            };
            assert_eq!(
                verified(&package, &account, &reply),
                Ok(&reply.identity),
                "{plugin}"
            );
            let mut other = reply.clone();
            other.identity.authority = "someone-else".into();
            assert!(verified(&package, &account, &other).is_err(), "{plugin}");
            let mut untenanted = reply.clone();
            untenanted.identity.tenant = None;
            assert!(
                verified(&package, &account, &untenanted).is_err(),
                "{plugin}"
            );
            let mut key = reply.clone();
            key.method_id = "api-key".into();
            assert!(verified(&package, &account, &key).is_err(), "{plugin}");
        }
    }
}

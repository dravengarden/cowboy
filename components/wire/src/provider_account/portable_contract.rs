//! The legacy Claude vault and native-owned accounts have separate contracts.
//!
//! Adding a native login recipe must not rotate the default account's portable
//! credential identity. Only native methods are omitted: every existing
//! method, projection, required key and refresh rule remains in the digest.
//! The complete signed package still binds native methods, and native profiles
//! continue to require its full auth fingerprint and exact release digest.

use std::borrow::Cow;

use anyhow::Result;
use cowboy_provider_sdk::{AuthenticationContract, ProviderPackage};
use sha2::{Digest as _, Sha256};

pub fn contract(package: &ProviderPackage) -> Cow<'_, AuthenticationContract> {
    let auth = &package.manifest.authentication;
    if !auth.native_accounts() {
        return Cow::Borrowed(auth);
    }
    let mut legacy = auth.clone();
    legacy
        .methods
        .retain(|method| method.native_identity.is_none());
    Cow::Owned(legacy)
}

pub fn fingerprint(package: &ProviderPackage) -> Result<String> {
    match contract(package) {
        Cow::Borrowed(_) => Ok(package
            .manifest
            .compatibility
            .auth_contract_fingerprint
            .clone()),
        // Provider typed fingerprints preserve Serde field order, unlike the
        // SDK's sorted arbitrary-JSON fingerprint. Keep the historical bytes.
        Cow::Owned(legacy) => Ok(format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&legacy)?)
        )),
    }
}

/// The portable method a named subscription's vault bundle is stored under:
/// the signed method without a native identity recipe that runs the same
/// executor and requires the same bundle keys as the native sign-in method.
/// The native method only signs in and proves identity; the credential it
/// produces is byte-for-byte the legacy portable credential, so projection,
/// refresh and delivery follow the default account's contract unchanged.
#[must_use]
pub fn portable_twin<'a>(
    auth: &'a AuthenticationContract,
    native: &cowboy_provider_sdk::AuthMethod,
) -> Option<&'a cowboy_provider_sdk::AuthMethod> {
    native.native_identity.as_ref()?;
    auth.methods.iter().find(|method| {
        method.native_identity.is_none()
            && method.executor == native.executor
            && method.required_bundle_keys == native.required_bundle_keys
    })
}

/// The native sign-in method whose portable twin is `twin_id`.
#[must_use]
pub fn native_method_for<'a>(
    auth: &'a AuthenticationContract,
    twin_id: &str,
) -> Option<&'a cowboy_provider_sdk::AuthMethod> {
    auth.methods
        .iter()
        .find(|method| portable_twin(auth, method).is_some_and(|twin| twin.id == twin_id))
}

/// Whether every native sign-in method has a portable twin: its named
/// accounts then keep credentials in the account's Service vault.
#[must_use]
pub fn subscription_accounts(auth: &AuthenticationContract) -> bool {
    auth.native_accounts()
        && auth
            .methods
            .iter()
            .filter(|method| method.native_identity.is_some())
            .all(|method| portable_twin(auth, method).is_some())
}

/// Whether `recipe` reads the identity from a file the portable bundle
/// carries, so a refreshed credential alone proves which account it is.
#[must_use]
pub fn identity_in_credentials(
    auth: &AuthenticationContract,
    recipe: &cowboy_provider_sdk::NativeIdentityRecipe,
) -> bool {
    match recipe {
        cowboy_provider_sdk::NativeIdentityRecipe::FreshLoginJsonV1 { relative_path, .. } => auth
            .credential_files
            .iter()
            .any(|file| &file.relative_path == relative_path),
        cowboy_provider_sdk::NativeIdentityRecipe::OwnerDeclaredV1 { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> cowboy_provider_sdk::StandardProviderSource {
        serde_json::from_str(include_str!(
            "../../../../plugins/claude-code/provider.json"
        ))
        .unwrap()
    }

    #[test]
    fn native_recipes_do_not_change_legacy_credentials_but_remain_signed() {
        let source = source();
        let next = cowboy_provider_sdk::build_package(source.clone().compile().unwrap()).unwrap();
        let mut previous = source;
        previous
            .authentication
            .methods
            .retain(|method| method.native_identity.is_none());
        let previous = cowboy_provider_sdk::build_package(previous.compile().unwrap()).unwrap();
        assert_eq!(
            fingerprint(&next).unwrap(),
            previous.manifest.compatibility.auth_contract_fingerprint
        );
        // Public signed Claude 3.20.0 contract, independently observed in the
        // Catalog. This also catches accidental typed serialization drift.
        assert_eq!(
            fingerprint(&next).unwrap(),
            "sha256:ceb8d6e09bd5d375f085435d23a52c63976cdf165f07a8b6946484207af4ede7"
        );
        assert_ne!(next.contract_fingerprint, previous.contract_fingerprint);
        assert_ne!(
            next.manifest.compatibility.auth_contract_fingerprint,
            previous.manifest.compatibility.auth_contract_fingerprint
        );
        assert!(
            contract(&next)
                .methods
                .iter()
                .all(|method| method.native_identity.is_none())
        );
    }

    #[test]
    fn every_subscription_signs_in_natively_and_stores_its_portable_twin() {
        for (source, native, twin, in_credentials) in [
            (
                include_str!("../../../../plugins/claude-code/provider.json"),
                "claude-subscription",
                "claude-account",
                false,
            ),
            (
                include_str!("../../../../plugins/codex/provider.json"),
                "chatgpt-subscription",
                "chatgpt",
                true,
            ),
            (
                include_str!("../../../../plugins/grok/provider.json"),
                "xai-subscription",
                "xai-account",
                true,
            ),
        ] {
            let source: cowboy_provider_sdk::StandardProviderSource =
                serde_json::from_str(source).unwrap();
            let auth = source.compile().unwrap().authentication;
            assert!(subscription_accounts(&auth));
            let method = auth.methods.iter().find(|m| m.id == native).unwrap();
            assert_eq!(portable_twin(&auth, method).unwrap().id, twin);
            assert_eq!(native_method_for(&auth, twin).unwrap().id, native);
            assert!(native_method_for(&auth, "api-key").is_none());
            assert_eq!(
                identity_in_credentials(&auth, method.native_identity.as_ref().unwrap()),
                in_credentials
            );
            // The twin is part of the legacy portable contract.
            let mut legacy = auth.clone();
            legacy.methods.retain(|m| m.native_identity.is_none());
            assert!(legacy.methods.iter().any(|m| m.id == twin));
        }
        let deepseek: cowboy_provider_sdk::StandardProviderSource = serde_json::from_str(
            include_str!("../../../../plugins/claude-deepseek/provider.json"),
        )
        .unwrap();
        assert!(!subscription_accounts(
            &deepseek.compile().unwrap().authentication
        ));
    }

    #[test]
    fn existing_credential_contract_changes_are_not_compatible() {
        let source = source();
        let package =
            cowboy_provider_sdk::build_package(source.clone().compile().unwrap()).unwrap();
        let original = fingerprint(&package).unwrap();
        for mutation in 0..5 {
            let mut changed = source.clone();
            match mutation {
                0 => changed.authentication.projection_schema.push_str("-new"),
                1 => changed.authentication.portable_schema.push_str("-new"),
                2 => {
                    changed.authentication.credential_files[0].relative_path =
                        ".other/credentials.json".into()
                }
                3 => changed.authentication.methods[0].label.push_str(" changed"),
                _ => {
                    changed.authentication.refresh = cowboy_provider_sdk::RefreshOwnership::Service
                }
            }
            let changed = cowboy_provider_sdk::build_package(changed.compile().unwrap()).unwrap();
            assert_ne!(
                fingerprint(&changed).unwrap(),
                original,
                "mutation {mutation}"
            );
        }
        // Native ownership follows the signed recipe, not a Provider id: any
        // Provider that adds native methods keeps its legacy vault identity.
        let mut other = source;
        other.id = "native-fixture".into();
        let other = cowboy_provider_sdk::build_package(other.compile().unwrap()).unwrap();
        assert_eq!(fingerprint(&other).unwrap(), fingerprint(&package).unwrap());
    }

    #[test]
    fn adding_native_subscriptions_keeps_each_legacy_vault_identity() {
        for (plugin, source) in [
            (
                "codex",
                include_str!("../../../../plugins/codex/provider.json"),
            ),
            (
                "grok",
                include_str!("../../../../plugins/grok/provider.json"),
            ),
        ] {
            let source: cowboy_provider_sdk::StandardProviderSource =
                serde_json::from_str(source).unwrap();
            let next =
                cowboy_provider_sdk::build_package(source.clone().compile().unwrap()).unwrap();
            assert!(next.manifest.authentication.native_accounts(), "{plugin}");
            let mut previous = source;
            previous
                .authentication
                .methods
                .retain(|method| method.native_identity.is_none());
            let previous = cowboy_provider_sdk::build_package(previous.compile().unwrap()).unwrap();
            assert_eq!(
                fingerprint(&next).unwrap(),
                previous.manifest.compatibility.auth_contract_fingerprint,
                "{plugin}"
            );
        }
    }
}

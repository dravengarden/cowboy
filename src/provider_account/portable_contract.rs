//! The legacy Claude vault and native-owned accounts have separate contracts.
//!
//! Adding a native login recipe must not rotate the default account's portable
//! credential identity. Only native Claude methods are omitted: every existing
//! method, projection, required key and refresh rule remains in the digest.
//! The complete signed package still binds native methods, and native profiles
//! continue to require its full auth fingerprint and exact release digest.

use std::borrow::Cow;

use anyhow::Result;
use cowboy_provider_sdk::{AuthenticationContract, ProviderPackage};
use sha2::{Digest as _, Sha256};

pub(crate) fn contract(package: &ProviderPackage) -> Cow<'_, AuthenticationContract> {
    let auth = &package.manifest.authentication;
    if package.manifest.id != "claude-code"
        || !auth
            .methods
            .iter()
            .any(|method| method.native_identity.is_some())
    {
        return Cow::Borrowed(auth);
    }
    let mut legacy = auth.clone();
    legacy
        .methods
        .retain(|method| method.native_identity.is_none());
    Cow::Owned(legacy)
}

pub(crate) fn fingerprint(package: &ProviderPackage) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> cowboy_provider_sdk::StandardProviderSource {
        serde_json::from_str(include_str!("../../plugins/claude-code/provider.json")).unwrap()
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
        // Other Providers' identity recipes can belong to a portable grant;
        // they retain the complete contract until their ownership is defined.
        let mut other = source;
        other.id = "native-fixture".into();
        let other = cowboy_provider_sdk::build_package(other.compile().unwrap()).unwrap();
        assert_eq!(
            fingerprint(&other).unwrap(),
            other.manifest.compatibility.auth_contract_fingerprint
        );
    }
}

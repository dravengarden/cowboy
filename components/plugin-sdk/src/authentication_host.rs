//! Capability restrictions for data-only, Controller-owned authentication.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, ensure};

use crate::{
    AuthenticationProtocol, AuthenticationProviderContract, PluginHostSpec, PluginRendererId,
    PluginSlotId,
};

pub(crate) fn validate(
    contract: &AuthenticationProviderContract,
    host: Option<&PluginHostSpec>,
    files: Option<&BTreeMap<String, String>>,
) -> Result<()> {
    let (slot, renderer) = match contract.protocol {
        AuthenticationProtocol::OpenIdConnect(_) => {
            // Pre-host-bundle OIDC releases remain valid. They grant no host
            // capabilities; absence must never borrow another release's host.
            if host.is_none() {
                return Ok(());
            }
            (PluginSlotId::LoginMethod, PluginRendererId::LoginOidcV1)
        }
        AuthenticationProtocol::LocalPassword(_) => {
            (PluginSlotId::LoginMethod, PluginRendererId::LoginPasswordV1)
        }
        AuthenticationProtocol::Webauthn(_) => (
            PluginSlotId::AccountPanel,
            PluginRendererId::AccountPasskeysV1,
        ),
    };
    let host = host.context("local Authentication Plugin requires a release-bound host bundle")?;
    ensure!(
        files.is_some_and(|files| files.len() == 1 && files.contains_key("host.json")),
        "Authentication Plugins cannot include executable or auxiliary host files"
    );
    ensure!(
        host.slots == [slot]
            && host
                .ui
                .as_ref()
                .is_some_and(|ui| ui.renderers.get(&slot) == Some(&renderer)),
        "Authentication host renderer does not match its protocol"
    );
    let webauthn = matches!(contract.protocol, AuthenticationProtocol::Webauthn(_));
    let password = matches!(contract.protocol, AuthenticationProtocol::LocalPassword(_));
    ensure!(
        !webauthn || host.storage.is_some(),
        "WebAuthn host requires plugin-owned storage"
    );
    // An explicit typed allowlist fails to compile if the host contract grows;
    // new host powers must be consciously reviewed for Authentication use.
    let allowed = PluginHostSpec {
        schema_version: host.schema_version,
        slots: host.slots.clone(),
        ui: host.ui.clone(),
        storage: if webauthn { host.storage.clone() } else { None },
        native_capabilities: if webauthn {
            vec!["webauthn".to_owned()]
        } else {
            Vec::new()
        },
        usage: None,
        label: host.label.clone(),
        loopback_origin: None,
        loopback_detail: None,
        loopback_requires_catalog: false,
        loopback_catalog: None,
        loopback_env: None,
        adapter_slot: None,
        cli_executable: None,
        cli_executable_env: None,
        cli_auth: crate::PluginCliAuthKind::default(),
        cli_auth_argv: Vec::new(),
        cli_auth_rules: None,
        login_fields: if password {
            host.login_fields.clone()
        } else {
            None
        },
        rpc_argv: Vec::new(),
        visual: host.visual.clone(),
        isolated_home_env: None,
        isolated_home_settings: BTreeMap::new(),
        isolated_shell: false,
        isolated_shell_env: Vec::new(),
    };
    ensure!(
        host == &allowed,
        "Authentication host claims capabilities outside its Controller protocol"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PluginContractInventory, PluginManifest, PluginPackage, PluginPayload};
    use serde_json::{Value, json};

    fn contract(protocol: &str) -> AuthenticationProviderContract {
        serde_json::from_value(json!({
            "schema_version": 2, "id": "future-login", "version": "1.0.0",
            "display_name": "Future login", "button_label": "Continue",
            "protocol": {"kind": protocol, "configuration": {}}
        }))
        .unwrap()
    }

    fn local_package(contract: AuthenticationProviderContract) -> Result<PluginPackage> {
        let manifest: PluginManifest = serde_json::from_value(json!({
            "schema_version": 1, "id": contract.id, "version": contract.version,
            "component_release": "2.4.0", "publisher": "fixture",
            "kind": "authentication_provider", "entrypoint": "authentication.json",
            "components": [
                {"id": "cowboy.plugin-contract", "version": "1.5.0"},
                {"id": "cowboy.plugin-sdk", "version": crate::PLUGIN_SDK_VERSION}
            ]
        }))
        .unwrap();
        PluginPackage::new(
            manifest,
            "2.4.0".to_owned(),
            PluginPayload::AuthenticationProvider(contract),
        )
    }

    fn host(webauthn: bool) -> Value {
        if webauthn {
            let migrations = json!({"migrations":[{
                "version":"0001", "sql":"CREATE TABLE credentials (id TEXT PRIMARY KEY);"
            }]});
            json!({
                "schema_version": 1, "slots": ["account.panel"],
                "ui": {"schema_version": 1, "renderers": {"account.panel": "account-passkeys-v1"}},
                "native_capabilities": ["webauthn"],
                "storage": {"postgres": migrations, "sqlite": migrations}
            })
        } else {
            json!({
                "schema_version": 1, "slots": ["login.method"],
                "ui": {"schema_version": 1, "renderers": {"login.method": "login-password-v1"}}
            })
        }
    }

    fn files(host: &Value) -> BTreeMap<String, String> {
        BTreeMap::from([("host.json".to_owned(), host.to_string())])
    }

    #[test]
    fn local_protocols_are_versioned_and_not_bound_to_first_party_ids() {
        for (protocol, webauthn) in [("local_password", false), ("webauthn", true)] {
            let contract = contract(protocol);
            let package = local_package(contract.clone()).unwrap();
            let bytes = package.canonical_bytes().unwrap();
            assert_eq!(PluginPackage::from_bytes(&bytes).unwrap(), package);
            package
                .validate_host_contract(Some(&files(&host(webauthn))))
                .unwrap();
            assert!(package.validate_host_contract(None).is_err());

            let mut downgraded = contract;
            downgraded.schema_version = 1;
            assert!(local_package(downgraded).is_err());
        }
        let inventory = PluginContractInventory::current_machine(1);
        assert_eq!(inventory.min_authentication_provider_schema, 1);
        assert_eq!(inventory.max_authentication_provider_schema, 2);
    }

    #[test]
    fn local_protocol_configuration_cannot_carry_policy_or_secrets() {
        for protocol in ["local_password", "webauthn"] {
            for configuration in [
                json!({"secret":"fixture"}),
                json!({"algorithm":"custom"}),
                json!(null),
                json!([]),
            ] {
                let mut value = serde_json::to_value(contract(protocol)).unwrap();
                value["protocol"]["configuration"] = configuration;
                assert!(
                    serde_json::from_value::<AuthenticationProviderContract>(value.clone())
                        .is_err(),
                    "{value}"
                );
            }
            let mut value = serde_json::to_value(contract(protocol)).unwrap();
            value["protocol"]["script"] = json!("not-allowed");
            assert!(serde_json::from_value::<AuthenticationProviderContract>(value).is_err());
        }
    }

    #[test]
    fn authentication_hosts_cannot_claim_other_drivers_or_execution() {
        let package = local_package(contract("local_password")).unwrap();
        for (field, value) in [
            ("adapter_slot", json!("future-adapter")),
            ("cli_executable", json!("future-cli")),
            ("native_capabilities", json!(["webauthn"])),
            ("isolated_home_env", json!("FUTURE_HOME")),
            (
                "usage",
                json!({"account":"future-account","collector":"session"}),
            ),
        ] {
            let mut changed = host(false);
            changed[field] = value;
            assert!(
                package
                    .validate_host_contract(Some(&files(&changed)))
                    .is_err(),
                "{field}"
            );
        }
        let mut wrong_renderer = host(false);
        wrong_renderer["ui"]["renderers"]["login.method"] = json!("login-oidc-v1");
        assert!(
            package
                .validate_host_contract(Some(&files(&wrong_renderer)))
                .is_err()
        );
        let mut executable = files(&host(false));
        executable.insert(
            "collector/index.js".to_owned(),
            "export default 1;".to_owned(),
        );
        assert!(package.validate_host_contract(Some(&executable)).is_err());
    }

    #[test]
    fn webauthn_requires_storage_and_exact_native_capability() {
        let package = local_package(contract("webauthn")).unwrap();
        for field in ["storage", "native_capabilities"] {
            let mut changed = host(true);
            changed.as_object_mut().unwrap().remove(field);
            assert!(
                package
                    .validate_host_contract(Some(&files(&changed)))
                    .is_err(),
                "{field}"
            );
        }
    }
}

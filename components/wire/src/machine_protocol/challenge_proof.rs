//! Handshake proofs a Machine signs over the Controller's challenge and its
//! own `Hello`; each version binds the fields its protocol generation added.

use super::{ConnectionMode, MachineHello, Platform};

/// Build the exact version-one proof signed during a remote Machine handshake.
/// Length-prefixed fields avoid delimiter ambiguity while keeping the proof
/// independent from later additions to the wire `Hello` object.
#[must_use]
pub fn challenge_proof_v1(
    challenge_id: &str,
    nonce: &str,
    expires_at_ms: i64,
    hello: &MachineHello,
) -> Vec<u8> {
    let platform = match hello.platform {
        Platform::Linux => "linux",
        Platform::Macos => "macos",
    };
    let connection_mode = match hello.connection_mode {
        ConnectionMode::LocalUds => "local_uds",
        ConnectionMode::OutboundTls => "outbound_tls",
    };
    let fields = [
        challenge_id.to_owned(),
        nonce.to_owned(),
        expires_at_ms.to_string(),
        hello.machine_id.clone(),
        platform.to_owned(),
        hello.arch.clone(),
        connection_mode.to_owned(),
        hello.min_protocol.to_string(),
        hello.max_protocol.to_string(),
        hello.min_runtime_protocol.to_string(),
        hello.max_runtime_protocol.to_string(),
        hello.host_build.clone(),
    ];
    let mut proof = b"cowboy-machine-proof-v1\n".to_vec();
    for field in fields {
        proof.extend_from_slice(field.len().to_string().as_bytes());
        proof.push(b':');
        proof.extend_from_slice(field.as_bytes());
        proof.push(b'\n');
    }
    proof
}

/// Version-two handshake proof additionally binds the credential-replica key.
/// A protocol-three Machine must use this proof so an enrolled signing key
/// cannot advertise a substituted X25519 recipient.
#[must_use]
pub fn challenge_proof_v2(
    challenge_id: &str,
    nonce: &str,
    expires_at_ms: i64,
    hello: &MachineHello,
) -> Vec<u8> {
    let mut proof = challenge_proof_v1(challenge_id, nonce, expires_at_ms, hello);
    proof.extend_from_slice(b"cowboy-machine-proof-v2\n");
    let encryption_key = hello.encryption_public_key.as_deref().unwrap_or_default();
    proof.extend_from_slice(encryption_key.len().to_string().as_bytes());
    proof.push(b':');
    proof.extend_from_slice(encryption_key.as_bytes());
    proof.push(b'\n');
    if let Some(provider_contracts) = &hello.provider_contracts {
        proof.extend_from_slice(b"provider-contracts:");
        proof.extend_from_slice(
            provider_contracts
                .provider_sdk_version
                .len()
                .to_string()
                .as_bytes(),
        );
        proof.push(b':');
        proof.extend_from_slice(provider_contracts.provider_sdk_version.as_bytes());
        for value in [
            provider_contracts.min_package_schema,
            provider_contracts.max_package_schema,
            provider_contracts.min_runtime_binding_schema,
            provider_contracts.max_runtime_binding_schema,
            provider_contracts.min_ui_schema,
            provider_contracts.max_ui_schema,
            provider_contracts.min_host_schema,
            provider_contracts.max_host_schema,
            provider_contracts.machine_contract,
        ] {
            proof.push(b':');
            proof.extend_from_slice(value.to_string().as_bytes());
        }
        proof.push(b'\n');
    }
    proof
}

/// Version-three handshake proof additionally binds the generic Plugin
/// decoder inventory introduced with Machine protocol seven. A new Machine
/// still signs proof v2 when an older Controller requests it, preserving
/// rolling upgrade compatibility in both directions.
#[must_use]
pub fn challenge_proof_v3(
    challenge_id: &str,
    nonce: &str,
    expires_at_ms: i64,
    hello: &MachineHello,
) -> Vec<u8> {
    let mut proof = challenge_proof_v2(challenge_id, nonce, expires_at_ms, hello);
    proof.extend_from_slice(b"cowboy-machine-proof-v3\n");
    if let Some(plugin_contracts) = &hello.plugin_contracts {
        proof.extend_from_slice(b"plugin-contracts:");
        proof.extend_from_slice(
            plugin_contracts
                .plugin_sdk_version
                .len()
                .to_string()
                .as_bytes(),
        );
        proof.push(b':');
        proof.extend_from_slice(plugin_contracts.plugin_sdk_version.as_bytes());
        for value in [
            plugin_contracts.min_manifest_schema,
            plugin_contracts.max_manifest_schema,
            plugin_contracts.min_package_schema,
            plugin_contracts.max_package_schema,
            plugin_contracts.min_release_schema,
            plugin_contracts.max_release_schema,
            plugin_contracts.min_agent_provider_schema,
            plugin_contracts.max_agent_provider_schema,
            plugin_contracts.min_authentication_provider_schema,
            plugin_contracts.max_authentication_provider_schema,
            plugin_contracts.min_code_intelligence_schema,
            plugin_contracts.max_code_intelligence_schema,
            plugin_contracts.min_host_bundle_schema,
            plugin_contracts.max_host_bundle_schema,
            plugin_contracts.min_host_schema,
            plugin_contracts.max_host_schema,
        ] {
            proof.push(b':');
            proof.extend_from_slice(value.to_string().as_bytes());
        }
        proof.push(b'\n');
    } else {
        proof.extend_from_slice(b"plugin-contracts:absent\n");
    }
    proof
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::MachineCapacity;

    #[test]
    fn challenge_proof_binds_machine_and_protocol_without_display_metadata() {
        let hello = MachineHello {
            machine_id: "falcon".to_owned(),
            display_name: "Falcon".to_owned(),
            platform: Platform::Linux,
            arch: "x86_64".to_owned(),
            connection_mode: ConnectionMode::OutboundTls,
            min_protocol: 1,
            max_protocol: 1,
            min_runtime_protocol: 1,
            max_runtime_protocol: 2,
            host_build: "build-a".to_owned(),
            challenge_id: None,
            challenge_signature: None,
            encryption_public_key: None,
            components: Vec::new(),
            plugins: Vec::new(),
            provider_contracts: None,
            plugin_contracts: None,
            workspaces: Vec::new(),
            workspace_identities: Vec::new(),
            workspace_revision: None,
            workspace_owner: None,
            capacity: MachineCapacity::default(),
            browser: None,
        };
        let proof = challenge_proof_v1("id", "nonce", 42, &hello);
        let mut renamed = hello.clone();
        renamed.display_name = "Renamed Falcon".to_owned();
        assert_eq!(proof, challenge_proof_v1("id", "nonce", 42, &renamed));
        let mut other_machine = hello.clone();
        other_machine.machine_id = "macbook-air".to_owned();
        assert_ne!(proof, challenge_proof_v1("id", "nonce", 42, &other_machine));
        let mut other_protocol = hello;
        other_protocol.max_runtime_protocol = 3;
        assert_ne!(
            proof,
            challenge_proof_v1("id", "nonce", 42, &other_protocol)
        );
    }

    #[test]
    fn challenge_proof_v2_binds_provider_contract_inventory_when_present() {
        let mut hello: MachineHello = serde_json::from_value(serde_json::json!({
            "machine_id": "falcon",
            "display_name": "Falcon",
            "platform": "linux",
            "arch": "x86_64",
            "connection_mode": "outbound_tls",
            "min_protocol": 3,
            "max_protocol": 4,
            "min_runtime_protocol": 1,
            "max_runtime_protocol": 3,
            "host_build": "test",
            "encryption_public_key": "machine-key"
        }))
        .unwrap();
        hello.provider_contracts =
            Some(cowboy_provider_sdk::ProviderContractInventory::current_machine());
        let proof = challenge_proof_v2("id", "nonce", 42, &hello);

        let mut downgraded = hello.clone();
        downgraded
            .provider_contracts
            .as_mut()
            .unwrap()
            .max_host_schema = 1;
        assert_ne!(proof, challenge_proof_v2("id", "nonce", 42, &downgraded));

        let mut omitted = hello;
        omitted.provider_contracts = None;
        assert_ne!(proof, challenge_proof_v2("id", "nonce", 42, &omitted));
    }

    #[test]
    fn challenge_proof_v3_binds_plugin_contracts_without_changing_v2() {
        let mut hello: MachineHello = serde_json::from_value(serde_json::json!({
            "machine_id": "falcon",
            "display_name": "Falcon",
            "platform": "linux",
            "arch": "x86_64",
            "connection_mode": "outbound_tls",
            "min_protocol": 1,
            "max_protocol": 7,
            "min_runtime_protocol": 1,
            "max_runtime_protocol": 3,
            "host_build": "test",
            "encryption_public_key": "machine-key"
        }))
        .unwrap();
        hello.plugin_contracts = Some(cowboy_plugin_sdk::PluginContractInventory::current_machine(
            cowboy_plugin_sdk::host::PLUGIN_HOST_SCHEMA_VERSION,
        ));
        let proof = challenge_proof_v3("id", "nonce", 42, &hello);
        let legacy_proof = challenge_proof_v2("id", "nonce", 42, &hello);

        let mut downgraded = hello.clone();
        downgraded
            .plugin_contracts
            .as_mut()
            .unwrap()
            .max_release_schema = 1;
        assert_ne!(proof, challenge_proof_v3("id", "nonce", 42, &downgraded));
        assert_eq!(
            legacy_proof,
            challenge_proof_v2("id", "nonce", 42, &downgraded)
        );

        let mut omitted = hello;
        omitted.plugin_contracts = None;
        assert_ne!(proof, challenge_proof_v3("id", "nonce", 42, &omitted));
    }
}

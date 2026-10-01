use super::*;

#[test]
fn execution_payload_cannot_downgrade_its_catalog_envelope() {
    let source: cowboy_provider_sdk::StandardProviderSource =
        serde_json::from_str(include_str!("../../../plugins/codex/provider.json")).unwrap();
    let provider = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
    let manifest: PluginManifest =
        serde_json::from_str(include_str!("../../../plugins/codex/plugin.json")).unwrap();
    let package = PluginPackage::new(
        manifest.clone(),
        manifest.component_release.clone(),
        PluginPayload::AgentProvider(Box::new(provider)),
    )
    .unwrap();
    assert_eq!(package.minimum_release_schema(), 4);
    let mut release = PluginRelease {
        release_schema: 4,
        plugin_id: manifest.id,
        plugin_version: manifest.version,
        plugin_kind: manifest.kind,
        package_digest: String::new(),
        artifact_digest: String::new(),
        artifact_url: String::new(),
        publisher: manifest.publisher,
        contract_fingerprint: package.contract_fingerprint.clone(),
        component_release: manifest.component_release,
        host_bundle_digest: None,
        signature: String::new(),
        supported_platforms: vec![],
        runtime_artifacts: vec![],
    };
    release.validate_envelope_for(&package).unwrap();
    release.host_bundle_digest = Some(format!("sha256:{}", "a".repeat(64)));
    release.validate_envelope_for(&package).unwrap();
    for older in [1, 2, 3] {
        release.release_schema = older;
        assert!(release.validate_envelope_for(&package).is_err());
    }
    let mut older_sdk = package.clone();
    older_sdk
        .manifest
        .components
        .iter_mut()
        .find(|c| c.id == "cowboy.plugin-sdk")
        .unwrap()
        .version = "1.10.0".into();
    release.release_schema = 4;
    assert!(release.validate_envelope_for(&older_sdk).is_err());
    let mut legacy = package;
    if let PluginPayload::AgentProvider(provider) = &mut legacy.payload {
        provider.manifest.runtime.behavior.execution = None;
    }
    assert_eq!(legacy.minimum_release_schema(), 1);
    assert!(release.validate_envelope_for(&legacy).is_err());
    release.release_schema = 2;
    release.validate_envelope_for(&legacy).unwrap();
}

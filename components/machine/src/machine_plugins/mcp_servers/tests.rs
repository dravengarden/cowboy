use super::*;
use base64::Engine as _;
use cowboy_plugin_sdk::*;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn desired(
    publisher: &crate::machine_auth::MachineIdentity,
    url: &str,
    script: &[u8],
) -> crate::machine_protocol::DesiredPlugin {
    let manifest = serde_json::from_value(json!({
        "schema_version":1,"id":"fixture-mcp","version":"1.0.0",
        "component_release":"3.79.0","publisher":"fixture-publisher",
        "kind":"mcp_server","entrypoint":"contract.json",
        "components":[{"id":"cowboy.plugin-contract","version":"1.11.0"},{"id":"cowboy.plugin-sdk","version":"1.13.0"}]
    }))
    .unwrap();
    let contract: McpServerContract = serde_json::from_value(json!({
        "schema_version":1,"id":"fixture-mcp","version":"1.0.0",
        "display_name":"Fixture MCP","description":"Signed test server",
        "supported_platforms":[{"os":"linux","architecture":"x86_64"}],
        "protocol":"stdio_session_v1","command":"fixture-mcp","arguments":["serve"],
        "tools":["fixture_read"],
        "discovery":[{"name":"fixture_read","description":"Read fixture","inputSchema":{"type":"object","properties":{}}}]
    }))
    .unwrap();
    let package = PluginPackage::new(
        manifest,
        "3.79.0".into(),
        PluginPayload::McpServer(contract.clone()),
    )
    .unwrap();
    let bytes = package.canonical_bytes().unwrap();
    let mut release: PluginRelease = serde_json::from_value(json!({
        "release_schema":6,"plugin_id":"fixture-mcp","plugin_version":"1.0.0",
        "plugin_kind":"mcp_server","package_digest":PluginPackage::artifact_digest(&bytes),
        "artifact_digest":"","artifact_url":"https://fixture.invalid/server.cowboy-plugin",
        "publisher":"fixture-publisher","contract_fingerprint":package.contract_fingerprint,
        "component_release":"3.79.0","signature":"",
        "supported_platforms":contract.supported_platforms,
        "runtime_artifacts":[{"os":"linux","architecture":"x86_64","components":[{
            "kind":"mcp_server","slot":"server","dependency":"fixture-mcp","version":"1.0.0",
            "command":"fixture-mcp","artifact_url":url,"artifact_digest":PluginPackage::artifact_digest(script),
            "artifact_format":"raw","probe":{"args":["--version"],"timeout_ms":1000}
        }]}]
    })).unwrap();
    release.artifact_digest = release.computed_artifact_digest().unwrap();
    release.signature = publisher
        .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
        .unwrap();
    release.validate_bytes(&bytes).unwrap();
    for older in 1..6 {
        let mut downgraded = release.clone();
        downgraded.release_schema = older;
        downgraded.artifact_digest = downgraded.computed_artifact_digest().unwrap();
        assert!(downgraded.validate_for(&package).is_err());
    }
    let mut wrong_command = release.clone();
    wrong_command.runtime_artifacts[0].components[0].command = "other-server".into();
    let mut missing_target = release.clone();
    missing_target.runtime_artifacts.clear();
    let mut extra_artifact = release.clone();
    let duplicate = extra_artifact.runtime_artifacts[0].components[0].clone();
    extra_artifact.runtime_artifacts[0]
        .components
        .push(duplicate);
    for mut invalid in [wrong_command, missing_target, extra_artifact] {
        invalid.artifact_digest = invalid.computed_artifact_digest().unwrap();
        assert!(invalid.validate_for(&package).is_err());
    }
    crate::machine_protocol::DesiredPlugin {
        release,
        package_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        publisher_public_key: publisher.public_key().to_owned(),
        host_bundle_base64: None,
    }
}

#[tokio::test]
async fn signed_mcp_installation_resolves_only_the_exact_verified_generation() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-mcp-test-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let publisher =
        crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher")).unwrap();
    let store = MachinePluginStore::new(
        &root.join("machine"),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let script = b"#!/bin/sh\nexit 0\n";
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/server", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_fixture_http_request(&mut stream).await;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    script.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(script).await.unwrap();
    });
    let desired = desired(&publisher, &url, script);
    let selection = McpServerSelection {
        plugin_id: "fixture-mcp".into(),
        generation_digest: desired.release.artifact_digest.clone(),
    };
    assert!(store.prepare_mcp_server(&selection).is_err());
    let installed = store.install(&desired).await.unwrap();
    server.await.unwrap();
    assert_eq!(installed.generation_digest, selection.generation_digest);
    let prepared = store.prepare_mcp_server(&selection).unwrap();
    assert_eq!(prepared.selection, selection);
    assert_eq!(prepared.contract.arguments, ["serve"]);
    assert!(prepared.executable.is_absolute());
    assert_eq!(std::fs::read(&prepared.executable).unwrap(), script);
    let wrong = McpServerSelection {
        generation_digest: format!("sha256:{}", "a".repeat(64)),
        ..selection.clone()
    };
    assert!(store.prepare_mcp_server(&wrong).is_err());
    // Installation must not implicitly expose the server to ordinary sessions.
    let descriptor = store
        .session_extensions(crate::runtime_wire::SessionExtensions {
            schema: crate::runtime_wire::SESSION_EXTENSIONS_SCHEMA,
            memory_tools: true,
            memory_recall: true,
        })
        .unwrap();
    assert!(descriptor.get("mcp").is_none());
    let mut binding = Binding {
        name: "fixture".into(),
        server: selection.clone(),
        tools: vec!["fixture_read".into()],
    };
    let bound = store.prepare_mcp_binding(&binding).unwrap();
    assert_eq!(bound.tools, ["fixture_read"]);
    assert_eq!(bound.server.selection, selection);
    let native = bound.native_descriptor(&"a".repeat(32)).unwrap();
    native.validate().unwrap();
    assert_eq!(native.binding.server, selection);
    assert!(bound.native_descriptor("latest").is_err());
    let discovery = bound.discovery().unwrap();
    assert_eq!(discovery["serverInfo"]["name"], "fixture-mcp");
    assert_eq!(
        discovery["tools"],
        json!([{"name":"fixture_read","description":"Read fixture","inputSchema":{"type":"object","properties":{}}}])
    );
    binding.tools.push("unsigned_execute".into());
    assert!(store.prepare_mcp_binding(&binding).is_err());
    std::fs::write(&prepared.executable, b"tampered").unwrap();
    assert!(store.prepare_mcp_server(&selection).is_err());
    std::fs::write(&prepared.executable, script).unwrap();
    store
        .uninstall(&selection.plugin_id, &selection.generation_digest)
        .await
        .unwrap();
    assert!(store.prepare_mcp_server(&selection).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
mod process;

async fn read_fixture_http_request(stream: &mut tokio::net::TcpStream) {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        assert!(request.len() < 4096, "fixture HTTP headers too large");
        request.push(stream.read_u8().await.unwrap());
    }
}

#[cfg(target_os = "linux")]
mod archive;

#[cfg(target_os = "linux")]
pub(crate) mod conformance;

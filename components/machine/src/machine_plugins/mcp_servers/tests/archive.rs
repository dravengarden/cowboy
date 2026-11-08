use super::*;
use crate::{
    machine_plugins::PluginExecutionScope,
    mcp_binding::{Binding, ProcessProfile},
};
use flate2::{Compression, write::GzEncoder};

#[tokio::test]
async fn signed_archive_server_reads_its_generation_resources() {
    let script = br#"#!/bin/sh
if [ "$1" = --version ]; then echo fixture; exit 0; fi
IFS= read -r resource < resource.json || exit 3
while IFS= read -r line; do
case "$line" in
*initialize*) echo '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}' ;;
*tools/call*) echo '{"jsonrpc":"2.0","id":2,"result":{"resource":'"$resource"'}}' ;;
esac
done
"#;
    let mut archive = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    for (name, bytes, mode) in [
        ("bundle/server", script.as_slice(), 0o755),
        (
            "bundle/resource.json",
            b"{\"generation\":\"fixture-v1\"}\n".as_slice(),
            0o444,
        ),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        archive.append_data(&mut header, name, bytes).unwrap();
    }
    let bytes = archive.into_inner().unwrap().finish().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/server.tar.gz", listener.local_addr().unwrap());
    let download = bytes.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_fixture_http_request(&mut stream).await;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    download.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&download).await.unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let publisher =
        crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
            .unwrap();
    let mut request = desired(&publisher, &url, &bytes);
    let artifact = &mut request.release.runtime_artifacts[0].components[0];
    artifact.artifact_format = PluginArtifactFormat::TarGz;
    artifact.entrypoint = Some("bundle/server".into());
    request.release.artifact_digest = request.release.computed_artifact_digest().unwrap();
    request.release.signature = publisher
        .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &request.release.proof())
        .unwrap();
    let store = MachinePluginStore::new(
        &root.path().join("machine"),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let installed = store.install(&request).await.unwrap();
    server.await.unwrap();
    let profile = ProcessProfile {
        schema: 1,
        service_id: format!("svc-{}", "a".repeat(32)),
        session_id: "archive-session".into(),
        runtime_machine: "ovh".into(),
        execution_machine: "hawk".into(),
        workspace_id: "fixture".into(),
        epoch: "b".repeat(32),
        binding: Binding {
            name: "fixture".into(),
            server: McpServerSelection {
                plugin_id: "fixture-mcp".into(),
                generation_digest: installed.generation_digest,
            },
            tools: vec!["fixture_read".into()],
        },
    };
    let scope = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
    let (invocation, owner) = scope.mcp(profile.clone()).unwrap();
    let process = store.open_mcp(invocation).await.unwrap();
    process
        .request(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await
        .unwrap();
    let reply = process
        .request(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"fixture_read"}}),
        )
        .await
        .unwrap();
    assert_eq!(
        reply["result"]["resource"],
        json!({"generation":"fixture-v1"})
    );
    drop(owner);
    assert!(process.request(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"fixture_read"}})).await.is_err());
}

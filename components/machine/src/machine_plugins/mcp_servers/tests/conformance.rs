//! Opt-in real artifact acceptance using disposable identities and storage.
use super::*;
use crate::{
    machine_plugins::PluginExecutionScope,
    mcp_binding::{Binding, ProcessProfile},
};

pub(crate) struct Installed {
    pub(crate) root: tempfile::TempDir,
    pub(crate) store: MachinePluginStore,
    pub(crate) profile: ProcessProfile,
    pub(crate) case: serde_json::Value,
}

pub(crate) async fn install() -> Installed {
    let case: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var("COWBOY_MCP_CONFORMANCE_CASE").expect("conformance case required"),
        )
        .unwrap(),
    )
    .unwrap();
    let read = |key: &str| std::fs::read(case[key].as_str().expect("local artifact path")).unwrap();
    let package_bytes = read("package");
    let package = PluginPackage::from_bytes(&package_bytes).unwrap();
    let mut release: PluginRelease = serde_json::from_slice(&read("release")).unwrap();
    release.validate_for(&package).unwrap();
    let archive = read("archive");
    assert_eq!(release.runtime_artifacts.len(), 1);
    assert_eq!(release.runtime_artifacts[0].components.len(), 1);
    assert_eq!(
        release.runtime_artifacts[0].components[0].artifact_digest,
        PluginPackage::artifact_digest(&archive)
    );
    let PluginPayload::McpServer(contract) = &package.payload else {
        panic!("MCP package required")
    };
    let tool = case["tool"].as_str().unwrap();
    assert!(contract.tools.iter().any(|name| name == tool));
    let (url, server) = serve_archive(archive).await;
    release.runtime_artifacts[0].components[0].artifact_url = url;
    let root = tempfile::tempdir().unwrap();
    let publisher =
        crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
            .unwrap();
    release.artifact_digest = release.computed_artifact_digest().unwrap();
    release.signature = publisher
        .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
        .unwrap();
    let request = crate::machine_protocol::DesiredPlugin {
        release,
        package_base64: base64::engine::general_purpose::STANDARD.encode(&package_bytes),
        publisher_public_key: publisher.public_key().to_owned(),
        host_bundle_base64: None,
    };
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
        session_id: "acceptance".into(),
        runtime_machine: "runtime".into(),
        execution_machine: "execution".into(),
        workspace_id: "fixture".into(),
        epoch: "b".repeat(32),
        binding: Binding {
            name: "acceptance".into(),
            server: McpServerSelection {
                plugin_id: package.manifest.id.clone(),
                generation_digest: installed.generation_digest,
            },
            tools: vec![tool.into()],
        },
    };
    Installed {
        root,
        store,
        profile,
        case,
    }
}

#[tokio::test]
#[ignore = "requires COWBOY_MCP_CONFORMANCE_CASE with local artifact paths and a read-only call"]
async fn external_signed_codeact_archive() {
    let Installed {
        root,
        store,
        profile,
        case,
    } = install().await;
    let tool = case["tool"].as_str().unwrap();
    let scope = PluginExecutionScope::new(Some(&profile.service_id), "execution");
    let (invocation, owner) = scope.mcp(profile).unwrap();
    let process = store.open_mcp(invocation).await.unwrap();
    process
        .request(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await
        .unwrap();
    process
        .request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await
        .unwrap();
    let list = process
        .request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await
        .unwrap();
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(list["result"]["tools"][0]["name"], tool);
    let call = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":tool,"arguments":case["arguments"]}});
    let response = process.request(call.clone()).await.unwrap();
    assert_ne!(response["result"]["isError"], true, "{response}");
    let content: serde_json::Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(content["state"], "ok", "{content}");
    assert_eq!(content["value"], case["expected"]);
    if let Some(arguments) = case.get("cancel_arguments") {
        cancel_active(&process, owner, root.path(), tool, arguments.clone()).await;
    } else {
        drop(owner);
    }
    assert!(process.request(call).await.is_err());
}

async fn serve_archive(archive: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/runtime.tar.gz", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_fixture_http_request(&mut stream).await;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    archive.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&archive).await.unwrap();
    });
    (url, server)
}

async fn cancel_active(
    process: &McpProcess,
    owner: crate::machine_plugins::McpOwner,
    root: &std::path::Path,
    tool: &str,
    arguments: serde_json::Value,
) {
    use tokio::time::{Duration, timeout};
    let call = process.request(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":tool,"arguments":arguments}}));
    tokio::pin!(call);
    let observe = async {
        loop {
            let pids = artifact_processes(root);
            if pids.len() >= 4 {
                break pids;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let pids = tokio::select! {
        result = &mut call => panic!("long-running call ended before cancellation: {result:?}"),
        pids = timeout(Duration::from_secs(3), observe) => pids.expect("sandbox was not observed"),
    };
    drop(owner);
    assert!(
        timeout(Duration::from_secs(2), &mut call)
            .await
            .unwrap()
            .is_err()
    );
    timeout(Duration::from_secs(2), async {
        while pids.iter().any(|pid| process_live(*pid)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("artifact descendants survived lease loss");
}

pub(crate) fn artifact_processes(root: &std::path::Path) -> Vec<u32> {
    let needle = root.to_string_lossy();
    let mut parents = std::collections::HashMap::new();
    let mut owned = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let stat = std::fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
        if let Some(parent) = stat
            .rsplit_once(") ")
            .and_then(|(_, fields)| fields.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u32>().ok())
        {
            parents.insert(pid, parent);
        }
        let bytes = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        if String::from_utf8_lossy(&bytes).contains(needle.as_ref()) {
            owned.insert(pid);
        }
    }
    loop {
        let before = owned.len();
        for (pid, parent) in &parents {
            if owned.contains(parent) {
                owned.insert(*pid);
            }
        }
        if owned.len() == before {
            break;
        }
    }
    owned.into_iter().collect()
}

pub(crate) fn process_live(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(") ")
            .is_some_and(|(_, fields)| !fields.starts_with('Z'))
    })
}

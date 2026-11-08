use super::*;
use crate::machine_plugins::PluginExecutionScope;
use crate::mcp_binding::{Binding, ProcessProfile};
use tokio::time::{Duration, timeout};

async fn installed() -> (MachinePluginStore, ProcessProfile, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let script = format!(r#"#!/bin/sh
if [ "$1" = --version ]; then echo fixture; exit 0; fi
echo "$$" > '{}/started'
IFS= read -r profile < "$COWBOY_MCP_PROFILE" || true
while IFS= read -r line; do
case "$line" in
*notifications/initialized*) : ;;
*initialize*) echo '{{"jsonrpc":"2.0","id":1,"result":{{"pid":'"$$"',"profile":'"$profile"',"capabilities":{{"roots":{{}}}}}}}}' ;;
*tools/list*) echo '{{"jsonrpc":"2.0","id":2,"result":{{"tools":[{{"name":"fixture_read","description":"Read fixture","inputSchema":{{"type":"object","properties":{{}}}}}},{{"name":"unsigned_execute"}}]}}}}' ;;
*malformed*) echo '{{"jsonrpc":"2.0","id":3,"result":null}}' ;;
*oversize*) /run/current-system/sw/bin/head -c 1048577 /dev/zero ;;
*slow*) /run/current-system/sw/bin/sleep 30 & echo "$!" > '{}/descendant'; wait "$!"; echo '{{"jsonrpc":"2.0","id":3,"result":{{"late":true}}}}' ;;
*tools/call*) echo '{{"jsonrpc":"2.0","id":3,"result":{{"ok":true}}}}' ;;
esac
done
"#, root.path().display(), root.path().display()).into_bytes();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/server", listener.local_addr().unwrap());
    let bytes = script.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_fixture_http_request(&mut stream).await;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(&bytes).await.unwrap();
    });
    let publisher =
        crate::machine_auth::MachineIdentity::load_or_create(&root.path().join("publisher"))
            .unwrap();
    let store = MachinePluginStore::new(
        &root.path().join("machine"),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let installed = store
        .install(&desired(&publisher, &url, &script))
        .await
        .unwrap();
    server.await.unwrap();
    let profile = ProcessProfile {
        schema: 1,
        service_id: format!("svc-{}", "a".repeat(32)),
        session_id: "session-one".into(),
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
    (store, profile, root)
}

async fn gone(pid: u64) {
    timeout(Duration::from_secs(3), async {
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"));
            if stat.as_ref().is_err()
                || stat.as_ref().is_ok_and(|value| {
                    value
                        .rsplit_once(") ")
                        .is_some_and(|(_, tail)| tail.starts_with('Z'))
                })
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("MCP process still running");
}

#[tokio::test]
async fn signed_stdio_process_projects_tools_and_holds_immutable_profile() {
    let (store, profile, _root) = installed().await;
    let connection = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
    let (invocation, _owner) = connection.mcp(profile.clone()).unwrap();
    let process = store.open_mcp(invocation).await.unwrap();
    let init = process
        .request(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await
        .unwrap();
    assert_eq!(init["result"]["profile"], json!(profile));
    assert_eq!(init["result"]["capabilities"], json!({"tools":{}}));
    let pid = init["result"]["pid"].as_u64().unwrap();
    process
        .request(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await
        .unwrap();
    let list = process
        .request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await
        .unwrap();
    assert_eq!(
        list["result"]["tools"],
        json!([{"name":"fixture_read","description":"Read fixture","inputSchema":{"type":"object","properties":{}}}])
    );
    assert!(process.request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"unsigned_execute"}})).await.is_err());
    gone(pid).await;
}

#[tokio::test]
async fn revoke_disconnect_and_cancel_reap_process_group_without_late_results() {
    for cause in ["revoke", "disconnect", "caller_cancel"] {
        let (store, profile, root) = installed().await;
        let connection = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
        let (invocation, owner) = connection.mcp(profile).unwrap();
        let process = store.open_mcp(invocation).await.unwrap();
        let pid = process
            .request(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
            .await
            .unwrap()["result"]["pid"]
            .as_u64()
            .unwrap();
        let read = tokio::spawn(async move {
            process.request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fixture_read","arguments":{"slow":true}}})).await
        });
        let descendant = timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(root.path().join("descendant"))
                    && let Ok(pid) = pid.trim().parse::<u64>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        match cause {
            "revoke" => owner.revoke(),
            "disconnect" => drop(connection),
            _ => read.abort(),
        }
        let result = timeout(Duration::from_secs(3), read).await.unwrap();
        assert!(
            result.is_err() || result.unwrap().is_err(),
            "late result escaped after {cause}"
        );
        gone(pid).await;
        gone(descendant).await;
    }
}

#[tokio::test]
async fn a_foreign_or_expired_connection_cannot_start_a_process() {
    let (store, profile, _root) = installed().await;
    assert!(
        PluginExecutionScope::new(Some(&profile.service_id), "ovh")
            .mcp(profile.clone())
            .is_err()
    );
    assert!(
        PluginExecutionScope::new(None, "hawk")
            .mcp(profile.clone())
            .is_err()
    );
    let scope = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
    let (invocation, owner) = scope.mcp(profile).unwrap();
    owner.revoke();
    assert!(store.open_mcp(invocation).await.is_err());
}

#[tokio::test]
async fn idle_owner_loss_client_drop_and_expiry_stop_the_process() {
    for cause in ["owner_drop", "client_drop", "expiry"] {
        let (store, profile, _root) = installed().await;
        let scope = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
        let (invocation, owner) = scope.mcp(profile).unwrap();
        let process = store.open_mcp(invocation).await.unwrap();
        let pid = process
            .request(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
            .await
            .unwrap()["result"]["pid"]
            .as_u64()
            .unwrap();
        match cause {
            "owner_drop" => drop(owner),
            "client_drop" => drop(process),
            _ => {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                tokio::time::resume();
            }
        }
        gone(pid).await;
    }
}

#[tokio::test]
async fn malformed_and_oversized_output_end_the_process_without_panicking() {
    for argument in ["malformed", "oversize"] {
        let (store, profile, _root) = installed().await;
        let scope = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
        let (invocation, _owner) = scope.mcp(profile).unwrap();
        let process = store.open_mcp(invocation).await.unwrap();
        let pid = process
            .request(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
            .await
            .unwrap()["result"]["pid"]
            .as_u64()
            .unwrap();
        let result = process.request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fixture_read","arguments":{(argument):true}}})).await;
        assert!(result.is_err());
        gone(pid).await;
    }
}

#[tokio::test]
async fn connection_session_handles_do_not_cross_owners_and_close_cancels_reads() {
    use crate::machine_plugins::McpSessions;
    let (store, profile, root) = installed().await;
    let scope = std::sync::Arc::new(PluginExecutionScope::new(Some(&profile.service_id), "hawk"));
    let replacement =
        std::sync::Arc::new(PluginExecutionScope::new(Some(&profile.service_id), "hawk"));
    let sessions = McpSessions::new(&scope);
    let foreign = McpSessions::new(&replacement);
    let fence = store.lifecycle.lock().await;
    for _ in 0..12 {
        let mut opening = Box::pin(sessions.open(&store, profile.clone()));
        tokio::select! {
            biased;
            result = &mut opening => panic!("launch bypassed fence or leaked capacity: {result:?}"),
            () = tokio::task::yield_now() => {},
        }
        drop(opening);
    }
    drop(fence);
    let handle = sessions.open(&store, profile.clone()).await.unwrap();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize"});
    assert!(foreign.request(&handle, initialize.clone()).await.is_err());
    assert!(foreign.renew(&handle, &profile).is_err());
    let mut wrong_profile = profile.clone();
    wrong_profile.epoch = "c".repeat(32);
    assert!(sessions.renew(&handle, &wrong_profile).is_err());
    sessions.renew(&handle, &profile).unwrap();
    foreign.close(&handle);
    let pid = sessions.request(&handle, initialize.clone()).await.unwrap()["result"]["pid"]
        .as_u64()
        .unwrap();
    {
        let read = sessions.request(&handle, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fixture_read","arguments":{"slow":true}}}));
        tokio::pin!(read);
        let descendant = async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(root.path().join("descendant"))
                    && let Ok(pid) = pid.trim().parse::<u64>()
                {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let child = tokio::select! {
            result = &mut read => panic!("slow read finished early: {result:?}"),
            pid = timeout(Duration::from_secs(3), descendant) => pid.unwrap(),
        };
        sessions.close(&handle);
        sessions.close(&handle);
        assert!(
            timeout(Duration::from_secs(3), &mut read)
                .await
                .unwrap()
                .is_err()
        );
        gone(pid).await;
        gone(child).await;
    }
    assert!(sessions.request(&handle, initialize.clone()).await.is_err());
    assert!(sessions.renew(&handle, &profile).is_err());
    let new_handle = sessions.open(&store, profile.clone()).await.unwrap();
    assert_ne!(new_handle, handle);
    let pid = sessions.request(&new_handle, initialize).await.unwrap()["result"]["pid"]
        .as_u64()
        .unwrap();
    drop(scope);
    gone(pid).await;
    assert!(sessions.open(&store, profile).await.is_err());
    assert!(
        sessions
            .request(
                &new_handle,
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
            )
            .await
            .is_err()
    );
    drop(sessions);
}

#[tokio::test]
async fn mcp_renewal_preserves_scope_and_cannot_restore_ended_authority() {
    let (_store, profile, _root) = installed().await;
    for cause in ["expiry", "revoke", "disconnect", "owner_drop"] {
        let scope = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
        let (invocation, owner) = scope.mcp(profile.clone()).unwrap();
        let mut changed = profile.clone();
        changed.epoch = "c".repeat(32);
        assert!(owner.renew(&changed).is_err());
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(20)).await;
        owner.renew(&profile).unwrap();
        tokio::time::advance(Duration::from_secs(11)).await;
        assert!(invocation.check().is_ok());
        match cause {
            "expiry" => {
                tokio::time::advance(Duration::from_secs(20)).await;
                assert!(owner.renew(&profile).is_err());
            }
            "revoke" => {
                owner.revoke();
                assert!(owner.renew(&profile).is_err());
            }
            "disconnect" => {
                drop(scope);
                assert!(owner.renew(&profile).is_err());
            }
            _ => drop(owner),
        }
        assert!(invocation.check().is_err());
        invocation.ended().await;
        tokio::time::resume();
    }
}

#[tokio::test]
async fn mcp_process_renewal_keeps_idle_io_but_does_not_extend_a_read() {
    let (store, profile, root) = installed().await;
    let scope = PluginExecutionScope::new(Some(&profile.service_id), "hawk");
    let (invocation, owner) = scope.mcp(profile.clone()).unwrap();
    let process = std::sync::Arc::new(store.open_mcp(invocation).await.unwrap());
    let init = process
        .request(json!({"jsonrpc":"2.0","id":1,"method":"initialize"}))
        .await
        .unwrap();
    let pid = init["result"]["pid"].as_u64().unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    owner.renew(&profile).unwrap();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    process
        .request(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .await
        .unwrap();
    owner.renew(&profile).unwrap();
    let client = process.clone();
    let read = tokio::spawn(async move {
        client
            .request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"fixture_read","arguments":{"slow":true}}}))
            .await
    });
    let descendant = timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(root.path().join("descendant"))
                && let Ok(pid) = value.trim().parse::<u64>()
            {
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    owner.renew(&profile).unwrap();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    let error = timeout(Duration::from_secs(3), read)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error}");
    gone(pid).await;
    gone(descendant).await;
    assert!(process.closed());
}

#[tokio::test]
async fn native_descriptors_are_verified_private_and_connection_owned() {
    use crate::machine_plugins::McpSessions;
    use std::os::unix::fs::PermissionsExt as _;
    let (store, profile, root) = installed().await;
    let scope = std::sync::Arc::new(PluginExecutionScope::new(Some(&profile.service_id), "hawk"));
    let sessions = McpSessions::new(&scope);
    let prepared = sessions
        .prepare_native(&store, profile.clone())
        .await
        .unwrap();
    assert!(
        !root.path().join("started").exists(),
        "discovery must not start the server"
    );
    let path = std::path::PathBuf::from(prepared["path"].as_str().unwrap());
    assert!(path.is_absolute());
    let meta = std::fs::symlink_metadata(&path).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let descriptor: crate::mcp_binding::NativeDescriptor =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    descriptor.validate().unwrap();
    assert_eq!(descriptor.binding, profile.binding);
    assert_eq!(descriptor.epoch, profile.epoch);
    assert_eq!(json!(descriptor), prepared["descriptor"]);
    let reused = sessions
        .prepare_native(&store, profile.clone())
        .await
        .unwrap();
    assert_eq!(reused["path"], prepared["path"]);
    assert_eq!(reused["descriptor"], prepared["descriptor"]);
    assert_ne!(reused["handle"], prepared["handle"]);
    let foreign_scope =
        std::sync::Arc::new(PluginExecutionScope::new(Some(&profile.service_id), "hawk"));
    let foreign = McpSessions::new(&foreign_scope);
    assert!(!foreign.release_native(prepared["handle"].as_str().unwrap()));
    assert!(path.exists());
    let mut invalid = profile.clone();
    invalid.binding.server.generation_digest = format!("sha256:{}", "0".repeat(64));
    assert!(sessions.prepare_native(&store, invalid).await.is_err());
    assert!(
        path.exists(),
        "failed preparation must preserve the last descriptor"
    );
    let mut replacement = profile.clone();
    replacement.epoch = "c".repeat(32);
    let changed = sessions.prepare_native(&store, replacement).await.unwrap();
    let changed_path = std::path::PathBuf::from(changed["path"].as_str().unwrap());
    assert!(
        path.exists(),
        "new preparation must not delete an older owner's metadata"
    );
    assert!(sessions.release_native(prepared["handle"].as_str().unwrap()));
    assert!(path.exists(), "the second receipt still owns this file");
    assert!(sessions.release_native(reused["handle"].as_str().unwrap()));
    assert!(!path.exists());
    assert!(!sessions.release_native(reused["handle"].as_str().unwrap()));
    assert!(changed_path.exists());
    let fence = store.lifecycle.lock().await;
    let mut pending = Box::pin(sessions.prepare_native(&store, profile));
    tokio::select! {
        biased;
        result = &mut pending => panic!("descriptor bypassed lifecycle fence: {result:?}"),
        () = tokio::task::yield_now() => {},
    }
    drop(scope);
    assert!(
        timeout(Duration::from_secs(3), &mut pending)
            .await
            .unwrap()
            .is_err()
    );
    drop(pending);
    drop(fence);
    drop(sessions);
    assert!(!changed_path.exists());
}

#[tokio::test]
async fn releasing_native_receipts_recovers_capacity_without_ending_the_connection() {
    use crate::machine_plugins::McpSessions;
    let (store, profile, _root) = installed().await;
    let scope = std::sync::Arc::new(PluginExecutionScope::new(Some(&profile.service_id), "hawk"));
    let sessions = McpSessions::new(&scope);
    let mut receipts = Vec::new();
    for _ in 0..256 {
        receipts.push(
            sessions
                .prepare_native(&store, profile.clone())
                .await
                .unwrap(),
        );
    }
    assert!(
        sessions
            .prepare_native(&store, profile.clone())
            .await
            .is_err()
    );
    let path = std::path::PathBuf::from(receipts[0]["path"].as_str().unwrap());
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt["path"] == receipts[0]["path"])
    );
    assert!(sessions.release_native(receipts.pop().unwrap()["handle"].as_str().unwrap()));
    receipts.push(
        sessions
            .prepare_native(&store, profile.clone())
            .await
            .unwrap(),
    );
    for receipt in receipts {
        assert!(sessions.release_native(receipt["handle"].as_str().unwrap()));
    }
    assert!(!path.exists());
    let fresh = sessions.prepare_native(&store, profile).await.unwrap();
    let fresh_path = std::path::PathBuf::from(fresh["path"].as_str().unwrap());
    assert!(fresh_path.exists());
    assert_ne!(fresh_path, path);
    drop(sessions);
    assert!(!fresh_path.exists());
}

#[tokio::test]
async fn unconfirmed_native_receipts_expire_without_another_request() {
    use crate::machine_plugins::McpSessions;
    let (store, profile, _root) = installed().await;
    let scope = std::sync::Arc::new(PluginExecutionScope::new(Some(&profile.service_id), "hawk"));
    let sessions = McpSessions::new(&scope);
    let abandoned = sessions
        .prepare_native(&store, profile.clone())
        .await
        .unwrap();
    let mut other = profile;
    other.epoch = "f".repeat(32);
    let owned = sessions.prepare_native(&store, other).await.unwrap();
    let abandoned_path = std::path::PathBuf::from(abandoned["path"].as_str().unwrap());
    let owned_path = std::path::PathBuf::from(owned["path"].as_str().unwrap());
    assert!(sessions.confirm_native(owned["handle"].as_str().unwrap()));
    tokio::time::pause();
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(32)).await;
    tokio::task::yield_now().await;
    assert!(!abandoned_path.exists());
    assert!(!sessions.confirm_native(abandoned["handle"].as_str().unwrap()));
    assert!(owned_path.exists());
    assert!(sessions.release_native(owned["handle"].as_str().unwrap()));
    assert!(!owned_path.exists());
    assert!(!sessions.confirm_native(owned["handle"].as_str().unwrap()));
}

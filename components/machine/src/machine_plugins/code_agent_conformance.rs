//! Actual installed Machine/portable-adapter automatic-owner lifecycle after
//! synthetic admission. This isolated fixture deliberately hides host cgroups;
//! resource admission itself requires separate containment acceptance.
use super::MachinePluginStore;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) async fn exercise(store: &MachinePluginStore, root: &Path) -> PathBuf {
    let worktree = root.join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(worktree.join("fixture.txt"), "real isolated Zed buffer\n").unwrap();
    std::fs::write(worktree.join("automatic.txt"), "automatic native owner\n").unwrap();
    let first = format!("cowboy-agent-{:064x}-{:032x}", 1, 1);
    let second = format!("cowboy-agent-{:064x}-{:032x}", 1, 2);
    let acquire = |owner: &str| {
        json!({"type":"acquireAgentBuffer", "ownerProtocol":2,
        "worktree":worktree, "path":"automatic.txt", "leaseId":owner})
    };
    let close = |owner: &str| {
        json!({"type":"closeBuffer", "worktree":worktree,
        "path":"automatic.txt", "leaseId":owner})
    };
    let mut refresh = acquire(&first);
    refresh["refreshOnly"] = true.into();
    let worktree_open = json!({"type":"openWorktree", "path":worktree, "trusted":true});
    store
        .code_request("zed", &worktree_open, None)
        .await
        .unwrap();
    let opened = store
        .code_runtimes
        .fixture_acquire_admitted("zed", &acquire(&first))
        .await;
    assert_eq!(opened["type"], "buffer", "automatic admission: {opened}");
    let renewed = store.code_request("zed", &refresh, None).await.unwrap();
    assert_eq!(renewed["type"], "agentBufferRefreshed");
    assert_eq!(renewed["leaseId"], first);
    assert_eq!(
        store
            .code_request("zed", &close(&first), None)
            .await
            .unwrap()["leases"],
        0
    );
    assert_eq!(
        store.code_request("zed", &refresh, None).await.unwrap()["state"],
        "unavailable"
    );
    assert_eq!(
        store
            .code_runtimes
            .fixture_acquire_admitted("zed", &acquire(&second))
            .await["leases"],
        1
    );
    assert_eq!(
        store.code_request("zed", &refresh, None).await.unwrap()["state"],
        "unavailable"
    );
    let language = json!({"type":"bufferLanguage", "worktree":worktree, "path":"automatic.txt"});
    assert_eq!(
        store.code_request("zed", &language, None).await.unwrap()["sourceState"],
        "matching"
    );
    assert_eq!(
        store
            .code_request("zed", &close(&second), None)
            .await
            .unwrap()["leases"],
        0
    );
    // Exercise the production maintenance pass, including original receipt
    // retirement. This task owns only disposable namespace fixtures.
    store
        .code_request(
            "zed",
            &json!({"type":"closeWorktree", "path":worktree}),
            None,
        )
        .await
        .unwrap();
    let maintenance = store.code_runtimes.start_agent_maintenance();
    let settled = tokio::time::timeout(Duration::from_secs(10), async {
        while store.code_runtimes.live_generation_count().await != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    maintenance.abort();
    let _ = maintenance.await;
    settled.expect("automatic receipt retirement did not drain");
    std::fs::remove_file(worktree.join("automatic.txt")).unwrap();
    println!(
        "installed automatic owners after synthetic resource admission: refresh never reopens released nonce, replacement survives old refresh, original prepared Close and maintenance retirement drained"
    );
    worktree
}

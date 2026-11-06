//! Lost or cancelled legacy opens retain the exact runtime, even before the
//! first successful response. These run through real private Unix sockets.
use super::*;
use serde_json::json;

#[tokio::test]
async fn exited_generation_can_restart_without_retargeting_another_old_lease() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let a = root.0.join("a");
    let b = root.0.join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let open_a = json!({"type":"openBuffer","worktree":a,"path":"file.rs","leaseId":"agent-a"});
    let open_b = json!({"type":"openBuffer","worktree":b,"path":"file.rs","leaseId":"agent-b"});
    on(&host, &root.0, "same", &open_a).await.unwrap();
    on(&host, &root.0, "same", &open_b).await.unwrap();
    let old = host
        .engines
        .lock()
        .await
        .values()
        .next()
        .unwrap()
        .upgrade()
        .unwrap();
    old.child.lock().start_kill().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while old.child.lock().try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        on(&host, &root.0, "same", &open_a).await.is_err(),
        "original route reports its loss first"
    );
    assert!(
        on(&host, &root.0, "same", &open_a).await.is_ok(),
        "an explicit reopen must not select the exited runtime again"
    );
    assert!(
        on(&host, &root.0, "same", &open_b).await.is_err(),
        "another old owner must not silently move to the replacement"
    );
    let new = host
        .engines
        .lock()
        .await
        .values()
        .next()
        .unwrap()
        .upgrade()
        .unwrap();
    assert!(!Arc::ptr_eq(&old, &new));
}

#[test]
fn malformed_confirmations_cannot_release_a_reserved_route() {
    let mut route = WorktreeRoute::default();
    let open = json!({"type":"openBuffer","path":"a.rs","leaseId":"agent"});
    route.dispatching(&open).unwrap();
    let close = json!({"type":"closeBuffer","path":"a.rs","leaseId":"agent"});
    for reply in [
        json!({}),
        json!({"type":"worktree","api_version":1,"leases":0}),
        json!({"type":"buffer","api_version":2,"leases":0}),
        json!({"type":"buffer","api_version":1,"leases":-1}),
    ] {
        assert!(route.observe(&close, &reply).is_err());
        assert!(!route.is_idle());
    }
    assert!(
        route
            .observe(&close, &json!({"type":"buffer","api_version":1,"leases":0}))
            .unwrap()
    );
    let open = json!({"type":"openWorktree"});
    route.dispatching(&open).unwrap();
    assert!(
        route
            .observe(&open, &json!({"type":"worktree","api_version":1}))
            .is_err()
    );
    assert!(!route.is_idle());
}

async fn on(
    host: &CodeRuntimeHost,
    root: &Path,
    generation: &str,
    request: &Value,
) -> Result<Value> {
    let plan = super::tests::fixture_plan(root, generation);
    host.request_active("fixture-code", request, Some(generation.into()), || {
        Ok(CodeRuntimeSelection::Installed(plan))
    })
    .await
}

#[tokio::test]
async fn lost_buffer_open_pins_original_runtime_until_its_owner_closes() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let open = json!({"type":"openBuffer","worktree":root.0,"path":"a.rs","leaseId":"agent"});
    assert!(on(&host, &root.0, "lost-legacy", &open).await.is_err());
    let ensure = json!({"type":"ensureWorktree","path":root.0,"trusted":true});
    assert_eq!(
        on(&host, &root.0, "new", &ensure).await.unwrap()["generation"],
        "generation-lost-legacy"
    );
    let other_close =
        json!({"type":"closeBuffer","worktree":root.0,"path":"a.rs","leaseId":"pane"});
    on(&host, &root.0, "new", &other_close).await.unwrap();
    assert_eq!(
        on(&host, &root.0, "new", &ensure).await.unwrap()["generation"],
        "generation-lost-legacy"
    );
    let mut close = open.clone();
    close["type"] = json!("closeBuffer");
    assert_eq!(
        on(&host, &root.0, "new", &close).await.unwrap()["generation"],
        "generation-lost-legacy"
    );
    assert_eq!(
        on(&host, &root.0, "new", &ensure).await.unwrap()["generation"],
        "generation-new"
    );
}

#[tokio::test]
async fn cancelled_buffer_open_keeps_the_route_while_native_handler_finishes() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = Arc::new(CodeRuntimeHost::default());
    let pending = {
        let host = host.clone();
        let path = root.0.clone();
        tokio::spawn(async move {
            on(
                &host,
                &path,
                "pause-legacy",
                &json!({"type":"openBuffer","worktree":path,"path":"a.rs","leaseId":"agent"}),
            )
            .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while !root.0.join("pause-legacy/legacy-requested").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    fs::write(root.0.join("pause-legacy/resume-legacy"), "resume").unwrap();
    let ensure = json!({"type":"ensureWorktree","path":root.0,"trusted":true});
    assert_eq!(
        on(&host, &root.0, "new", &ensure).await.unwrap()["generation"],
        "generation-pause-legacy"
    );
}

#[tokio::test]
async fn a_later_observation_does_not_resolve_a_lost_worktree_open() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let open = json!({"type":"openWorktree","path":root.0,"trusted":true});
    assert!(on(&host, &root.0, "lost-legacy", &open).await.is_err());
    let ensure = json!({"type":"ensureWorktree","path":root.0,"trusted":true});
    for _ in 0..2 {
        assert_eq!(
            on(&host, &root.0, "new", &ensure).await.unwrap()["generation"],
            "generation-lost-legacy"
        );
    }
    assert!(
        on(&host, &root.0, "new", &open)
            .await
            .unwrap_err()
            .to_string()
            .contains("unconfirmed")
    );
    let close = json!({"type":"closeWorktree","path":root.0});
    on(&host, &root.0, "new", &close).await.unwrap();
    assert_eq!(
        on(&host, &root.0, "new", &ensure).await.unwrap()["generation"],
        "generation-lost-legacy"
    );
}

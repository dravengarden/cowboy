use super::*;
use crate::machine_code_plugins::{
    CodeRuntimeHost, CodeRuntimeSelection, PrivateRuntimeDirectory, tests::fixture_plan,
};
use serde_json::json;

#[tokio::test]
async fn support_is_a_core_probe_not_a_native_health_or_installation_claim() {
    let host = CodeRuntimeHost::default();
    let probe = json!({"type":"bufferLeaseSupport"});
    let reply = host
        .request("fixture-code", &probe, || {
            panic!("support started a Plugin")
        })
        .await
        .unwrap();
    assert_eq!(reply, json!({"type":"bufferLeaseSupport", "api_version":1}));
    assert!(host.routes.lock().await.is_empty());
    assert!(host.buffer_leases.registry.lock().await.entries.is_empty());
    assert_eq!(host.buffer_leases.capacity.available_permits(), MAX_LEASES);
    // The old generic path-based dispatcher cannot forward this to an adapter
    // that might advertise support on behalf of an old Machine host.
    assert!(crate::machine_code_plugins::request_worktree(&probe).is_err());
    assert!(
        Command::parse(&json!({"type":"bufferLeaseSupport", "worktree":"/replacement"})).is_err()
    );
}

#[tokio::test]
async fn read_support_and_invalid_requests_never_select_a_plugin() {
    let host = CodeRuntimeHost::default();
    let probe = json!({"type":"bufferLeaseReadSupport"});
    assert_eq!(
        host.request("fixture-code", &probe, || panic!("selected a Plugin"))
            .await
            .unwrap(),
        json!({"type":"bufferLeaseReadSupport","api_version":1})
    );
    assert!(crate::machine_code_plugins::request_worktree(&probe).is_err());
    for value in [
        json!({"type":"bufferLeaseReadSupport","extra":true}),
        json!({"type":"readBufferLease","lease":{"instance":"a".repeat(32),"id":"0000000000000001"},"request":{"kind":"language","path":"other"}}),
        json!({"type":"readBufferLease","lease":{"instance":"a".repeat(32),"id":"0000000000000001"},"request":{"kind":"hover","offset":0}}),
    ] {
        assert!(
            host.request("fixture-code", &value, || panic!("selected a Plugin"))
                .await
                .is_err()
        );
    }
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn reads_retain_exact_generation_and_do_not_need_a_filesystem_path() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let old = prepare(&host, &root.0, "old").await;
    let read = |lease: &Value| json!({"type":"readBufferLease","lease":lease,"request":{"kind":"language"}});
    assert!(
        host.request("fixture-code", &read(&old), || panic!("selected a Plugin"))
            .await
            .is_err()
    );
    request(&host, &old, "openBufferLease").await.unwrap();
    let other = root.0.join("other");
    std::fs::create_dir(&other).unwrap();
    let new = prepare(&host, &other, "new").await;
    request(&host, &new, "openBufferLease").await.unwrap();
    std::fs::rename(&other, root.0.join("moved")).unwrap();
    for (lease, generation) in [(&old, "old"), (&new, "new")] {
        let observed = host
            .request("fixture-code", &read(lease), || {
                panic!("reselected a Plugin")
            })
            .await
            .unwrap();
        assert_eq!(
            observed["result"]["diagnostics"][0]["message"],
            format!("generation-{generation}")
        );
    }
    request(&host, &old, "releaseBufferLease").await.unwrap();
    assert!(
        host.request("fixture-code", &read(&old), || panic!(
            "reselected a Plugin"
        ))
        .await
        .is_err()
    );
    request(&host, &new, "releaseBufferLease").await.unwrap();
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn wrong_read_owner_does_not_retire_the_original_runtime() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "bad-read").await;
    request(&host, &lease, "openBufferLease").await.unwrap();
    let read = json!({"type":"readBufferLease","lease":lease,"request":{"kind":"symbols"}});
    assert!(
        host.request("fixture-code", &read, || panic!("reselected a Plugin"))
            .await
            .is_err()
    );
    assert_eq!(host.live_generation_count().await, 1);
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "open"
    );
    request(&host, &lease, "releaseBufferLease").await.unwrap();
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn failed_preparation_releases_an_unleased_runtime_and_capacity() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let plan = fixture_plan(&root.0, "bad-prepare");
    assert!(
        host.request(
            "fixture-code",
            &json!({"type":"prepareBuffer", "worktree":root.0, "path":"file.txt"}),
            || Ok(CodeRuntimeSelection::Installed(plan))
        )
        .await
        .is_err()
    );
    assert_eq!(host.live_generation_count().await, 0);
    assert_eq!(host.buffer_leases.capacity.available_permits(), MAX_LEASES);
}

#[tokio::test]
async fn cancelled_preparation_reaps_only_the_unleased_fixture_process() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let plan = fixture_plan(&root.0, "pause-prepare");
    let payload = json!({"type":"prepareBuffer", "worktree":root.0, "path":"file.txt"});
    let mut preparing = Box::pin(host.request("fixture-code", &payload, || {
        Ok(CodeRuntimeSelection::Installed(plan.clone()))
    }));
    let observed = async {
        for _ in 0..300 {
            if plan.home.join("prepare-requested").exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("native preparation was not dispatched");
    };
    tokio::select! { result = &mut preparing => panic!("preparation completed before cancellation: {result:?}"), () = observed => {} }
    let descendant = std::fs::read_to_string(plan.home.join("descendant.pid"))
        .unwrap()
        .parse()
        .unwrap();
    drop(preparing);
    assert_eq!(host.live_generation_count().await, 0);
    assert_eq!(host.buffer_leases.capacity.available_permits(), MAX_LEASES);
    crate::machine_code_plugins::tests::assert_process_stopped(descendant).await;
}

#[tokio::test]
async fn cancellation_during_expiry_keeps_the_cleanup_record() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "old").await;
    let route = {
        let mut registry = host.buffer_leases.registry.lock().await;
        let entry = registry.entries.values_mut().next().unwrap();
        entry.until = Some(Instant::now());
        Arc::clone(&entry.route)
    };
    let held = route.lock().await;
    let mut reaping = Box::pin(host.buffer_leases.reserve());
    tokio::select! { _ = &mut reaping => panic!("retired before obtaining cleanup lock"), () = tokio::time::sleep(Duration::from_millis(5)) => {} }
    drop(reaping);
    assert_eq!(host.buffer_leases.registry.lock().await.entries.len(), 1);
    drop(held);
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "released"
    );
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn missing_release_reply_keeps_evidence_until_an_exact_read() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "lost-close").await;
    request(&host, &lease, "openBufferLease").await.unwrap();
    assert!(request(&host, &lease, "releaseBufferLease").await.is_err());
    assert_eq!(host.buffer_leases.registry.lock().await.entries.len(), 1);
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "released"
    );
    assert_eq!(host.live_generation_count().await, 0);
}

async fn prepare(host: &CodeRuntimeHost, root: &std::path::Path, generation: &str) -> Value {
    let plan = fixture_plan(root, generation);
    host.request(
        "fixture-code",
        &json!({"type":"prepareBuffer", "worktree":root, "path":"file.txt"}),
        || Ok(CodeRuntimeSelection::Installed(plan)),
    )
    .await
    .unwrap()["lease"]
        .clone()
}

async fn request(host: &CodeRuntimeHost, lease: &Value, kind: &str) -> Result<Value> {
    host.request("fixture-code", &json!({"type":kind, "lease":lease}), || {
        panic!("retained native process was reselected")
    })
    .await
}

#[tokio::test]
async fn released_handle_does_not_resolve_deleted_paths_or_new_installations() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let worktree = root.0.join("worktree");
    std::fs::create_dir(&worktree).unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &worktree, "old").await;
    assert_eq!(
        request(&host, &lease, "openBufferLease").await.unwrap()["state"],
        "open"
    );
    std::fs::rename(&worktree, root.0.join("moved")).unwrap();
    assert_eq!(
        request(&host, &lease, "releaseBufferLease").await.unwrap()["state"],
        "released"
    );
    assert_eq!(host.live_generation_count().await, 0);
    for kind in ["queryBufferLease", "releaseBufferLease"] {
        assert_eq!(
            request(&host, &lease, kind).await.unwrap()["state"],
            "released"
        );
    }
    assert!(request(&host, &lease, "openBufferLease").await.is_err());
    std::fs::create_dir(&worktree).unwrap();
    let next = prepare(&host, &worktree, "new").await;
    assert_ne!(lease, next);
    assert_eq!(
        request(&host, &lease, "releaseBufferLease").await.unwrap()["state"],
        "released"
    );
    assert_eq!(
        request(&host, &next, "queryBufferLease").await.unwrap()["state"],
        "prepared"
    );
}

#[tokio::test]
async fn independent_worktrees_keep_their_own_native_generations() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let second = root.0.join("second");
    std::fs::create_dir(&second).unwrap();
    let host = CodeRuntimeHost::default();
    let old = prepare(&host, &root.0, "old").await;
    let new = prepare(&host, &second, "new").await;
    assert_ne!(old["instance"], new["instance"]);
    assert_eq!(host.live_generation_count().await, 2);
    request(&host, &old, "openBufferLease").await.unwrap();
    request(&host, &old, "releaseBufferLease").await.unwrap();
    assert_eq!(host.live_generation_count().await, 1);
    assert_eq!(
        request(&host, &new, "openBufferLease").await.unwrap()["state"],
        "open"
    );
    request(&host, &new, "releaseBufferLease").await.unwrap();
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn missing_open_reply_retains_original_process_for_read_only_observation() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "lost").await;
    assert!(request(&host, &lease, "openBufferLease").await.is_err());
    assert_eq!(host.live_generation_count().await, 1);
    assert!(
        host.buffer_leases
            .registry
            .lock()
            .await
            .entries
            .values()
            .all(|entry| entry.until.is_none())
    );
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "open"
    );
    request(&host, &lease, "releaseBufferLease").await.unwrap();
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn cancellation_after_native_open_does_not_expire_or_replay_its_effect() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let plan = fixture_plan(&root.0, "paused");
    let lease = prepare(&host, &root.0, "paused").await;
    let mut opening = Box::pin(request(&host, &lease, "openBufferLease"));
    let observed = async {
        for _ in 0..300 {
            if plan.home.join("open-requested").exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("native open was not dispatched");
    };
    tokio::select! { result = &mut opening => panic!("open completed before cancellation: {result:?}"), () = observed => {} }
    drop(opening);
    assert!(
        host.buffer_leases
            .registry
            .lock()
            .await
            .entries
            .values()
            .all(|entry| entry.until.is_none())
    );
    std::fs::write(plan.home.join("resume-open"), "resume").unwrap();
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "open"
    );
    request(&host, &lease, "releaseBufferLease").await.unwrap();
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn wrong_native_release_reply_keeps_the_original_lease() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "mismatch").await;
    request(&host, &lease, "openBufferLease").await.unwrap();
    assert!(
        request(&host, &lease, "releaseBufferLease")
            .await
            .unwrap_err()
            .to_string()
            .contains("reference")
    );
    assert_eq!(host.live_generation_count().await, 1);
    assert_eq!(host.buffer_leases.registry.lock().await.entries.len(), 1);
    // Only a subsequent exact read may observe that the native owner released.
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "released"
    );
    assert_eq!(host.live_generation_count().await, 0);
}

#[tokio::test]
async fn dead_process_handle_never_targets_replacement_on_the_same_worktree() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "old").await;
    request(&host, &lease, "openBufferLease").await.unwrap();
    let runtime = Arc::clone(
        &host
            .buffer_leases
            .registry
            .lock()
            .await
            .entries
            .values()
            .next()
            .unwrap()
            .runtime,
    );
    let descendant =
        std::fs::read_to_string(fixture_plan(&root.0, "old").home.join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap();
    runtime.child.lock().start_kill().unwrap();
    for _ in 0..300 {
        if runtime.child.lock().try_wait().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(runtime.child.lock().try_wait().unwrap().is_some());
    // The ordinary route detects the old death once, without replaying.
    assert!(
        host.request(
            "fixture-code",
            &json!({"type":"prepareBuffer", "worktree":root.0, "path":"file.txt"}),
            || panic!("old route reselected too early")
        )
        .await
        .is_err()
    );
    let next = prepare(&host, &root.0, "new").await;
    assert!(
        request(&host, &lease, "releaseBufferLease")
            .await
            .unwrap_err()
            .to_string()
            .contains("original buffer runtime exited")
    );
    crate::machine_code_plugins::tests::assert_process_stopped(descendant).await;
    assert!(runtime.process_group.lock().is_none());
    assert_eq!(
        request(&host, &next, "queryBufferLease").await.unwrap()["state"],
        "prepared"
    );
    request(&host, &next, "releaseBufferLease").await.unwrap();
    assert_eq!(host.buffer_leases.registry.lock().await.entries.len(), 1);
}

#[tokio::test]
async fn effect_free_expiry_releases_capacity_without_native_open() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "old").await;
    for entry in host
        .buffer_leases
        .registry
        .lock()
        .await
        .entries
        .values_mut()
    {
        entry.until = Some(Instant::now());
    }
    assert!(request(&host, &lease, "openBufferLease").await.is_err());
    assert_eq!(host.live_generation_count().await, 0);
    assert_eq!(host.buffer_leases.capacity.available_permits(), MAX_LEASES);
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "released"
    );
}

#[tokio::test]
async fn pending_preparations_also_count_against_capacity() {
    let routes = Routes::default();
    let mut reserved = Vec::new();
    for _ in 0..MAX_LEASES {
        reserved.push(routes.reserve().await.unwrap());
    }
    assert!(routes.reserve().await.is_err());
    reserved.pop();
    assert!(routes.reserve().await.is_ok());
    drop(reserved);
    assert_eq!(routes.capacity.available_permits(), MAX_LEASES);
}

#[tokio::test]
async fn unknown_or_foreign_plugin_reference_does_not_select_a_runtime() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let lease = prepare(&host, &root.0, "old").await;
    assert!(
        host.request(
            "different-code",
            &json!({"type":"releaseBufferLease", "lease":lease}),
            || panic!("foreign lease selected an engine")
        )
        .await
        .is_err()
    );
    assert_eq!(
        request(&host, &lease, "queryBufferLease").await.unwrap()["state"],
        "prepared"
    );
}

#[test]
fn malformed_or_retargeting_commands_fail_closed() {
    let lease = json!({"instance":"a".repeat(32), "id":"0000000000000001"});
    for payload in [
        json!({"type":"openBufferLease", "lease":lease, "worktree":"/new"}),
        json!({"type":"releaseBufferLease", "lease":{"instance":"a", "id":"1"}}),
        json!({"type":"queryBufferLease", "lease":{"instance":"a".repeat(32), "id":1}}),
        json!({"type":"prepareBuffer", "worktree":"a".repeat(4_097), "path":"a.txt"}),
    ] {
        assert!(Command::parse(&payload).is_err());
    }
}

use super::*;
use serde_json::json;

const OWNER: &str = "cowboy-agent-3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175-00000000000000000000000000000001";

#[tokio::test]
async fn refresh_never_opens_reclaimed_unknown_or_releasing_owners() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let (target, generation) = host
        .target(CodeRuntimeSelection::Installed(
            super::super::tests::fixture_plan(&root.0, "refresh-agent"),
        ))
        .await
        .unwrap();
    // A successful refresh must not exchange any native request at all.
    fs::remove_file(target.socket()).unwrap();
    let key = ("file.rs".to_owned(), OWNER.to_owned());
    let mut held = Holding::opening(root.0.to_str().unwrap(), true);
    held.confirm();
    held.last_used = Instant::now() - IDLE;
    let mut route = WorktreeRoute {
        target: Some(target),
        generation,
        ..Default::default()
    };
    route.buffers.insert(key.clone());
    route.agent_buffers.insert(key.clone(), held);
    let route = Arc::new(Mutex::new(route));
    host.routes
        .lock()
        .await
        .insert(("zed".into(), root.0.clone()), Arc::clone(&route));
    let payload = json!({"type":"acquireAgentBuffer", "ownerProtocol":2, "refreshOnly":true,
        "worktree":root.0, "path":"file.rs", "leaseId":OWNER});
    let reply = host.refresh_agent("zed", &root.0, &payload).await.unwrap();
    assert_eq!(reply["type"], "agentBufferRefreshed");
    assert_eq!(reply["leaseId"], OWNER);
    assert!(route.lock().await.agent_buffers[&key].last_used.elapsed() < IDLE);
    route
        .lock()
        .await
        .agent_buffers
        .get_mut(&key)
        .unwrap()
        .confirmed = false;
    assert_eq!(
        host.refresh_agent("zed", &root.0, &payload).await.unwrap()["state"],
        "unavailable"
    );
    {
        let mut r = route.lock().await;
        let h = r.agent_buffers.get_mut(&key).unwrap();
        h.confirm();
        h.releasing = true;
    }
    assert_eq!(
        host.refresh_agent("zed", &root.0, &payload).await.unwrap()["state"],
        "unavailable"
    );
    assert_eq!(
        route.lock().await.agent_buffers.len(),
        1,
        "unknown ownership remains charged"
    );
    route.lock().await.agent_buffers.remove(&key);
    assert_eq!(
        host.refresh_agent("zed", &root.0, &payload).await.unwrap()["state"],
        "unavailable"
    );
    assert!(
        route.lock().await.agent_buffers.is_empty(),
        "refresh did not recreate the owner"
    );
    host.routes.lock().await.clear();
    assert_eq!(
        host.refresh_agent("zed", &root.0, &payload).await.unwrap()["state"],
        "unavailable"
    );
    assert!(host.routes.lock().await.is_empty());
}

#[tokio::test]
async fn refresh_requires_explicit_version_two_true_without_runtime_selection() {
    let host = CodeRuntimeHost::default();
    for (protocol, refresh) in [
        (json!(2), json!(false)),
        (json!(2), json!("true")),
        (json!(1), json!(true)),
    ] {
        let payload = json!({"type":"acquireAgentBuffer", "ownerProtocol":protocol, "refreshOnly":refresh,
            "worktree":"/tmp", "path":"file.rs", "leaseId":OWNER});
        assert!(
            host.acquire_agent(
                "zed",
                &payload,
                None,
                resource_budget::Policy::from_config(None),
                || panic!("invalid refresh cannot select a runtime")
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn passive_observation_cannot_start_a_backend_before_acquisition_admission() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let plan = super::super::tests::fixture_plan(&root.0, "passive-cold");
    for kind in [
        "bufferLanguage",
        "bufferHover",
        "bufferNavigate",
        "bufferSymbols",
    ] {
        let request = json!({"type":kind, "worktree":root.0, "path":"file.rs"});
        let error = host
            .request("fixture-code", &request, || {
                Ok(CodeRuntimeSelection::Installed(plan.clone()))
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not open"), "{error}");
        assert!(host.engines.lock().await.is_empty());
        for route in host.routes.lock().await.values() {
            assert!(route.lock().await.target.is_none());
        }
    }
}

#[tokio::test]
async fn invalid_files_refuse_before_runtime_selection_or_owner_reservation() {
    use std::os::unix::fs::symlink;
    let root = PrivateRuntimeDirectory::create().unwrap();
    fs::create_dir(root.0.join("src")).unwrap();
    fs::write(root.0.join("src/good.rs"), "fn main() {}\n").unwrap();
    symlink("src", root.0.join("alias")).unwrap();
    symlink("src/good.rs", root.0.join("linked.rs")).unwrap();
    let host = CodeRuntimeHost::default();
    for path in [
        "missing.rs",
        "src",
        "alias/good.rs",
        "linked.rs",
        "../outside.rs",
        "/etc/passwd",
    ] {
        let payload = json!({"type":"acquireAgentBuffer", "ownerProtocol":2, "worktree":root.0, "path":path, "leaseId":OWNER});
        let reply = host
            .acquire_agent(
                "zed",
                &payload,
                None,
                resource_budget::Policy::from_config(None),
                || panic!("invalid input must not start a runtime"),
            )
            .await
            .unwrap();
        assert_eq!(reply["type"], "agentBufferBudget");
        assert_eq!(reply["state"], "unavailable");
        assert!(host.routes.lock().await.is_empty());
    }
    assert!(regular_file(&root.0, "src/good.rs"));
}

#[test]
fn uncertain_effects_and_aliases_never_free_or_reuse_an_owner() {
    let mut legacy = Holding::opening("/tree", false);
    legacy.confirm();
    legacy.last_used = Instant::now() - IDLE;
    assert!(
        !legacy.begin_release(),
        "rollback Controllers retain their own cleanup authority"
    );
    assert_eq!(
        owner("cowboy-agent-3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175-00000000000000000000000000000001", true).unwrap(),
        "3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175"
    );
    assert_eq!(
        owner("cowboy-agent-3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175-00000000000000000000000000000002", true).unwrap(),
        "3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175"
    );
    assert_eq!(
        owner("cowboy-agent-session", false).unwrap(),
        "3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175"
    );
    let mut held = Holding::opening("/original-link", true);
    held.last_used = Instant::now() - IDLE;
    assert!(
        !held.begin_release(),
        "unknown Open cannot be guessed complete"
    );
    held.confirm();
    assert!(!held.begin_release(), "fresh observations retain ownership");
    held.last_used = Instant::now() - IDLE;
    assert!(held.begin_release());
    assert!(!held.begin_release(), "unknown Close is not replayed");
    assert!(held.refresh("/original-link").is_err());
    assert!(
        held.dispatching(&json!({"type":"closeBuffer", "worktree":"/resolved"}))
            .is_err()
    );
    assert!(
        held.dispatching(&json!({"type":"openBuffer", "worktree":"/original-link"}))
            .is_err()
    );
    assert!(
        held.dispatching(&json!({"type":"closeBuffer", "worktree":"/original-link"}))
            .is_err()
    );
}

#[tokio::test]
async fn nonces_do_not_reset_the_session_share_after_controller_restart() {
    let host = CodeRuntimeHost::default();
    let mut route = WorktreeRoute::default();
    for i in 0..4 {
        let lease = format!(
            "cowboy-agent-3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175-{i:032x}"
        );
        route.agent_buffers.insert(
            (format!("file{i}"), lease.clone()),
            Holding::opening("/tree", true),
        );
    }
    host.routes
        .lock()
        .await
        .insert(("zed".into(), "/tree".into()), Arc::new(Mutex::new(route)));
    assert!(
        !host
            .agent_holding_capacity(
                "zed",
                Path::new("/tree"),
                &("next".into(), format!("cowboy-agent-3f3af1ecebbd1410ab417ec0d27bbfcb5d340e177ae159b59fc8626c2dfd9175-{:032x}", 5)), true)
            .await
            .unwrap()
    );
    assert!(
        !host
            .agent_holding_capacity(
                "zed",
                Path::new("/tree"),
                &("rollback".into(), "cowboy-agent-session".into()),
                false
            )
            .await
            .unwrap(),
        "rolling back the Controller must not reset the session share"
    );
    assert!(
        host.agent_holding_capacity(
            "zed",
            Path::new("/tree"),
            &("next".into(), format!("cowboy-agent-d9298a10d1b0735837dc4bd85dac641b0f3cef27a47e5d53a54f2f3f5b2fcffa-{:032x}", 5)), true)
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn legacy_request_cannot_opt_into_reaping_by_imitating_a_nonce() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let (target, generation) = host
        .target(CodeRuntimeSelection::Installed(
            super::super::tests::fixture_plan(&root.0, "legacy-owner"),
        ))
        .await
        .unwrap();
    let socket = target.socket().to_owned();
    let mut route = WorktreeRoute {
        target: Some(target),
        generation,
        ..Default::default()
    };
    let payload =
        json!({"type":"acquireAgentBuffer", "worktree":root.0, "path":"a.rs", "leaseId":OWNER});
    host.acquire_admitted(&mut route, &socket, &payload)
        .await
        .unwrap();
    route.agent_buffers.values_mut().next().unwrap().last_used = Instant::now() - IDLE;
    let route = Arc::new(Mutex::new(route));
    host.routes
        .lock()
        .await
        .insert(("fixture-code".into(), root.0.clone()), Arc::clone(&route));
    host.release_idle_agents().await;
    assert_eq!(route.lock().await.agent_buffers.len(), 1);
}

#[tokio::test]
async fn unknown_owner_protocol_refuses_before_runtime_selection() {
    let host = CodeRuntimeHost::default();
    let payload = json!({"type":"acquireAgentBuffer", "ownerProtocol":3, "worktree":"/tmp", "path":"a.rs", "leaseId":OWNER});
    let result = host
        .acquire_agent(
            "zed",
            &payload,
            None,
            resource_budget::Policy::from_config(None),
            || panic!("invalid protocol must not select a runtime"),
        )
        .await;
    assert!(result.is_err());
    assert!(host.routes.lock().await.is_empty());
}

#[tokio::test]
async fn machine_limits_survive_controller_absence_and_count_unknowns() {
    let host = CodeRuntimeHost::default();
    for n in 0..2 {
        let tree = format!("/tree{n}");
        let mut route = WorktreeRoute::default();
        for i in 0..4 {
            route.agent_buffers.insert(
                (format!("file{i}"), format!("owner{n}")),
                Holding::opening(&tree, false),
            );
        }
        host.routes
            .lock()
            .await
            .insert(("zed".into(), tree.into()), Arc::new(Mutex::new(route)));
    }
    assert!(
        !host
            .agent_holding_capacity(
                "zed",
                Path::new("/tree0"),
                &("extra".into(), "owner0".into()),
                false
            )
            .await
            .unwrap()
    );
    assert!(
        !host
            .agent_holding_capacity(
                "zed",
                Path::new("/tree2"),
                &("file".into(), "new".into()),
                false
            )
            .await
            .unwrap()
    );
    assert!(
        host.agent_holding_capacity(
            "zed",
            Path::new("/tree0"),
            &("file0".into(), "owner0".into()),
            false
        )
        .await
        .unwrap()
    );
}

#[tokio::test]
async fn idle_machine_cleanup_uses_original_runtime_and_preserves_interactive_owner() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let (target, generation) = host
        .target(CodeRuntimeSelection::Installed(
            super::super::tests::fixture_plan(&root.0, "idle-agent"),
        ))
        .await
        .unwrap();
    let socket = target.socket().to_owned();
    let mut route = WorktreeRoute {
        target: Some(target),
        generation,
        ..Default::default()
    };
    let payload = json!({"type":"acquireAgentBuffer", "ownerProtocol":2, "worktree":root.0, "path":"a.rs", "leaseId":OWNER});
    host.acquire_admitted(&mut route, &socket, &payload)
        .await
        .unwrap();
    route.buffers.insert(("a.rs".into(), "interactive".into()));
    route.agent_buffers.values_mut().next().unwrap().last_used = Instant::now() - IDLE;
    let route = Arc::new(Mutex::new(route));
    host.routes
        .lock()
        .await
        .insert(("fixture-code".into(), root.0.clone()), Arc::clone(&route));
    // No Controller object exists: the Machine owns this lifetime.
    host.release_idle_agents().await;
    let held = route.lock().await;
    assert!(held.agent_buffers.is_empty());
    assert!(!held.buffers.contains(&("a.rs".into(), OWNER.into())));
    assert!(
        held.buffers
            .contains(&("a.rs".into(), "interactive".into()))
    );
    assert!(held.target.is_some());
}

#[tokio::test]
async fn lost_idle_close_stays_counted_and_cannot_be_replayed_or_reopened() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let host = CodeRuntimeHost::default();
    let plan = super::super::tests::fixture_plan(&root.0, "lost-agent-close");
    let marker = plan.home.join("close-count");
    let (target, generation) = host
        .target(CodeRuntimeSelection::Installed(plan))
        .await
        .unwrap();
    let socket = target.socket().to_owned();
    let mut route = WorktreeRoute {
        target: Some(target),
        generation,
        ..Default::default()
    };
    let payload = json!({"type":"acquireAgentBuffer", "ownerProtocol":2, "worktree":root.0, "path":"a.rs", "leaseId":OWNER});
    host.acquire_admitted(&mut route, &socket, &payload)
        .await
        .unwrap();
    route.agent_buffers.values_mut().next().unwrap().last_used = Instant::now() - IDLE;
    let route = Arc::new(Mutex::new(route));
    host.routes
        .lock()
        .await
        .insert(("fixture-code".into(), root.0.clone()), Arc::clone(&route));
    host.release_idle_agents().await;
    host.release_idle_agents().await;
    assert_eq!(fs::read_to_string(marker).unwrap(), "1");
    let mut route = route.lock().await;
    assert_eq!(route.agent_buffers.len(), 1);
    assert!(!route.is_idle());
    assert!(
        host.acquire_admitted(&mut route, &socket, &payload)
            .await
            .is_err()
    );
    let mut ordinary_open = payload;
    ordinary_open["type"] = json!("openBuffer");
    assert!(route.dispatching(&ordinary_open).is_err());
}

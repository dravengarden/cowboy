use super::super::{
    CodeRuntimeSelection, CodeTarget, PrivateRuntimeDirectory, tests::fixture_plan,
};
use super::*;
use std::{fs, path::PathBuf};
mod fixture;
pub(crate) use fixture::Fixture;

async fn setup(
    root: &Path,
    mode: &str,
) -> (
    CodeRuntimeHost,
    Arc<RunningCodeRuntime>,
    WorktreeRoute,
    Value,
    PathBuf,
) {
    let host = CodeRuntimeHost::default();
    let plan = fixture_plan(root, mode);
    let home = plan.home.clone();
    let (target, generation) = host
        .target(CodeRuntimeSelection::Installed(plan))
        .await
        .unwrap();
    let CodeTarget::Installed(runtime) = &target else {
        panic!("not installed")
    };
    let runtime = runtime.clone();
    let route = WorktreeRoute {
        target: Some(target),
        generation,
        ..Default::default()
    };
    let payload = json!({"type":"acquireAgentBuffer","worktree":root,"path":"file","leaseId":format!("cowboy-agent-{:064x}-{:032x}",1,1),"ownerProtocol":2});
    (host, runtime, route, payload, home)
}

fn log(home: &Path) -> Vec<Value> {
    fs::read_to_string(home.join("acquisition-log"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn lost_replies_keep_original_reference_and_never_repeat_effects() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, payload, home) = setup(&root.0, "acquisition-lost").await;
    // Lost prepare can only repeat the idempotent, effect-free preparation.
    assert!(
        host.acquire_admitted(&mut route, &runtime.socket, &payload)
            .await
            .is_err()
    );
    assert!(
        route
            .agent_buffers
            .values()
            .next()
            .unwrap()
            .acquisition
            .as_ref()
            .unwrap()
            .reference
            .is_none()
    );
    assert!(
        host.acquire_admitted(&mut route, &runtime.socket, &payload)
            .await
            .is_err()
    );
    let original = route
        .agent_buffers
        .values()
        .next()
        .unwrap()
        .acquisition
        .as_ref()
        .unwrap()
        .reference
        .clone()
        .unwrap();
    fs::write(home.join("foreign-query"), "yes").unwrap();
    assert!(
        host.acquire_admitted(&mut route, &runtime.socket, &payload)
            .await
            .is_err()
    );
    fs::remove_file(home.join("foreign-query")).unwrap();
    let ready = host
        .acquire_admitted(&mut route, &runtime.socket, &payload)
        .await
        .unwrap();
    assert_eq!(ready["buffer_id"], 77);
    let mut close = payload.clone();
    close["type"] = json!("closeBuffer");
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .is_err()
    );
    assert_eq!(route.agent_buffers.len(), 1);
    host.release_agent(&mut route, &runtime, &close)
        .await
        .unwrap()
        .unwrap();
    assert!(route.agent_buffers.is_empty());
    assert!(route.target.is_none());
    host.agent_acquisitions.observe().await;
    host.agent_acquisitions.observe().await;
    let requests = log(&home);
    for action in ["execute", "release"] {
        assert_eq!(requests.iter().filter(|r| r["action"] == action).count(), 1);
    }
    assert_eq!(
        requests.iter().filter(|r| r["action"] == "retire").count(),
        2
    );
    for request in requests.iter().filter(|r| r["type"] == "agentAcquisition") {
        assert_eq!(
            request["acquisition"],
            serde_json::to_value(&original).unwrap()
        );
    }
    assert!(!requests.iter().any(|r| matches!(
        r["type"].as_str(),
        Some("ensureWorktree" | "openBuffer" | "closeBuffer")
    )));
}

#[tokio::test]
async fn native_support_errors_never_fall_back_or_reserve_an_owner() {
    for mode in ["acquisition-unsupported", "acquisition-malformed"] {
        let root = PrivateRuntimeDirectory::create().unwrap();
        let (host, runtime, mut route, payload, home) = setup(&root.0, mode).await;
        assert!(
            host.acquire_admitted(&mut route, &runtime.socket, &payload)
                .await
                .is_err()
        );
        assert!(route.agent_buffers.is_empty());
        assert!(route.buffers.is_empty());
        assert_eq!(log(&home).len(), 1);
    }
}

#[tokio::test]
async fn unfinished_worktree_acquisition_can_be_released_without_reexecution() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, payload, home) = setup(&root.0, "acquisition-pending").await;
    for _ in 0..2 {
        assert!(
            host.acquire_admitted(&mut route, &runtime.socket, &payload)
                .await
                .is_err()
        );
    }
    let mut close = payload.clone();
    close["type"] = json!("closeBuffer");
    host.release_agent(&mut route, &runtime, &close)
        .await
        .unwrap()
        .unwrap();
    assert!(route.agent_buffers.is_empty());
    let requests = log(&home);
    assert_eq!(
        requests.iter().filter(|r| r["action"] == "execute").count(),
        1
    );
    assert_eq!(
        requests.iter().filter(|r| r["action"] == "query").count(),
        1
    );
    assert_eq!(
        requests.iter().filter(|r| r["action"] == "release").count(),
        1
    );
}

#[tokio::test]
async fn exhausted_receipt_capacity_does_not_leave_an_unreclaimable_owner() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, payload, home) = setup(&root.0, "acquisition-capacity").await;
    let permits: Vec<_> = (0..1024)
        .map(|_| host.agent_acquisitions.reserve().unwrap())
        .collect();
    assert!(
        host.acquire_admitted(&mut route, &runtime.socket, &payload)
            .await
            .is_err()
    );
    assert!(route.agent_buffers.is_empty());
    assert!(route.buffers.is_empty());
    assert_eq!(log(&home).len(), 1);
    drop(permits);
    assert_eq!(
        host.acquire_admitted(&mut route, &runtime.socket, &payload)
            .await
            .unwrap()["buffer_id"],
        77
    );
}

#[tokio::test]
async fn private_acquisition_protocol_is_not_generic_rpc_authority() {
    let host = CodeRuntimeHost::default();
    for kind in [
        "agentAcquisitionSupport",
        "prepareAgentAcquisition",
        "agentAcquisition",
    ] {
        assert!(
            host.request("zed", &json!({"type":kind}), || panic!("must not select"))
                .await
                .is_err()
        );
    }
    assert!(host.routes.lock().await.is_empty());
}

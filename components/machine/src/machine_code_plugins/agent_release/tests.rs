use super::super::{
    CodeRuntimeSelection, CodeTarget, PrivateRuntimeDirectory, agent_holdings, tests::fixture_plan,
};
use super::*;
use std::fs;
use std::path::PathBuf;
use tokio::sync::Mutex as AsyncMutex;

mod fixture;
pub(crate) use fixture::Fixture;

const OWNER: &str = "cowboy-agent-0000000000000000000000000000000000000000000000000000000000000001-00000000000000000000000000000001";

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
    let runtime = Arc::clone(runtime);
    let mut held = agent_holdings::Holding::opening(root.to_str().unwrap(), true);
    held.confirm();
    let key = ("file".into(), OWNER.into());
    let mut route = WorktreeRoute {
        target: Some(target),
        generation,
        ..Default::default()
    };
    route.buffers.insert(key.clone());
    route.agent_buffers.insert(key, held);
    let close = json!({"type":"closeBuffer", "worktree":root, "path":"file", "leaseId":OWNER});
    (host, runtime, route, close, home)
}

fn log(home: &Path) -> Vec<Value> {
    fs::read_to_string(home.join("release-log"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn lost_execute_queries_original_reference_without_reselection_or_replay() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, close, home) = setup(&root.0, "recover-agent-foreign").await;
    route.buffers.insert(("file".into(), "interactive".into()));
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .is_err()
    );
    assert_eq!(route.agent_buffers.len(), 1);
    fs::write(home.join("foreign-query"), "yes").unwrap();
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .is_err()
    );
    assert_eq!(route.agent_buffers.len(), 1);
    fs::remove_file(home.join("foreign-query")).unwrap();
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .unwrap()
            .is_some()
    );
    assert!(route.agent_buffers.is_empty());
    assert_eq!(route.buffers.len(), 1);
    assert!(route.target.is_some());
    let commands = log(&home);
    assert_eq!(
        commands.iter().filter(|r| r["action"] == "execute").count(),
        1
    );
    let refs: Vec<_> = commands
        .iter()
        .filter(|r| r["type"] == "legacyRelease")
        .map(|r| &r["release"])
        .collect();
    assert!(refs.iter().all(|r| *r == refs[0]));
    assert_eq!(
        host.agent_retirements.permits.available_permits(),
        CAPACITY - 1
    );
    host.agent_retirements.observe().await;
    assert!(host.agent_retirements.pending.lock().is_empty());
    assert_eq!(host.agent_retirements.permits.available_permits(), CAPACITY);
}

#[tokio::test]
async fn lost_open_confirmation_uses_prepared_release_and_recovers_lost_close() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, close, home) = setup(&root.0, "recover-agent-lost-open").await;
    let held = route.agent_buffers.values_mut().next().unwrap();
    held.refresh(root.0.to_str().unwrap()).unwrap();
    held.confirm_worktree();
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .is_err()
    );
    assert_eq!(route.agent_buffers.len(), 1);
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .unwrap()
            .is_some()
    );
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
    assert!(
        requests
            .iter()
            .all(|r| r["type"] != "closeBuffer" && r["type"] != "openBuffer")
    );
}

#[tokio::test]
async fn unknown_worktree_refuses_before_release_or_legacy_close_and_stays_charged() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, close, home) =
        setup(&root.0, "recover-agent-unknown-tree").await;
    *route.agent_buffers.values_mut().next().unwrap() =
        agent_holdings::Holding::opening(root.0.to_str().unwrap(), true);
    for _ in 0..3 {
        assert!(
            host.release_agent(&mut route, &runtime, &close)
                .await
                .is_err()
        );
        assert!(route.dispatching(&close).is_err());
    }
    assert_eq!(route.agent_buffers.len(), 1);
    assert_eq!(route.buffers.len(), 1);
    assert!(!route.is_idle());
    assert!(!home.join("release-log").exists());
    assert_eq!(host.agent_retirements.permits.available_permits(), CAPACITY);
}

#[tokio::test]
async fn cancelled_retirement_preserves_reference_and_its_capacity_permit() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, close, home) =
        setup(&root.0, "recover-agent-pause-retire").await;
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .is_err()
    );
    host.release_agent(&mut route, &runtime, &close)
        .await
        .unwrap();
    let host = Arc::new(host);
    let task = {
        let host = Arc::clone(&host);
        tokio::spawn(async move {
            host.agent_retirements.observe().await;
        })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while !home.join("retire-started").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(host.agent_retirements.pending.lock().len(), 1);
    assert_eq!(
        host.agent_retirements.permits.available_permits(),
        CAPACITY - 1
    );
    fs::write(home.join("resume-retire"), "yes").unwrap();
    host.agent_retirements.observe().await;
    assert!(host.agent_retirements.pending.lock().is_empty());
    assert_eq!(host.agent_retirements.permits.available_permits(), CAPACITY);
    assert_eq!(
        log(&home)
            .iter()
            .filter(|r| r["action"] == "execute")
            .count(),
        1
    );
}

#[tokio::test]
async fn maintenance_rotates_four_original_queries_without_a_controller() {
    let root = PrivateRuntimeDirectory::create().unwrap();
    let (host, runtime, mut route, close, home) = setup(&root.0, "recover-agent-pending").await;
    assert!(
        host.release_agent(&mut route, &runtime, &close)
            .await
            .is_err()
    );
    for n in 1..8 {
        let path = format!("file{n}");
        let owner = format!("cowboy-agent-{:064x}-{:032x}", n / 4 + 1, n + 1);
        let mut held = agent_holdings::Holding::opening(root.0.to_str().unwrap(), true);
        held.confirm();
        route
            .agent_buffers
            .insert((path.clone(), owner.clone()), held);
        route.buffers.insert((path.clone(), owner.clone()));
        let close = json!({"type":"closeBuffer", "worktree":root.0, "path":path, "leaseId":owner});
        assert!(
            host.release_agent(&mut route, &runtime, &close)
                .await
                .is_err()
        );
    }
    let route = Arc::new(AsyncMutex::new(route));
    host.routes
        .lock()
        .await
        .insert(("fixture-code".into(), root.0.clone()), Arc::clone(&route));
    host.release_idle_agents().await;
    assert_eq!(
        log(&home).iter().filter(|r| r["action"] == "query").count(),
        4
    );
    host.release_idle_agents().await;
    let commands = log(&home);
    let queries: Vec<_> = commands
        .iter()
        .filter(|r| r["action"] == "query")
        .map(|r| r["release"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(queries.len(), 8);
    assert_eq!(
        queries
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        8
    );
    assert_eq!(
        commands.iter().filter(|r| r["action"] == "execute").count(),
        8
    );
    assert_eq!(route.lock().await.agent_buffers.len(), 8);
}

#[tokio::test]
async fn generic_code_rpc_cannot_invoke_internal_automatic_release() {
    let host = CodeRuntimeHost::default();
    for kind in [
        "legacyReleaseSupport",
        "prepareLegacyRelease",
        "legacyRelease",
    ] {
        assert!(
            host.request("zed", &json!({"type":kind}), || panic!(
                "must not select a runtime"
            ))
            .await
            .is_err()
        );
    }
    assert!(host.routes.lock().await.is_empty());
}

use super::*;
use crate::machine_code_plugins::{
    CodeRuntimeHost, CodeRuntimeSelection, PrivateRuntimeDirectory, tests::fixture_plan,
};
use crate::machine_plugins::PluginExecutionScope;
use crate::machine_protocol::code_buffer_navigation::{Content, Kind, Point, Request};
use serde_json::Value;
use std::path::PathBuf;

struct Fixture {
    host: CodeRuntimeHost,
    scope: PluginExecutionScope,
    service: String,
    root: PrivateRuntimeDirectory,
    home: PathBuf,
    lease: BufferRef,
}

impl Fixture {
    async fn new(mode: &str) -> Self {
        let host = CodeRuntimeHost::default();
        let root = PrivateRuntimeDirectory::create().unwrap();
        let mut plan = fixture_plan(&root.0, mode);
        plan.plugin_id = "zed".into();
        let home = plan.home.clone();
        let prepared = host
            .request(
                "zed",
                &json!({"type":"prepareBuffer","worktree":root.0,"path":"file"}),
                || Ok(CodeRuntimeSelection::Installed(plan)),
            )
            .await
            .unwrap();
        let lease: BufferRef = serde_json::from_value(prepared["lease"].clone()).unwrap();
        host.request(
            "zed",
            &json!({"type":"openBufferLease","lease":lease}),
            || panic!("reselected original runtime"),
        )
        .await
        .unwrap();
        let service = format!("svc-{}", "a".repeat(32));
        let scope = PluginExecutionScope::new(Some(&service), "hawk");
        Self {
            host,
            scope,
            service,
            root,
            home,
            lease,
        }
    }

    fn invocation(&self, action: Action) -> CodeNavigationInvocation {
        self.scope
            .code_navigation(Request {
                service_id: self.service.clone(),
                machine_id: "hawk".into(),
                action,
            })
            .unwrap()
    }

    fn preparation(&self) -> Action {
        Action::Prepare {
            lease: self.lease.clone(),
            content: content(),
            position: Point { row: 0, column: 1 },
            query: Kind::Definition,
        }
    }

    async fn act(&self, action: Action) -> Result<Snapshot> {
        self.host.navigate(self.invocation(action)).await
    }

    async fn prepare(&self) -> NavigationRef {
        self.act(self.preparation()).await.unwrap().navigation
    }

    async fn retained(&self) -> NavigationRef {
        let id = self.prepare().await;
        assert_eq!(
            self.act(Action::Execute {
                navigation: id.clone()
            })
            .await
            .unwrap()
            .phase,
            Phase::Retained
        );
        id
    }

    async fn buffer(&self, command: &str, lease: &BufferRef) -> Result<Value> {
        self.host
            .request("zed", &json!({"type":command,"lease":lease}), || {
                panic!("reselected original buffer runtime")
            })
            .await
    }

    fn count(&self, name: &str) -> u64 {
        std::fs::read_to_string(self.home.join(name))
            .unwrap()
            .parse()
            .unwrap()
    }

    async fn wait(&self, name: &str) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !self.home.join(name).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

fn content() -> Content {
    Content {
        sha256: "c".repeat(64),
        utf8_bytes: 4,
    }
}
fn destination(id: &NavigationRef) -> Action {
    Action::PrepareDestination {
        navigation: id.clone(),
        destination: 0,
        content: content(),
    }
}

#[tokio::test]
async fn waiting_for_the_original_route_does_not_renew_command_authority() {
    let f = Fixture::new("navigation-ok").await;
    let id = f.prepare().await;
    let entry = f.host.buffer_navigation.registry.lock().find(&id).unwrap();
    let route = Arc::clone(&entry.lock().await.target.as_ref().unwrap().route);
    let held = route.lock().await;
    tokio::time::pause();
    let mut queued = Box::pin(f.act(Action::Execute {
        navigation: id.clone(),
    }));
    assert!(
        tokio::time::timeout(Duration::from_millis(1), &mut queued)
            .await
            .is_err()
    );
    tokio::time::advance(Duration::from_secs(15)).await;
    drop(held);
    assert!(queued.await.is_err());
    tokio::time::resume();
    assert!(!f.home.join("nav-executes").exists());
    assert_eq!(entry.lock().await.phase, Phase::Prepared);
    f.act(Action::Release { navigation: id }).await.unwrap();
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
}

#[tokio::test]
async fn cancellation_during_destination_preparation_leaves_no_core_buffer_route() {
    let f = Fixture::new("navigation-pause-destination").await;
    let id = f.retained().await;
    let entry = f.host.buffer_navigation.registry.lock().find(&id).unwrap();
    let route = Arc::clone(&entry.lock().await.target.as_ref().unwrap().route);
    let mut preparing = Box::pin(f.act(destination(&id)));
    tokio::select! {
        result = &mut preparing => panic!("unexpected completion: {result:?}"),
        () = f.wait("nav-destinations") => {}
    }
    drop(preparing);
    assert!(entry.lock().await.destinations.is_empty());
    assert_eq!(route.lock().await.owned_buffers.len(), 1);
    std::fs::write(f.home.join("resume-destination"), "ready").unwrap();
    // Only an inert native reservation can have been created. No open was
    // attempted; its adapter TTL, not this cancellation, owns its retirement.
    let prepared = f.act(destination(&id)).await.unwrap();
    assert_eq!(f.count("nav-destinations"), 2);
    let lease = &prepared.destinations[0].lease;
    f.buffer("releaseBufferLease", lease).await.unwrap();
    f.act(Action::Release { navigation: id }).await.unwrap();
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
}

#[tokio::test]
async fn runtime_death_cannot_redirect_navigation_to_a_replacement_generation() {
    let f = Fixture::new("navigation-ok").await;
    let id = f.retained().await;
    let entry = f.host.buffer_navigation.registry.lock().find(&id).unwrap();
    let runtime = Arc::clone(&entry.lock().await.target.as_ref().unwrap().runtime);
    runtime.child.lock().start_kill().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.is_running().unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let open = json!({"type":"openWorktree","path":f.root.0,"trusted":true});
    assert!(
        f.host
            .request("zed", &open, || panic!("old route selected a replacement"))
            .await
            .is_err()
    );
    let mut replacement = fixture_plan(&f.root.0, "navigation-replacement");
    replacement.plugin_id = "zed".into();
    let home = replacement.home.clone();
    f.host
        .request("zed", &open, || {
            Ok(CodeRuntimeSelection::Installed(replacement))
        })
        .await
        .unwrap();
    assert!(f.act(destination(&id)).await.is_err());
    assert!(
        f.act(Action::Release {
            navigation: id.clone()
        })
        .await
        .is_err()
    );
    assert!(!home.join("nav-destinations").exists());
    assert!(!home.join("nav-releases").exists());
    assert_eq!(
        f.act(Action::Query { navigation: id }).await.unwrap().phase,
        Phase::Retained,
        "saved evidence is not discarded or reported released when the process dies"
    );
    f.host
        .request(
            "zed",
            &json!({"type":"closeWorktree","path":f.root.0}),
            || panic!("replacement close changed runtime"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn exact_navigation_support_is_required_before_any_preparation() {
    let f = Fixture::new("navigation-old").await;
    assert!(f.act(f.preparation()).await.is_err());
    assert!(!f.home.join("nav-prepares").exists());
    assert_eq!(
        f.host.buffer_navigation.capacity.available_permits(),
        MAX_NAVIGATIONS
    );
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
    assert_eq!(f.host.live_generation_count().await, 0);
}

#[tokio::test]
async fn independent_destination_joins_original_buffer_routes_and_survives_group_release() {
    let f = Fixture::new("navigation-ok").await;
    let id = f.retained().await;
    let saved = f
        .act(Action::Query {
            navigation: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(saved.locations.len(), 2);
    assert_eq!(
        f.act(Action::Execute {
            navigation: id.clone()
        })
        .await
        .unwrap(),
        saved
    );
    assert_eq!(f.count("nav-executes"), 1);
    assert!(!f.home.join("nav-queries").exists());
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
    assert_eq!(f.host.live_generation_count().await, 1);
    f.host
        .request(
            "zed",
            &json!({"type":"closeWorktree","path":f.root.0}),
            || panic!("navigation lost route pin"),
        )
        .await
        .unwrap();
    assert_eq!(f.host.live_generation_count().await, 1);
    let prepared = f.act(destination(&id)).await.unwrap();
    assert_eq!(prepared.destinations.len(), 1);
    assert_eq!(f.act(destination(&id)).await.unwrap(), prepared);
    assert_eq!(f.count("nav-destinations"), 1);
    let lease = &prepared.destinations[0].lease;
    f.buffer("openBufferLease", lease).await.unwrap();
    let released = f
        .act(Action::Release {
            navigation: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(released.phase, Phase::Released);
    assert_eq!(released.destinations, prepared.destinations);
    assert_eq!(
        f.act(Action::Release {
            navigation: id.clone()
        })
        .await
        .unwrap(),
        released
    );
    assert_eq!(
        f.act(Action::Execute { navigation: id }).await.unwrap(),
        released
    );
    assert_eq!(f.count("nav-releases"), 1);
    assert_eq!(f.host.live_generation_count().await, 1);
    f.host
        .request(
            "zed",
            &json!({"type":"readBufferLease","lease":lease,"request":{"kind":"symbols"}}),
            || panic!("destination adopted a replacement"),
        )
        .await
        .unwrap();
    f.buffer("releaseBufferLease", lease).await.unwrap();
    assert_eq!(f.host.live_generation_count().await, 0);
    assert_eq!(
        f.host.buffer_navigation.capacity.available_permits(),
        MAX_NAVIGATIONS
    );
}

#[tokio::test]
async fn lost_or_invalid_execution_observations_are_query_only_never_replayed() {
    for mode in [
        "navigation-lost",
        "navigation-bad-owner",
        "navigation-bad-path",
        "navigation-bad-range",
        "navigation-prepared",
        "navigation-released",
        "navigation-extra",
    ] {
        let f = Fixture::new(mode).await;
        let id = f.prepare().await;
        assert!(
            f.act(Action::Execute {
                navigation: id.clone()
            })
            .await
            .is_err()
        );
        for action in [
            Action::Execute {
                navigation: id.clone(),
            },
            Action::Release {
                navigation: id.clone(),
            },
        ] {
            let unknown = f.act(action).await.unwrap();
            assert_eq!(unknown.phase, Phase::Unknown);
            assert!(unknown.locations.is_empty());
        }
        assert!(f.act(destination(&id)).await.is_err());
        assert_eq!(f.count("nav-executes"), 1);
        assert!(!f.home.join("nav-releases").exists());
        f.buffer("releaseBufferLease", &f.lease).await.unwrap();
        assert_eq!(f.host.live_generation_count().await, 1);
        assert_eq!(
            f.act(Action::Query {
                navigation: id.clone()
            })
            .await
            .unwrap()
            .phase,
            Phase::Retained
        );
        f.act(Action::Release { navigation: id }).await.unwrap();
        assert_eq!(f.host.live_generation_count().await, 0);
    }
}

#[tokio::test]
async fn lost_release_keeps_target_evidence_and_never_resends_release() {
    let f = Fixture::new("navigation-lost-release").await;
    let id = f.retained().await;
    assert!(
        f.act(Action::Release {
            navigation: id.clone()
        })
        .await
        .is_err()
    );
    let unknown = f
        .act(Action::Release {
            navigation: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(unknown.phase, Phase::ReleaseUnknown);
    assert_eq!(unknown.locations.len(), 2);
    assert!(f.act(destination(&id)).await.is_err());
    assert_eq!(f.count("nav-releases"), 1);
    assert_eq!(
        f.act(Action::Query { navigation: id }).await.unwrap().phase,
        Phase::Released
    );
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
    assert_eq!(f.host.live_generation_count().await, 0);
}

#[tokio::test]
async fn cancellation_after_dispatch_retains_unknown_and_original_runtime() {
    let f = Fixture::new("navigation-pause").await;
    let id = f.prepare().await;
    let mut executing = Box::pin(f.act(Action::Execute {
        navigation: id.clone(),
    }));
    tokio::select! {
        result = &mut executing => panic!("unexpected completion: {result:?}"),
        () = f.wait("nav-executes") => {}
    }
    drop(executing);
    let saved = f
        .act(Action::Execute {
            navigation: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(saved.phase, Phase::Unknown);
    assert_eq!(
        f.host.buffer_navigation.capacity.available_permits(),
        MAX_NAVIGATIONS - 1
    );
    std::fs::write(f.home.join("resume-navigation"), "ready").unwrap();
    assert_eq!(
        f.act(Action::Query {
            navigation: id.clone()
        })
        .await
        .unwrap()
        .phase,
        Phase::Retained
    );
    assert_eq!(f.count("nav-executes"), 1);
    f.act(Action::Release { navigation: id }).await.unwrap();
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
}

#[tokio::test]
async fn a_lost_destination_observer_discovers_the_same_prepared_reference() {
    let f = Arc::new(Fixture::new("navigation-pause-destination").await);
    let id = f.retained().await;
    let caller = Arc::clone(&f);
    let action = destination(&id);
    let observer = tokio::spawn(async move { caller.act(action).await.unwrap() });
    f.wait("nav-destinations").await;
    drop(observer); // Detached admitted work, not cancellation of the operation.
    std::fs::write(f.home.join("resume-destination"), "ready").unwrap();
    let saved = f
        .act(Action::Query {
            navigation: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(saved.destinations.len(), 1);
    assert_eq!(f.act(destination(&id)).await.unwrap(), saved);
    assert_eq!(f.count("nav-destinations"), 1);
    let lease = &saved.destinations[0].lease;
    f.buffer("openBufferLease", lease).await.unwrap();
    f.act(Action::Release { navigation: id }).await.unwrap();
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
    f.buffer("releaseBufferLease", lease).await.unwrap();
}

#[tokio::test]
async fn source_release_or_sync_reservation_before_execute_cannot_acquire_targets() {
    let f = Fixture::new("navigation-ok").await;
    let id = f.prepare().await;
    use crate::machine_protocol::code_buffer_sync as sync;
    let sync = f
        .host
        .synchronize(
            f.scope
                .code_buffer_sync(sync::Request {
                    service_id: f.service.clone(),
                    machine_id: "hawk".into(),
                    action: sync::Action::Prepare {
                        lease: f.lease.clone(),
                        purpose: sync::Purpose::RefreshFromDisk,
                        content: content(),
                    },
                })
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        f.act(Action::Execute {
            navigation: id.clone()
        })
        .await
        .is_err()
    );
    f.host
        .synchronize(
            f.scope
                .code_buffer_sync(sync::Request {
                    service_id: f.service.clone(),
                    machine_id: "hawk".into(),
                    action: sync::Action::Retire {
                        operation: sync.operation,
                    },
                })
                .unwrap(),
        )
        .await
        .unwrap();
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
    assert!(
        f.act(Action::Execute {
            navigation: id.clone()
        })
        .await
        .is_err()
    );
    assert!(!f.home.join("nav-executes").exists());
    f.act(Action::Release { navigation: id }).await.unwrap();
    assert_eq!(f.host.live_generation_count().await, 0);
}

#[tokio::test]
async fn queued_connection_loss_cannot_dispatch_or_adopt_an_old_group() {
    let mut f = Fixture::new("navigation-ok").await;
    let id = f.prepare().await;
    let entry = f.host.buffer_navigation.registry.lock().find(&id).unwrap();
    let route = Arc::clone(&entry.lock().await.target.as_ref().unwrap().route);
    let held = route.lock().await;
    let invocation = f.invocation(Action::Execute {
        navigation: id.clone(),
    });
    let mut queued = Box::pin(f.host.navigate(invocation));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut queued)
            .await
            .is_err()
    );
    f.scope = PluginExecutionScope::new(Some(&f.service), "hawk");
    drop(held);
    assert!(queued.await.is_err());
    assert!(!f.home.join("nav-executes").exists());
    assert!(f.act(Action::Query { navigation: id }).await.is_err());
    assert_eq!(entry.lock().await.phase, Phase::Prepared);
}

#[tokio::test]
async fn queued_expiry_is_inert_and_does_not_retain_an_idle_runtime() {
    let f = Fixture::new("navigation-ok").await;
    let id = f.prepare().await;
    let entry = f.host.buffer_navigation.registry.lock().find(&id).unwrap();
    entry.lock().await.until = Some(Instant::now() + Duration::from_millis(20));
    let route = Arc::clone(&entry.lock().await.target.as_ref().unwrap().route);
    let held = route.lock().await;
    let mut queued = Box::pin(f.act(Action::Execute {
        navigation: id.clone(),
    }));
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut queued)
            .await
            .is_err()
    );
    drop(held);
    assert_eq!(queued.await.unwrap().phase, Phase::Released);
    assert!(!f.home.join("nav-executes").exists());
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
    assert_eq!(f.host.live_generation_count().await, 0);
}

#[tokio::test]
async fn bounded_capacity_only_expires_preparations_never_unknown_effects() {
    let f = Fixture::new("navigation-unknown").await;
    let id = f.prepare().await;
    assert_eq!(
        f.act(Action::Execute {
            navigation: id.clone()
        })
        .await
        .unwrap()
        .phase,
        Phase::Unknown
    );
    for _ in 1..MAX_NAVIGATIONS {
        f.prepare().await;
    }
    assert!(f.act(f.preparation()).await.is_err());
    assert_eq!(f.count("nav-prepares"), MAX_NAVIGATIONS as u64);
    let entries: Vec<_> = f
        .host
        .buffer_navigation
        .registry
        .lock()
        .active
        .values()
        .cloned()
        .collect();
    for entry in entries {
        entry.lock().await.until = Some(Instant::now() - Duration::from_secs(1));
    }
    f.host.buffer_navigation.expire_inert();
    assert_eq!(
        f.host.buffer_navigation.capacity.available_permits(),
        MAX_NAVIGATIONS - 1
    );
    assert_eq!(
        f.act(Action::Query {
            navigation: id.clone()
        })
        .await
        .unwrap()
        .phase,
        Phase::Unknown
    );
    let commands = f
        .host
        .buffer_navigation
        .commands
        .acquire_many(MAX_COMMANDS as u32)
        .await
        .unwrap();
    assert!(f.act(f.preparation()).await.is_err());
    drop(commands);
    assert_eq!(
        f.act(Action::Release { navigation: id })
            .await
            .unwrap()
            .phase,
        Phase::Unknown
    );
    assert!(!f.home.join("nav-releases").exists());
}

#[tokio::test]
async fn destination_indices_content_and_terminal_groups_never_allocate_replacements() {
    let f = Fixture::new("navigation-ok").await;
    let id = f.retained().await;
    for action in [
        Action::PrepareDestination {
            navigation: id.clone(),
            destination: 2,
            content: content(),
        },
        Action::PrepareDestination {
            navigation: id.clone(),
            destination: 0,
            content: Content {
                sha256: "d".repeat(64),
                ..content()
            },
        },
    ] {
        assert!(f.act(action).await.is_err());
    }
    assert!(!f.home.join("nav-destinations").exists());
    f.act(Action::Release {
        navigation: id.clone(),
    })
    .await
    .unwrap();
    assert_eq!(
        f.act(destination(&id)).await.unwrap().phase,
        Phase::Released
    );
    assert!(!f.home.join("nav-destinations").exists());
    f.buffer("releaseBufferLease", &f.lease).await.unwrap();
}

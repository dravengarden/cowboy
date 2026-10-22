use super::*;
use crate::machine_code_plugins::{
    CodeRuntimeHost, CodeRuntimeSelection, PrivateRuntimeDirectory, tests::fixture_plan,
};
use crate::machine_plugins::PluginExecutionScope;
use crate::machine_protocol::code_buffer_navigation as navigation;
use crate::machine_protocol::code_buffer_sync::{BufferRef, Purpose, Request};

struct Fixture {
    host: CodeRuntimeHost,
    scope: PluginExecutionScope,
    service: String,
    _root: PrivateRuntimeDirectory,
    home: std::path::PathBuf,
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
            || panic!("reselected a runtime"),
        )
        .await
        .unwrap();
        let service = format!("svc-{}", "a".repeat(32));
        let scope = PluginExecutionScope::new(Some(&service), "hawk");
        Self {
            host,
            scope,
            service,
            _root: root,
            home,
            lease,
        }
    }

    fn invocation(&self, action: Action) -> CodeBufferSyncInvocation {
        self.scope
            .code_buffer_sync(Request {
                service_id: self.service.clone(),
                machine_id: "hawk".into(),
                action,
            })
            .unwrap()
    }

    fn preparation(&self) -> Action {
        Action::Prepare {
            lease: self.lease.clone(),
            purpose: Purpose::RefreshFromDisk,
            content: Content {
                sha256: "c".repeat(64),
                utf8_bytes: 4,
            },
        }
    }

    async fn prepare(&self) -> Snapshot {
        self.host
            .synchronize(self.invocation(self.preparation()))
            .await
            .unwrap()
    }

    async fn act(&self, action: Action) -> Result<Snapshot> {
        self.host.synchronize(self.invocation(action)).await
    }

    fn navigation_preparation(&self) -> navigation::Action {
        navigation::Action::Prepare {
            lease: self.lease.clone(),
            content: Content {
                sha256: "c".repeat(64),
                utf8_bytes: 4,
            },
            position: navigation::Point { row: 0, column: 1 },
            query: navigation::Kind::Definition,
        }
    }

    async fn navigate(&self, action: navigation::Action) -> Result<navigation::Snapshot> {
        self.host
            .navigate(self.scope.code_navigation(navigation::Request {
                service_id: self.service.clone(),
                machine_id: "hawk".into(),
                action,
            })?)
            .await
    }

    async fn release(&self) -> Result<Value> {
        self.host
            .request(
                "zed",
                &json!({"type":"releaseBufferLease","lease":self.lease}),
                || panic!("reselected a runtime"),
            )
            .await
    }

    fn count(&self, name: &str) -> u64 {
        std::fs::read_to_string(self.home.join(name))
            .unwrap()
            .parse()
            .unwrap()
    }
}

#[tokio::test]
async fn exact_owner_support_is_required_before_preparation_or_effect() {
    let fixture = Fixture::new("sync-old").await;
    assert!(fixture.act(fixture.preparation()).await.is_err());
    assert!(!fixture.home.join("sync-prepares").exists());
    fixture.release().await.unwrap();
    assert_eq!(fixture.host.live_generation_count().await, 0);
    assert_eq!(
        fixture.host.buffer_sync.capacity.available_permits(),
        MAX_OPERATIONS
    );
}

#[tokio::test]
async fn apply_is_once_and_observation_or_duplicate_calls_never_select_a_runtime() {
    let fixture = Fixture::new("sync-ok").await;
    let id = fixture.prepare().await.operation;
    assert_ne!(id.instance, fixture.lease.instance);
    assert!(fixture.release().await.is_err());
    let applied = fixture
        .act(Action::Apply {
            operation: id.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(applied.state, State::Applied { .. }));
    for action in [
        Action::Apply {
            operation: id.clone(),
        },
        Action::Query {
            operation: id.clone(),
        },
    ] {
        assert_eq!(fixture.act(action).await.unwrap(), applied);
    }
    assert_eq!(fixture.count("sync-applies"), 1);
    fixture.release().await.unwrap();
    assert_eq!(
        fixture.host.live_generation_count().await,
        1,
        "sync still owns original process"
    );
    let retired = fixture
        .act(Action::Retire {
            operation: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(retired.state, State::Retired {});
    assert_eq!(fixture.count("sync-retires"), 1);
    assert_eq!(fixture.host.live_generation_count().await, 0);
    assert_eq!(
        fixture.act(Action::Apply { operation: id }).await.unwrap(),
        retired
    );
    assert_eq!(
        fixture.host.buffer_sync.capacity.available_permits(),
        MAX_OPERATIONS
    );
}

#[tokio::test]
async fn bad_or_missing_apply_replies_keep_the_original_fence_until_query() {
    for mode in [
        "sync-lost",
        "sync-bad-content",
        "sync-bad-owner",
        "sync-retired",
        "sync-pending",
    ] {
        let fixture = Fixture::new(mode).await;
        let id = fixture.prepare().await.operation;
        let applied = fixture
            .act(Action::Apply {
                operation: id.clone(),
            })
            .await;
        let expected = if mode == "sync-pending" {
            State::Pending {}
        } else {
            State::Unknown {}
        };
        if mode == "sync-pending" {
            assert_eq!(applied.unwrap().state, expected);
        } else {
            assert!(applied.is_err());
        }
        assert!(fixture.release().await.is_err());
        assert_eq!(
            fixture
                .act(Action::Apply {
                    operation: id.clone()
                })
                .await
                .unwrap()
                .state,
            expected
        );
        assert_eq!(
            fixture
                .act(Action::Retire {
                    operation: id.clone()
                })
                .await
                .unwrap()
                .state,
            expected
        );
        assert_eq!(fixture.count("sync-applies"), 1);
        assert!(!fixture.home.join("sync-retires").exists());
        assert!(matches!(
            fixture
                .act(Action::Query {
                    operation: id.clone()
                })
                .await
                .unwrap()
                .state,
            State::Applied { .. }
        ));
        fixture.act(Action::Retire { operation: id }).await.unwrap();
        fixture.release().await.unwrap();
    }
}

#[tokio::test]
async fn budget_refusal_requires_exact_evidence_and_separate_retirement_without_replay() {
    use crate::machine_protocol::code_buffer_sync::Reason;
    for mode in [
        "sync-budget",
        "sync-budget-lost",
        "sync-budget-owner",
        "sync-budget-partial",
    ] {
        let fixture = Fixture::new(mode).await;
        let id = fixture.prepare().await.operation;
        let refused = State::Refused {
            reason: Reason::Budget,
        };
        let applied = fixture
            .act(Action::Apply {
                operation: id.clone(),
            })
            .await;
        if mode == "sync-budget" {
            assert_eq!(applied.unwrap().state, refused);
        } else {
            assert!(applied.is_err());
            assert!(fixture.release().await.is_err());
            for action in [
                Action::Apply {
                    operation: id.clone(),
                },
                Action::Retire {
                    operation: id.clone(),
                },
            ] {
                assert_eq!(fixture.act(action).await.unwrap().state, State::Unknown {});
            }
            assert!(!fixture.home.join("sync-retires").exists());
            assert_eq!(
                fixture
                    .act(Action::Query {
                        operation: id.clone()
                    })
                    .await
                    .unwrap()
                    .state,
                refused
            );
        }
        for action in [
            Action::Apply {
                operation: id.clone(),
            },
            Action::Query {
                operation: id.clone(),
            },
        ] {
            assert_eq!(fixture.act(action).await.unwrap().state, refused);
        }
        assert_eq!(fixture.count("sync-applies"), 1);
        assert!(!fixture.home.join("sync-retires").exists());
        fixture.release().await.unwrap();
        assert_eq!(fixture.host.live_generation_count().await, 1);
        assert_eq!(
            fixture
                .act(Action::Retire {
                    operation: id.clone()
                })
                .await
                .unwrap()
                .state,
            State::Retired {}
        );
        assert_eq!(
            fixture
                .act(Action::Retire { operation: id })
                .await
                .unwrap()
                .state,
            State::Retired {}
        );
        assert_eq!(fixture.count("sync-retires"), 1);
        assert_eq!(fixture.host.live_generation_count().await, 0);
        assert_eq!(
            fixture.host.buffer_sync.capacity.available_permits(),
            MAX_OPERATIONS
        );
    }
}

#[tokio::test]
async fn cancelled_apply_keeps_a_nonexpiring_owner_and_is_observed_not_replayed() {
    let fixture = Fixture::new("sync-pause").await;
    let id = fixture.prepare().await.operation;
    let mut applying = Box::pin(fixture.act(Action::Apply {
        operation: id.clone(),
    }));
    tokio::select! {
        _ = &mut applying => panic!("fixture did not hold Apply"),
        () = async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !fixture.home.join("sync-applies").exists() { tokio::time::sleep(Duration::from_millis(10)).await; }
            }).await.unwrap();
        } => {}
    }
    drop(applying);
    let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
    assert!(entry.lock().await.until.is_none());
    assert!(fixture.release().await.is_err());
    assert_eq!(
        fixture
            .act(Action::Apply {
                operation: id.clone()
            })
            .await
            .unwrap()
            .state,
        State::Unknown {}
    );
    std::fs::write(fixture.home.join("resume-sync"), "ready").unwrap();
    assert!(matches!(
        fixture
            .act(Action::Query {
                operation: id.clone()
            })
            .await
            .unwrap()
            .state,
        State::Applied { .. }
    ));
    assert_eq!(fixture.count("sync-applies"), 1);
    fixture.act(Action::Retire { operation: id }).await.unwrap();
    fixture.release().await.unwrap();
}

#[tokio::test]
async fn retirement_receipt_loss_queries_the_same_operation_without_resending() {
    let fixture = Fixture::new("sync-lost-retire").await;
    let id = fixture.prepare().await.operation;
    fixture
        .act(Action::Apply {
            operation: id.clone(),
        })
        .await
        .unwrap();
    assert!(
        fixture
            .act(Action::Retire {
                operation: id.clone()
            })
            .await
            .is_err()
    );
    fixture
        .act(Action::Retire {
            operation: id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(fixture.count("sync-retires"), 1);
    assert_eq!(
        fixture
            .act(Action::Query { operation: id })
            .await
            .unwrap()
            .state,
        State::Retired {}
    );
    fixture.release().await.unwrap();
}

#[tokio::test]
async fn a_new_connection_never_adopts_an_old_operation_even_on_the_same_site() {
    let fixture = Fixture::new("sync-ok").await;
    let id = fixture.prepare().await.operation;
    let next = PluginExecutionScope::new(Some(&fixture.service), "hawk");
    for action in [
        Action::Apply {
            operation: id.clone(),
        },
        Action::Query {
            operation: id.clone(),
        },
        Action::Retire {
            operation: id.clone(),
        },
    ] {
        let invocation = next
            .code_buffer_sync(Request {
                service_id: fixture.service.clone(),
                machine_id: "hawk".into(),
                action,
            })
            .unwrap();
        assert!(fixture.host.synchronize(invocation).await.is_err());
    }
    assert!(!fixture.home.join("sync-applies").exists());
    fixture.act(Action::Retire { operation: id }).await.unwrap();
    fixture.release().await.unwrap();
}

#[tokio::test]
async fn direct_navigation_expires_inert_sync_without_an_unrelated_buffer_request() {
    for prepared_first in [false, true] {
        let fixture = Fixture::new("sync-ok").await;
        let action = if prepared_first {
            navigation::Action::Execute {
                navigation: fixture
                    .navigate(fixture.navigation_preparation())
                    .await
                    .unwrap()
                    .navigation,
            }
        } else {
            fixture.navigation_preparation()
        };
        let id = fixture.prepare().await.operation;
        let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
        entry.lock().await.until = Some(Instant::now());
        // No generic buffer or synchronization call between expiry and this
        // admission: both fresh preparation and existing Execute must work.
        let observed = fixture.navigate(action).await.unwrap();
        assert_eq!(
            observed.phase,
            if prepared_first {
                navigation::Phase::Retained
            } else {
                navigation::Phase::Prepared
            }
        );
        assert_eq!(entry.lock().await.state, State::Retired {});
        assert!(!fixture.home.join("sync-applies").exists());
        assert!(!fixture.home.join("sync-retires").exists());
        fixture
            .navigate(navigation::Action::Release {
                navigation: observed.navigation,
            })
            .await
            .unwrap();
        fixture.release().await.unwrap();
        assert_eq!(fixture.host.live_generation_count().await, 0);
    }
}

#[tokio::test]
async fn direct_navigation_cannot_expire_an_uncertain_synchronization_effect() {
    let fixture = Fixture::new("sync-lost").await;
    let id = fixture.prepare().await.operation;
    assert!(
        fixture
            .act(Action::Apply {
                operation: id.clone(),
            })
            .await
            .is_err()
    );
    let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
    entry.lock().await.until = Some(Instant::now());
    assert!(
        fixture
            .navigate(fixture.navigation_preparation())
            .await
            .is_err()
    );
    assert_eq!(entry.lock().await.state, State::Unknown {});
    assert!(!fixture.home.join("nav-prepares").exists());
    assert_eq!(fixture.count("sync-applies"), 1);
}

#[tokio::test]
async fn ordinary_buffer_requests_expire_only_inert_synchronization_fences() {
    let fixture = Fixture::new("sync-ok").await;
    let id = fixture.prepare().await.operation;
    let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
    entry.lock().await.until = Some(Instant::now());
    fixture.release().await.unwrap();
    assert!(!fixture.home.join("sync-applies").exists());
    assert_eq!(fixture.host.live_generation_count().await, 0);
    assert_eq!(
        fixture
            .act(Action::Query { operation: id })
            .await
            .unwrap()
            .state,
        State::Retired {}
    );

    let fixture = Fixture::new("sync-lost").await;
    let id = fixture.prepare().await.operation;
    assert!(
        fixture
            .act(Action::Apply {
                operation: id.clone()
            })
            .await
            .is_err()
    );
    // Even an erroneously elapsed clock field cannot expire a possible effect.
    let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
    entry.lock().await.until = Some(Instant::now());
    assert!(fixture.release().await.is_err());
    assert_eq!(fixture.count("sync-applies"), 1);
}

#[tokio::test]
async fn only_inert_preparations_expire_and_original_ids_never_reopen_paths() {
    let fixture = Fixture::new("sync-ok").await;
    let id = fixture.prepare().await.operation;
    let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
    entry.lock().await.until = Some(Instant::now());
    assert_eq!(
        fixture
            .act(Action::Apply {
                operation: id.clone()
            })
            .await
            .unwrap()
            .state,
        State::Retired {}
    );
    assert!(!fixture.home.join("sync-applies").exists());
    let next = fixture.prepare().await.operation;
    assert_ne!(next, id);
    let entry = fixture.host.buffer_sync.registry.lock().active[&next].clone();
    let socket = entry
        .lock()
        .await
        .target
        .as_ref()
        .unwrap()
        .runtime
        .socket
        .clone();
    let moved = socket.with_extension("held");
    std::fs::rename(&socket, &moved).unwrap();
    // Transport uses its original socket. It must not select another process
    // when that path is unavailable; moving it back permits original-ID query.
    assert!(
        fixture
            .act(Action::Apply {
                operation: next.clone()
            })
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .act(Action::Apply {
                operation: next.clone()
            })
            .await
            .unwrap()
            .state,
        State::Unknown {}
    );
    std::fs::rename(&moved, &socket).unwrap();
    // The fixture never saw Apply. Prepared is not proof of no effect; the
    // coordinator must retain Unknown and refuse to reset its one-use budget.
    assert!(
        fixture
            .act(Action::Query { operation: next })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn capacity_does_not_evict_prepared_unknown_or_terminal_owners() {
    for mode in ["sync-ok", "sync-lost"] {
        let fixture = Fixture::new(mode).await;
        let id = fixture.prepare().await.operation;
        let capacity = Arc::clone(&fixture.host.buffer_sync.capacity)
            .try_acquire_many_owned(u32::try_from(MAX_OPERATIONS - 1).unwrap())
            .unwrap();
        assert!(fixture.act(fixture.preparation()).await.is_err());
        let _ = fixture
            .act(Action::Apply {
                operation: id.clone(),
            })
            .await;
        assert!(fixture.act(fixture.preparation()).await.is_err());
        assert_eq!(fixture.count("sync-prepares"), 1);
        assert!(
            fixture
                .host
                .buffer_sync
                .registry
                .lock()
                .active
                .contains_key(&id)
        );
        drop(capacity);
        if mode == "sync-lost" {
            fixture
                .act(Action::Query {
                    operation: id.clone(),
                })
                .await
                .unwrap();
        }
        fixture.act(Action::Retire { operation: id }).await.unwrap();
        fixture.release().await.unwrap();
        assert_eq!(
            fixture.host.buffer_sync.capacity.available_permits(),
            MAX_OPERATIONS
        );
    }
}

#[tokio::test]
async fn queued_apply_rechecks_connection_authority_after_acquiring_the_original_route() {
    let mut fixture = Fixture::new("sync-ok").await;
    let id = fixture.prepare().await.operation;
    let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
    let route = Arc::clone(&entry.lock().await.target.as_ref().unwrap().route);
    let held = route.lock().await;
    let invocation = fixture.invocation(Action::Apply {
        operation: id.clone(),
    });
    let mut applying = Box::pin(fixture.host.synchronize(invocation));
    assert!(futures::poll!(&mut applying).is_pending());
    let next = PluginExecutionScope::new(Some(&fixture.service), "hawk");
    drop(std::mem::replace(&mut fixture.scope, next));
    drop(held);
    assert!(applying.await.is_err());
    assert!(!fixture.home.join("sync-applies").exists());
    assert!(!entry.lock().await.attempted);
}

#[tokio::test]
async fn a_dead_original_process_is_not_replaced_even_when_a_new_generation_exists() {
    let fixture = Fixture::new("sync-lost").await;
    let id = fixture.prepare().await.operation;
    assert!(
        fixture
            .act(Action::Apply {
                operation: id.clone()
            })
            .await
            .is_err()
    );
    let entry = fixture.host.buffer_sync.registry.lock().active[&id].clone();
    let runtime = Arc::clone(&entry.lock().await.target.as_ref().unwrap().runtime);
    runtime.child.lock().start_kill().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while runtime.is_running().unwrap() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let other = PrivateRuntimeDirectory::create().unwrap();
    let mut plan = fixture_plan(&other.0, "sync-replacement");
    plan.plugin_id = "zed".into();
    let home = plan.home.clone();
    fixture
        .host
        .request(
            "zed",
            &json!({"type":"openWorktree","path":other.0}),
            || Ok(CodeRuntimeSelection::Installed(plan)),
        )
        .await
        .unwrap();
    assert!(
        fixture
            .act(Action::Query {
                operation: id.clone()
            })
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .act(Action::Apply { operation: id })
            .await
            .unwrap()
            .state,
        State::Unknown {}
    );
    assert!(!home.join("sync-prepares").exists());
    assert!(!home.join("sync-applies").exists());
}

#[tokio::test]
async fn command_admission_is_bounded_without_losing_the_retained_operation() {
    let fixture = Fixture::new("sync-ok").await;
    let id = fixture.prepare().await.operation;
    let capacity = fixture
        .host
        .buffer_sync
        .commands
        .try_acquire_many(u32::try_from(MAX_COMMANDS).unwrap())
        .unwrap();
    assert!(
        fixture
            .act(Action::Apply {
                operation: id.clone()
            })
            .await
            .is_err()
    );
    assert!(!fixture.home.join("sync-applies").exists());
    drop(capacity);
    fixture
        .act(Action::Apply {
            operation: id.clone(),
        })
        .await
        .unwrap();
    fixture.act(Action::Retire { operation: id }).await.unwrap();
    fixture.release().await.unwrap();
}

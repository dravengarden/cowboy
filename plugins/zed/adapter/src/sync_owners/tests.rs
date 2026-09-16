use super::*;
use crate::{Request, Worktrees, respond, sync_native::wire};
use prost::Message as _;
use std::sync::Arc;
use tokio::sync::mpsc;

struct Fixture {
    root: PathBuf,
    worktrees: Worktrees,
    buffers: Buffers,
    zed: Zed,
    receiver: mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
    lease: buffer_leases::LeaseRef,
}

fn content() -> Content {
    crate::coordinates::Mirror::new(1, "new\n")
        .unwrap()
        .content()
        .clone()
}

impl Fixture {
    async fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "cw-sync-owner-{:032x}",
            u128::from_be_bytes(random)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), "old\n").unwrap();
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        crate::ensure_worktree(root.clone(), true, false, &worktrees, None)
            .await
            .unwrap();
        worktrees.write().await.get_mut(&root).unwrap().state = crate::WorktreeState::Ready;
        let Response::BufferLease { lease, .. } = respond(
            Request::PrepareBuffer {
                worktree: root.clone(),
                path: "file".into(),
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap() else {
            panic!("wrong reply")
        };
        respond(
            Request::OpenBufferLease {
                lease: lease.clone(),
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        let (outbound, _) = mpsc::unbounded_channel();
        let mut child = crate::Command::new("true").spawn().unwrap();
        child.wait().await.unwrap();
        let (sync, receiver) = crate::sync_native::Transport::new();
        let zed = Arc::new(crate::ZedRuntime {
            _child: crate::Mutex::new(child),
            outbound,
            pending: Arc::default(),
            sync,
            events: crate::broadcast::channel(16).0,
            buffer_files: Arc::default(),
            worktree_paths: Arc::default(),
            diagnostics: Arc::default(),
            next_message_id: crate::AtomicU32::new(1),
            next_lsp_request_id: crate::AtomicU64::new(1),
        });
        for variant in [
            proto::create_buffer_for_peer::Variant::State(proto::BufferState {
                id: 1,
                base_text: "old\n".into(),
                ..Default::default()
            }),
            proto::create_buffer_for_peer::Variant::Chunk(proto::BufferChunk {
                buffer_id: 1,
                is_last: true,
                ..Default::default()
            }),
        ] {
            zed.diagnostics.lock().unwrap().observe(
                &proto::envelope::Payload::CreateBufferForPeer(proto::CreateBufferForPeer {
                    variant: Some(variant),
                    ..Default::default()
                }),
            );
        }
        Self {
            root,
            worktrees,
            buffers,
            zed,
            receiver,
            lease,
        }
    }

    fn spawn(&self, request: Request) -> tokio::task::JoinHandle<Result<Response>> {
        let (worktrees, buffers, zed) = (
            self.worktrees.clone(),
            self.buffers.clone(),
            self.zed.clone(),
        );
        tokio::spawn(async move { respond(request, &worktrees, &buffers, Some(&zed)).await })
    }

    fn preparing(&self) -> tokio::task::JoinHandle<Result<Response>> {
        self.spawn(Request::PrepareBufferSync {
            lease: self.lease.clone(),
            purpose: Purpose::RefreshFromDisk,
            content: content(),
        })
    }

    async fn message(&mut self, action: NativeAction) -> (u32, CowboyBufferSync) {
        let frame = tokio::time::timeout(Duration::from_secs(2), self.receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let Some(wire::cowboy_buffer_sync_envelope::Payload::Request(request)) = frame.payload
        else {
            panic!("not a request")
        };
        assert_eq!(request.action, action as i32);
        (frame.id, request)
    }

    fn reply(&self, id: u32, phase: Phase) {
        let mut response = CowboyBufferSyncResponse {
            protocol: 1,
            instance: vec![3; 16],
            operation_id: if phase == Phase::Supported { 0 } else { 42 },
            phase: phase as i32,
            ..Default::default()
        };
        if phase == Phase::Applied {
            response.content_sha256 = digest_bytes(&content().sha256);
            response.content_bytes = content().utf8_bytes;
            response.version = vec![CowboyBufferSyncVersion {
                replica_id: 0,
                timestamp: 2,
            }];
        }
        if phase == Phase::Refused {
            response.refusal = Refusal::Shared as i32;
        }
        self.send(id, response);
    }

    fn send(&self, id: u32, response: CowboyBufferSyncResponse) {
        let encoded = wire::CowboyBufferSyncEnvelope {
            id: 99,
            responding_to: Some(id),
            payload: Some(wire::cowboy_buffer_sync_envelope::Payload::Response(
                response,
            )),
        }
        .encode_to_vec();
        assert!(self.zed.sync.response(id, &encoded));
    }

    async fn prepare(&mut self) -> OperationRef {
        let task = self.preparing();
        let (id, _) = self.message(NativeAction::Probe).await;
        self.reply(id, Phase::Supported);
        let (id, native) = self.message(NativeAction::Prepare).await;
        assert_eq!(native.buffer_id, 1);
        assert_eq!(
            native.version,
            vec![CowboyBufferSyncVersion {
                replica_id: 0,
                timestamp: 1
            }]
        );
        self.reply(id, Phase::Prepared);
        let Response::BufferSync {
            operation,
            state: State::Prepared,
            ..
        } = task.await.unwrap().unwrap()
        else {
            panic!("not prepared")
        };
        operation
    }

    async fn local(&self, request: Request) -> Result<Response> {
        // None guarantees these refusals are adapter-local, before native I/O.
        respond(request, &self.worktrees, &self.buffers, None).await
    }

    async fn act(&self, operation: &OperationRef, action: Action) -> Result<Response> {
        respond(
            Request::BufferSync {
                operation: operation.clone(),
                action,
            },
            &self.worktrees,
            &self.buffers,
            Some(&self.zed),
        )
        .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn state(response: &Response) -> &State {
    let Response::BufferSync { state, .. } = response else {
        panic!("not sync")
    };
    state
}

#[tokio::test]
async fn shared_owners_and_same_native_id_aliases_refuse_before_native_dispatch() {
    let fixture = Fixture::new().await;
    let key = (fixture.root.clone(), PathBuf::from("file"));
    for owner in [
        BufferOwner::Legacy("reader".into()),
        BufferOwner::Owned(999),
    ] {
        fixture
            .buffers
            .active
            .write()
            .await
            .get_mut(&key)
            .unwrap()
            .lease_ids
            .insert(owner);
        assert!(
            fixture
                .preparing()
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("another owner")
        );
        fixture
            .buffers
            .active
            .write()
            .await
            .get_mut(&key)
            .unwrap()
            .lease_ids
            .retain(|owner| *owner == BufferOwner::Owned(1));
    }
    fixture.buffers.active.write().await.insert(
        (fixture.root.clone(), "alias".into()),
        BufferLease {
            lease_ids: std::collections::HashSet::from([BufferOwner::Owned(999)]),
            remote_id: 1,
            version: Vec::new(),
            sync: None,
        },
    );
    assert!(fixture.preparing().await.unwrap().is_err());
    assert_eq!(
        fixture.zed.next_message_id.load(crate::Ordering::Relaxed),
        1
    );
}

#[tokio::test]
async fn reservation_fences_legacy_and_owned_reads_closes_opens_and_navigation() {
    let mut fixture = Fixture::new().await;
    let operation = fixture.prepare().await;
    let root = fixture.root.clone();
    let file = PathBuf::from("file");
    for request in [
        Request::BufferLanguage {
            worktree: root.clone(),
            path: file.clone(),
        },
        Request::BufferHover {
            worktree: root.clone(),
            path: file.clone(),
            row: 0,
            column: 0,
        },
        Request::BufferSymbols {
            worktree: root.clone(),
            path: file.clone(),
        },
        Request::BufferNavigate {
            worktree: root.clone(),
            path: file.clone(),
            row: 0,
            column: 0,
            kind: crate::NavigationKind::Definition,
        },
        Request::ReadBufferLease {
            lease: fixture.lease.clone(),
            request: buffer_leases::ReadRequest::Symbols {},
        },
        Request::ReleaseBufferLease {
            lease: fixture.lease.clone(),
        },
        Request::OpenBuffer {
            worktree: root.clone(),
            path: file.clone(),
            lease_id: "new".into(),
        },
        Request::CloseBuffer {
            worktree: root.clone(),
            path: file.clone(),
            lease_id: "stranger".into(),
        },
    ] {
        assert!(fixture.local(request).await.is_err());
    }
    let Response::BufferLease { lease, .. } = fixture
        .local(Request::PrepareBuffer {
            worktree: root,
            path: file,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(
        fixture
            .local(Request::OpenBufferLease {
                lease: lease.clone()
            })
            .await
            .is_err()
    );
    assert!(matches!(
        fixture
            .local(Request::QueryBufferLease {
                lease: lease.clone()
            })
            .await
            .unwrap(),
        Response::BufferLease {
            state: buffer_leases::LeaseState::Prepared,
            ..
        }
    ));
    assert!(matches!(
        state(&fixture.act(&operation, Action::Retire).await.unwrap()),
        State::Retired
    ));
    fixture
        .local(Request::OpenBufferLease { lease })
        .await
        .unwrap();
    assert!(
        fixture.receiver.try_recv().is_err(),
        "local operations dispatched native work"
    );
}

#[tokio::test]
async fn cancelled_apply_keeps_fence_and_budget_until_original_query_observes_terminal() {
    let mut fixture = Fixture::new().await;
    let operation = fixture.prepare().await;
    let apply = fixture.spawn(Request::BufferSync {
        operation: operation.clone(),
        action: Action::Apply,
    });
    let (_, sent) = fixture.message(NativeAction::Apply).await;
    assert_eq!(sent.operation_id, 42);
    apply.abort();
    assert!(apply.await.unwrap_err().is_cancelled());
    // Even a preparation deadline far in the past cannot release uncertainty.
    fixture
        .buffers
        .syncs
        .lock()
        .await
        .slots
        .get_mut(&1)
        .unwrap()
        .until = Instant::now().checked_sub(Duration::from_mins(1)).unwrap();
    for action in [Action::Apply, Action::Retire] {
        assert!(matches!(
            state(&fixture.act(&operation, action).await.unwrap()),
            State::Unknown
        ));
    }
    assert!(
        fixture
            .local(Request::ReleaseBufferLease {
                lease: fixture.lease.clone()
            })
            .await
            .is_err()
    );
    assert!(fixture.receiver.try_recv().is_err());
    for phase in [
        Phase::Prepared,
        Phase::Retired,
        Phase::Pending,
        Phase::Applied,
    ] {
        let query = fixture.spawn(Request::BufferSync {
            operation: operation.clone(),
            action: Action::Query,
        });
        let (id, sent) = fixture.message(NativeAction::Query).await;
        assert_eq!(sent.operation_id, 42);
        assert_eq!(sent.instance, vec![3; 16]);
        fixture.reply(id, phase);
        query.await.unwrap().unwrap();
        if phase != Phase::Applied {
            assert!(
                fixture
                    .local(Request::ReleaseBufferLease {
                        lease: fixture.lease.clone()
                    })
                    .await
                    .is_err()
            );
        }
    }
    assert!(matches!(
        state(&fixture.act(&operation, Action::Apply).await.unwrap()),
        State::Applied { .. }
    ));
    std::fs::remove_file(fixture.root.join("file")).unwrap();
    fixture
        .local(Request::ReleaseBufferLease {
            lease: fixture.lease.clone(),
        })
        .await
        .unwrap();
    let retire = fixture.spawn(Request::BufferSync {
        operation: operation.clone(),
        action: Action::Retire,
    });
    let (id, sent) = fixture.message(NativeAction::Retire).await;
    assert_eq!(sent.operation_id, 42);
    fixture.reply(id, Phase::Retired);
    retire.await.unwrap().unwrap();
    assert!(matches!(
        state(&fixture.act(&operation, Action::Apply).await.unwrap()),
        State::Retired
    ));
    assert!(fixture.receiver.try_recv().is_err());
}

#[tokio::test]
async fn applied_receipt_must_match_original_content_and_native_instance() {
    let mut fixture = Fixture::new().await;
    let operation = fixture.prepare().await;
    let apply = fixture.spawn(Request::BufferSync {
        operation: operation.clone(),
        action: Action::Apply,
    });
    let (id, _) = fixture.message(NativeAction::Apply).await;
    fixture.send(
        id,
        CowboyBufferSyncResponse {
            protocol: 1,
            instance: vec![3; 16],
            operation_id: 42,
            phase: Phase::Applied as i32,
            content_sha256: vec![0; 32],
            content_bytes: 4,
            ..Default::default()
        },
    );
    assert!(apply.await.unwrap().is_err());
    let query = fixture.spawn(Request::BufferSync {
        operation: operation.clone(),
        action: Action::Query,
    });
    let (id, _) = fixture.message(NativeAction::Query).await;
    fixture.send(
        id,
        CowboyBufferSyncResponse {
            protocol: 1,
            instance: vec![4; 16],
            operation_id: 42,
            phase: Phase::Refused as i32,
            refusal: Refusal::Shared as i32,
            ..Default::default()
        },
    );
    assert!(query.await.unwrap().is_err());
    assert!(matches!(
        state(&fixture.act(&operation, Action::Apply).await.unwrap()),
        State::Unknown
    ));
    assert!(
        fixture
            .local(Request::ReleaseBufferLease {
                lease: fixture.lease.clone()
            })
            .await
            .is_err()
    );
    let query = fixture.spawn(Request::BufferSync {
        operation,
        action: Action::Query,
    });
    let (id, _) = fixture.message(NativeAction::Query).await;
    fixture.reply(id, Phase::Refused);
    assert!(matches!(
        state(&query.await.unwrap().unwrap()),
        State::Refused {
            reason: Reason::Shared
        }
    ));
    fixture
        .local(Request::ReleaseBufferLease {
            lease: fixture.lease.clone(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn expiry_and_retirement_never_recycle_ids_or_release_replacement_fences() {
    let mut fixture = Fixture::new().await;
    let original = fixture.prepare().await;
    let old = Instant::now().checked_sub(Duration::from_mins(1)).unwrap();
    fixture
        .buffers
        .syncs
        .lock()
        .await
        .slots
        .get_mut(&1)
        .unwrap()
        .until = old;
    fixture
        .buffers
        .active
        .write()
        .await
        .values_mut()
        .next()
        .unwrap()
        .sync
        .as_mut()
        .unwrap()
        .until = Some(old);
    assert!(matches!(
        state(&fixture.act(&original, Action::Apply).await.unwrap()),
        State::Retired
    ));
    let replacement = fixture.prepare().await;
    assert_ne!(original, replacement);
    fixture.act(&original, Action::Retire).await.unwrap();
    assert!(
        fixture
            .local(Request::ReleaseBufferLease {
                lease: fixture.lease.clone()
            })
            .await
            .is_err()
    );
    fixture.act(&replacement, Action::Retire).await.unwrap();
    fixture
        .local(Request::ReleaseBufferLease {
            lease: fixture.lease.clone(),
        })
        .await
        .unwrap();
    let mut registry = fixture.buffers.syncs.lock().await;
    registry.last_id = u64::MAX;
    assert!(registry.allocate().is_err());
    assert!(
        registry
            .resolve(&OperationRef {
                instance: "f".repeat(32),
                id: original.id
            })
            .is_err()
    );
    assert!(fixture.receiver.try_recv().is_err());
}

#[test]
fn private_wire_requires_separate_purpose_and_cannot_supply_native_ticket_or_path() {
    let prepare = serde_json::json!({"type":"prepareBufferSync", "lease":{"instance":"a".repeat(32),"id":"0000000000000001"},
        "purpose":"refresh_from_disk", "content":content()});
    assert!(serde_json::from_value::<Request>(prepare.clone()).is_ok());
    for field in ["path", "worktree", "version", "nativeId"] {
        let mut bad = prepare.clone();
        bad[field] = serde_json::json!("replacement");
        assert!(serde_json::from_value::<Request>(bad).is_err());
    }
    for purpose in [
        serde_json::Value::Null,
        serde_json::json!("read"),
        serde_json::json!("force"),
    ] {
        let mut bad = prepare.clone();
        bad["purpose"] = purpose;
        assert!(serde_json::from_value::<Request>(bad).is_err());
    }
    let mut bad = prepare;
    bad.as_object_mut().unwrap().remove("purpose");
    assert!(serde_json::from_value::<Request>(bad).is_err());
    assert!(
        serde_json::from_value::<Request>(
            serde_json::json!({"type":"bufferSync", "operation":{}, "action":"reload"})
        )
        .is_err()
    );
}

#[tokio::test]
async fn disconnected_socket_observer_does_not_cancel_apply_or_renew_its_budget() {
    use tokio::io::AsyncWriteExt as _;
    let mut fixture = Fixture::new().await;
    let operation = fixture.prepare().await;
    let (mut observer, peer) = tokio::net::UnixStream::pair().unwrap();
    let task = tokio::spawn(crate::handle(
        peer,
        fixture.worktrees.clone(),
        fixture.buffers.clone(),
        Some(fixture.zed.clone()),
    ));
    let mut encoded = serde_json::to_vec(&Request::BufferSync {
        operation: operation.clone(),
        action: Action::Apply,
    })
    .unwrap();
    encoded.push(b'\n');
    observer.write_all(&encoded).await.unwrap();
    let (id, _) = fixture.message(NativeAction::Apply).await;
    drop(observer);
    fixture.reply(id, Phase::Pending);
    assert!(task.await.unwrap().is_err());
    for action in [Action::Apply, Action::Retire] {
        assert!(matches!(
            state(&fixture.act(&operation, action).await.unwrap()),
            State::Pending
        ));
    }
    assert!(fixture.receiver.try_recv().is_err());
    assert!(
        fixture
            .local(Request::ReleaseBufferLease {
                lease: fixture.lease.clone()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn interrupted_prepare_expires_without_apply_and_cannot_be_adopted() {
    let mut fixture = Fixture::new().await;
    let task = fixture.preparing();
    let (id, _) = fixture.message(NativeAction::Probe).await;
    fixture.reply(id, Phase::Supported);
    fixture.message(NativeAction::Prepare).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let mut registry = fixture.buffers.syncs.lock().await;
    let operation = OperationRef {
        instance: registry.instance.clone().unwrap(),
        id: "0000000000000001".into(),
    };
    assert!(registry.slots[&1].ticket.is_none());
    assert!(
        registry
            .act(
                operation.clone(),
                Action::Apply,
                &fixture.buffers,
                Some(&fixture.zed)
            )
            .await
            .is_err()
    );
    let old = Instant::now().checked_sub(Duration::from_mins(1)).unwrap();
    registry.slots.get_mut(&1).unwrap().until = old;
    fixture
        .buffers
        .active
        .write()
        .await
        .values_mut()
        .next()
        .unwrap()
        .sync
        .as_mut()
        .unwrap()
        .until = Some(old);
    assert!(matches!(
        state(
            &registry
                .act(
                    operation,
                    Action::Apply,
                    &fixture.buffers,
                    Some(&fixture.zed)
                )
                .await
                .unwrap()
        ),
        State::Retired
    ));
    drop(registry);
    fixture
        .local(Request::ReleaseBufferLease {
            lease: fixture.lease.clone(),
        })
        .await
        .unwrap();
    assert!(fixture.receiver.try_recv().is_err());
}

#[tokio::test]
async fn an_existing_unrelated_buffer_remains_readable_and_releasable() {
    let mut fixture = Fixture::new().await;
    std::fs::write(fixture.root.join("independent"), "unrelated").unwrap();
    fixture
        .local(Request::OpenBuffer {
            worktree: fixture.root.clone(),
            path: "independent".into(),
            lease_id: "independent".into(),
        })
        .await
        .unwrap();
    let operation = fixture.prepare().await;
    fixture
        .local(Request::BufferLanguage {
            worktree: fixture.root.clone(),
            path: "independent".into(),
        })
        .await
        .unwrap();
    fixture
        .local(Request::CloseBuffer {
            worktree: fixture.root.clone(),
            path: "independent".into(),
            lease_id: "independent".into(),
        })
        .await
        .unwrap();
    assert_eq!(fixture.buffers.active.read().await.len(), 1);
    fixture.act(&operation, Action::Retire).await.unwrap();
}

#[tokio::test]
async fn terminal_observation_repairs_interrupted_local_fence_cleanup() {
    let mut fixture = Fixture::new().await;
    let operation = fixture.prepare().await;
    let revision = fixture.zed.diagnostics.lock().unwrap().revision(1).unwrap();
    let (worktrees, buffers, zed) = (
        fixture.worktrees.clone(),
        fixture.buffers.clone(),
        fixture.zed.clone(),
    );
    let mut apply = Box::pin(respond(
        Request::BufferSync {
            operation: operation.clone(),
            action: Action::Apply,
        },
        &worktrees,
        &buffers,
        Some(&zed),
    ));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(apply.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let (id, _) = fixture.message(NativeAction::Apply).await;
    let held = fixture.buffers.active.write().await;
    fixture.reply(id, Phase::Applied);
    // Deterministically record the native outcome, then suspend on active.
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(apply.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(apply);
    drop(held);
    assert!(
        fixture.buffers.syncs.lock().await.slots[&1]
            .state
            .terminal()
    );
    // No corresponding native edit event was delivered by this fixture. The
    // Applied reply alone must invalidate the old snapshot and require its new
    // clock, rather than treating the still-old mirror as current content.
    assert!(
        fixture
            .zed
            .diagnostics
            .lock()
            .unwrap()
            .check(1, revision)
            .is_err()
    );
    assert!(fixture.zed.diagnostics.lock().unwrap().version(1).is_err());
    assert!(matches!(
        state(&fixture.act(&operation, Action::Query).await.unwrap()),
        State::Applied { .. }
    ));
    fixture
        .local(Request::ReleaseBufferLease {
            lease: fixture.lease.clone(),
        })
        .await
        .unwrap();
    assert!(fixture.receiver.try_recv().is_err());
}

#[tokio::test]
async fn terminal_capacity_is_not_evicted_to_admit_another_operation() {
    let fixture = Fixture::new().await;
    {
        let mut registry = fixture.buffers.syncs.lock().await;
        for _ in 0..MAX_OPERATIONS {
            let (id, _) = registry.allocate().unwrap();
            registry.slots.insert(
                id,
                Slot {
                    key: (fixture.root.clone(), "file".into()),
                    owner: 1,
                    remote_id: 1,
                    content: content(),
                    until: Instant::now().checked_sub(Duration::from_mins(1)).unwrap(),
                    ticket: None,
                    state: State::Refused {
                        reason: Reason::Source,
                    },
                },
            );
        }
    }
    assert!(
        fixture
            .preparing()
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("capacity")
    );
    assert_eq!(
        fixture.zed.next_message_id.load(crate::Ordering::Relaxed),
        1
    );
    assert_eq!(
        fixture.buffers.syncs.lock().await.slots.len(),
        MAX_OPERATIONS
    );
}

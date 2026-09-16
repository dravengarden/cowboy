use super::*;

#[tokio::test]
async fn observations_use_original_owner_after_file_disappears() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    fixture
        .request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
    std::fs::remove_file(fixture.root.join("file.txt")).unwrap();
    for request in [ReadRequest::Language {}, ReadRequest::Symbols {}] {
        let reply = fixture
            .request(Request::ReadBufferLease {
                lease: lease.clone(),
                request,
            })
            .await
            .unwrap();
        let Response::BufferLeaseRead {
            lease: observed,
            api_version,
            ..
        } = reply
        else {
            panic!("wrong response");
        };
        assert_eq!(observed, lease);
        assert_eq!(api_version, 1);
    }
    assert_eq!(fixture.buffers.active.read().await.len(), 1);
    fixture
        .request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
    assert!(fixture.buffers.active.read().await.is_empty());
}

#[tokio::test]
async fn only_open_owners_can_read_and_a_legacy_owner_cannot_substitute() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    let read = || Request::ReadBufferLease {
        lease: lease.clone(),
        request: ReadRequest::Language {},
    };
    assert!(fixture.request(read()).await.is_err());
    fixture
        .buffers
        .leases
        .lock()
        .await
        .slots
        .get_mut(&1)
        .unwrap()
        .state = Phase::Unknown;
    assert!(fixture.request(read()).await.is_err());
    fixture
        .buffers
        .leases
        .lock()
        .await
        .slots
        .get_mut(&1)
        .unwrap()
        .state = Phase::Prepared;
    fixture
        .request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
    {
        let mut active = fixture.buffers.active.write().await;
        let owners = &mut active.values_mut().next().unwrap().lease_ids;
        owners.clear();
        owners.insert(BufferOwner::Legacy(lease.id.clone()));
    }
    assert!(fixture.request(read()).await.is_err());
    fixture
        .buffers
        .active
        .write()
        .await
        .values_mut()
        .next()
        .unwrap()
        .lease_ids
        .insert(BufferOwner::Owned(1));
    fixture
        .request(Request::ReleaseBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
    assert!(fixture.request(read()).await.is_err());
    let mut foreign = lease;
    foreign.instance = "f".repeat(32);
    assert!(
        fixture
            .request(Request::ReadBufferLease {
                lease: foreign,
                request: ReadRequest::Symbols {}
            })
            .await
            .is_err()
    );
}

#[test]
fn owned_read_requests_cannot_supply_a_path_or_cursor() {
    for request in [
        serde_json::json!({"kind":"language","path":"other"}),
        serde_json::json!({"kind":"hover","offset":0}),
        serde_json::json!({"kind":"symbols","version":[]}),
    ] {
        assert!(serde_json::from_value::<ReadRequest>(request).is_err());
    }
}

#[tokio::test]
async fn language_transport_failure_is_not_an_empty_success() {
    use crate::{AtomicU32, AtomicU64, Command, Mutex, ZedRuntime, broadcast, mpsc};
    let (outbound, receiver) = mpsc::unbounded_channel();
    drop(receiver);
    let mut child = Command::new("true").spawn().unwrap();
    child.wait().await.unwrap();
    let zed = ZedRuntime {
        _child: Mutex::new(child),
        outbound,
        pending: Arc::default(),
        events: broadcast::channel(4).0,
        buffer_files: Arc::default(),
        worktree_paths: Arc::default(),
        diagnostics: Arc::default(),
        next_message_id: AtomicU32::new(1),
        next_lsp_request_id: AtomicU64::new(1),
    };
    zed.diagnostics
        .lock()
        .unwrap()
        .observe(&proto::envelope::Payload::CreateBufferForPeer(
            proto::CreateBufferForPeer {
                variant: Some(proto::create_buffer_for_peer::Variant::State(
                    proto::BufferState {
                        id: 1,
                        base_text: "fixture\n".into(),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
        ));
    zed.diagnostics
        .lock()
        .unwrap()
        .observe(&proto::envelope::Payload::CreateBufferForPeer(
            proto::CreateBufferForPeer {
                variant: Some(proto::create_buffer_for_peer::Variant::Chunk(
                    proto::BufferChunk {
                        buffer_id: 1,
                        is_last: true,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
        ));
    let error = zed
        .language(1)
        .await
        .err()
        .expect("dead transport succeeded");
    assert!(
        error.to_string().contains("Zed writer task stopped"),
        "{error:#}"
    );
}

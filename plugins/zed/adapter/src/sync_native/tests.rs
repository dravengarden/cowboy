use super::*;

pub(super) async fn fixture() -> (
    Arc<ZedRuntime>,
    mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
) {
    let (outbound, _) = mpsc::unbounded_channel();
    let mut child = Command::new("true").spawn().unwrap();
    child.wait().await.unwrap();
    let (sync, receiver) = Transport::new();
    let zed = Arc::new(ZedRuntime {
        _child: Mutex::new(child),
        outbound,
        pending: Arc::default(),
        sync,
        events: broadcast::channel(16).0,
        buffer_files: Arc::default(),
        worktree_paths: Arc::default(),
        diagnostics: Arc::default(),
        next_message_id: AtomicU32::new(1),
        next_lsp_request_id: AtomicU64::new(1),
    });
    (zed, receiver)
}

fn probe() -> wire::CowboyBufferSync {
    wire::CowboyBufferSync {
        protocol: 1,
        action: Action::Probe as i32,
        ..Default::default()
    }
}

fn supported() -> wire::CowboyBufferSyncResponse {
    wire::CowboyBufferSyncResponse {
        protocol: 1,
        instance: vec![3; 16],
        phase: Phase::Supported as i32,
        ..Default::default()
    }
}

fn encoded(id: u32, response: wire::CowboyBufferSyncResponse) -> Vec<u8> {
    wire::CowboyBufferSyncEnvelope {
        id: 19,
        responding_to: Some(id),
        payload: Some(Payload::Response(response)),
    }
    .encode_to_vec()
}

#[tokio::test]
async fn navigation_support_cannot_borrow_sync_or_upstream_support() {
    for wrong in [true, false] {
        let (zed, mut receiver) = fixture().await;
        let task = {
            let zed = zed.clone();
            tokio::spawn(async move { crate::navigation_native::support(Some(&zed)).await })
        };
        let request = receiver.recv().await.unwrap();
        let Some(Payload::NavigationRequest(value)) = request.payload else {
            panic!("not a navigation probe")
        };
        assert!(value.query.is_empty());
        let response = if wrong {
            encoded(request.id, supported())
        } else {
            wire::CowboyBufferSyncEnvelope {
                responding_to: Some(request.id),
                payload: Some(Payload::NavigationResponse(
                    wire::CowboyNavigationResponse {
                        protocol: 1,
                        outcome: wire::cowboy_navigation_response::Outcome::Supported as i32,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }
            .encode_to_vec()
        };
        assert!(zed.sync.response(request.id, &response));
        assert_eq!(task.await.unwrap().is_err(), wrong);
        assert!(receiver.try_recv().is_err());
    }
}

#[tokio::test]
async fn owner_support_is_distinct_effect_free_and_requires_the_actual_native_pair() {
    let worktrees = Arc::default();
    let buffers: Buffers = Arc::default();
    assert!(
        respond(
            Request::BufferSyncOwnerSupport {},
            &worktrees,
            &buffers,
            None
        )
        .await
        .is_err()
    );
    for extra in ["worktree", "lease", "authorized"] {
        assert!(
            serde_json::from_value::<Request>(
                serde_json::json!({"type":"bufferSyncOwnerSupport",extra:true})
            )
            .is_err()
        );
    }
    let (zed, mut receiver) = fixture().await;
    let call = respond(
        Request::BufferSyncOwnerSupport {},
        &worktrees,
        &buffers,
        Some(&zed),
    );
    tokio::pin!(call);
    let observed = tokio::select! {
        _ = &mut call => panic!("support did not probe the native peer"),
        value = receiver.recv() => value.unwrap(),
    };
    let Some(Payload::Request(request)) = observed.payload else {
        panic!("wrong native payload")
    };
    assert_eq!(request.action, Action::Probe as i32);
    zed.sync
        .response(observed.id, &encoded(observed.id, supported()));
    assert_eq!(
        serde_json::to_value(call.await.unwrap()).unwrap(),
        serde_json::json!({"type":"bufferSyncOwnerSupport","api_version":1,"protocol":1})
    );
    assert!(buffers.active.read().await.is_empty());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn closed_response_schema_rejects_foreign_identity_and_impossible_state() {
    let query = wire::CowboyBufferSync {
        protocol: 1,
        action: Action::Query as i32,
        instance: vec![3; 16],
        operation_id: 42,
        ..Default::default()
    };
    let applied = wire::CowboyBufferSyncResponse {
        operation_id: 42,
        phase: Phase::Applied as i32,
        content_sha256: vec![7; 32],
        content_bytes: 1,
        version: vec![wire::CowboyBufferSyncVersion {
            replica_id: 1,
            timestamp: 2,
        }],
        ..supported()
    };
    validate_response(&query, &applied).unwrap();
    for case in 0..13 {
        let mut bad = applied.clone();
        match case {
            0 => bad.instance[0] ^= 1,
            1 => bad.operation_id += 1,
            2 => bad.protocol = 2,
            3 => bad.phase = Phase::Supported as i32,
            4 => bad.phase = 42,
            5 => bad.refusal = Refusal::Changed as i32,
            6 => bad.content_sha256.pop().map(|_| ()).unwrap(),
            7 => bad.content_bytes = 4 * 1024 * 1024 + 1,
            8 => bad.version[0].replica_id = u32::MAX,
            9 => bad.version.push(bad.version[0].clone()),
            10 => bad.version[0].timestamp = 0,
            11 => bad.phase = Phase::Refused as i32,
            _ => bad.instance.clear(),
        }
        assert!(validate_response(&query, &bad).is_err(), "case {case}");
    }
    for phase in [Phase::Prepared, Phase::Pending, Phase::Retired] {
        let mut response = wire::CowboyBufferSyncResponse {
            operation_id: 42,
            phase: phase as i32,
            ..supported()
        };
        validate_response(&query, &response).unwrap();
        response.version = applied.version.clone();
        assert!(validate_response(&query, &response).is_err());
    }
    let mut retired = wire::CowboyBufferSync {
        action: Action::Retire as i32,
        ..query.clone()
    };
    assert!(validate_response(&retired, &applied).is_err());
    retired.action = Action::Apply as i32;
    assert!(
        validate_response(
            &retired,
            &wire::CowboyBufferSyncResponse {
                operation_id: 42,
                phase: Phase::Prepared as i32,
                ..supported()
            }
        )
        .is_err()
    );
}

#[test]
fn budget_refusal_is_correlated_and_cannot_carry_a_partial_result() {
    let query = wire::CowboyBufferSync {
        protocol: 1,
        action: Action::Query as i32,
        instance: vec![3; 16],
        operation_id: 42,
        ..Default::default()
    };
    let refused = wire::CowboyBufferSyncResponse {
        operation_id: 42,
        phase: Phase::Refused as i32,
        refusal: Refusal::Budget as i32,
        ..supported()
    };
    validate_response(&query, &refused).unwrap();
    for case in 0..5 {
        let mut bad = refused.clone();
        match case {
            0 => bad.content_bytes = 1,
            1 => bad.content_sha256 = vec![0; 32],
            2 => bad.refusal = 5,
            3 => bad.operation_id += 1,
            _ => bad.phase = Phase::Applied as i32,
        }
        assert!(validate_response(&query, &bad).is_err());
    }
}

#[tokio::test]
async fn malformed_requests_never_enter_native_queue() {
    let (zed, mut receiver) = fixture().await;
    for case in 0..7 {
        let mut request = probe();
        match case {
            0 => request.instance.push(1),
            1 => request.operation_id = 1,
            2 => request.buffer_id = 1,
            3 => request.action = 100,
            4 => request.project_id = 1,
            5 => request.protocol = 2,
            _ => request.content_bytes = 1,
        }
        assert!(zed.sync.request(&zed, request).await.is_err());
        assert!(receiver.try_recv().is_err());
    }
    assert_eq!(zed.next_message_id.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn cancellation_drops_only_the_observer_and_late_reply_is_not_reused() {
    let (zed, mut receiver) = fixture().await;
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.sync.probe(&zed).await })
    };
    let first = receiver.recv().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(zed.sync.pending.lock().unwrap().is_empty());
    assert!(!zed.sync.response(first.id, &encoded(first.id, supported())));
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.sync.probe(&zed).await })
    };
    let second = receiver.recv().await.unwrap();
    assert!(second.id > first.id);
    assert!(!zed.sync.response(first.id, &encoded(first.id, supported())));
    assert!(
        zed.sync
            .response(second.id, &encoded(second.id, supported()))
    );
    assert_eq!(task.await.unwrap().unwrap(), [3; 16]);
}

#[tokio::test]
async fn transport_capacity_and_message_identity_are_bounded() {
    let (zed, mut receiver) = fixture().await;
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let zed = zed.clone();
        tasks.push(tokio::spawn(async move { zed.sync.probe(&zed).await }));
        receiver.recv().await.unwrap();
    }
    assert!(
        zed.sync
            .probe(&zed)
            .await
            .unwrap_err()
            .to_string()
            .contains("capacity")
    );
    assert!(receiver.try_recv().is_err());
    zed.sync.stopped();
    for task in tasks {
        assert!(task.await.unwrap().is_err());
    }
    assert!(zed.sync.pending.lock().unwrap().is_empty());
    zed.next_message_id.store(1_000_000_000, Ordering::Relaxed);
    assert!(
        zed.sync
            .probe(&zed)
            .await
            .unwrap_err()
            .to_string()
            .contains("exhausted")
    );
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn reader_rejects_mixed_upstream_private_reply_and_clears_disconnects() {
    let (zed, mut receiver) = fixture().await;
    let (read, mut write) = UnixStream::pair().unwrap();
    let reader = tokio::spawn(read_messages(
        read,
        zed.outbound.clone(),
        zed.pending.clone(),
        zed.events.clone(),
        zed.buffer_files.clone(),
        zed.diagnostics.clone(),
        zed.sync.clone(),
    ));
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.sync.probe(&zed).await })
    };
    let request = receiver.recv().await.unwrap();
    let mut bytes = encoded(request.id, supported());
    proto::Envelope {
        payload: Some(proto::envelope::Payload::Ack(proto::Ack {})),
        ..Default::default()
    }
    .encode(&mut bytes)
    .unwrap();
    write
        .write_u32_le(u32::try_from(bytes.len()).unwrap())
        .await
        .unwrap();
    write.write_all(&bytes).await.unwrap();
    assert!(task.await.unwrap().is_err());
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { zed.sync.probe(&zed).await })
    };
    receiver.recv().await.unwrap();
    drop(write);
    reader.await.unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(zed.sync.pending.lock().unwrap().is_empty());
}

#[tokio::test]
async fn private_reply_survives_every_split_and_bytewise_framing() {
    for navigation in [false, true] {
        let bytes = if navigation {
            wire::CowboyBufferSyncEnvelope {
                responding_to: Some(1),
                payload: Some(Payload::NavigationResponse(
                    wire::CowboyNavigationResponse {
                        protocol: 1,
                        outcome: wire::cowboy_navigation_response::Outcome::Supported as i32,
                        ..Default::default()
                    },
                )),
                ..Default::default()
            }
            .encode_to_vec()
        } else {
            encoded(1, supported())
        };
        for split in 0..=bytes.len() {
            let (zed, mut receiver) = fixture().await;
            let (read, mut write) = UnixStream::pair().unwrap();
            let reader = tokio::spawn(read_messages(
                read,
                zed.outbound.clone(),
                zed.pending.clone(),
                zed.events.clone(),
                zed.buffer_files.clone(),
                zed.diagnostics.clone(),
                zed.sync.clone(),
            ));
            let task = {
                let zed = zed.clone();
                tokio::spawn(async move {
                    if navigation {
                        crate::navigation_native::support(Some(&zed)).await
                    } else {
                        zed.sync
                            .probe(&zed)
                            .await
                            .map(|value| assert_eq!(value, [3; 16]))
                    }
                })
            };
            assert_eq!(receiver.recv().await.unwrap().id, 1);
            for byte in u32::try_from(bytes.len()).unwrap().to_le_bytes() {
                write.write_all(&[byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
            write.write_all(&bytes[..split]).await.unwrap();
            for byte in &bytes[split..] {
                write.write_all(&[*byte]).await.unwrap();
                tokio::task::yield_now().await;
            }
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            drop(write);
            reader.await.unwrap();
        }
    }
}

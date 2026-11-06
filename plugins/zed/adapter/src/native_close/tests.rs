//! Synthetic private transport; final acceptance also uses the real static pair.
use super::*;

#[test]
fn unknown_navigation_keeps_possible_target_alive_until_original_result_recovers() {
    let source = (PathBuf::from("/root"), PathBuf::from("source"));
    let target = (PathBuf::from("/root"), PathBuf::from("target"));
    let buffer = |remote_id, owners| BufferLease {
        remote_id,
        lease_ids: owners,
        closing: false,
        version: Vec::new(),
        sync: None,
    };
    let mut active = HashMap::from([
        (
            source.clone(),
            buffer(
                7,
                HashSet::from([BufferOwner::Owned(1), BufferOwner::NavigationPending(9)]),
            ),
        ),
        (
            target.clone(),
            buffer(8, HashSet::from([BufferOwner::Owned(2)])),
        ),
    ]);
    assert!(
        Plan::checked(
            HashSet::from([target.clone()]),
            &BufferOwner::Owned(2),
            &active
        )
        .is_err()
    );
    assert!(
        Plan::checked(
            HashSet::from([source.clone()]),
            &BufferOwner::Owned(1),
            &active
        )
        .unwrap()
        .native_ids
        .is_empty()
    );
    active
        .get_mut(&source)
        .unwrap()
        .lease_ids
        .remove(&BufferOwner::NavigationPending(9));
    assert_eq!(
        Plan::checked(HashSet::from([target]), &BufferOwner::Owned(2), &active)
            .unwrap()
            .native_ids,
        [8]
    );
}

#[tokio::test]
async fn shared_alias_cannot_drop_its_last_independent_owner_during_recovery() {
    let (mut zed, _) = coordinate_queries::fixture().await;
    let (transport, mut receiver) = sync_native::Transport::new();
    Arc::get_mut(&mut zed).unwrap().sync = transport;
    let key = |name: &str| (PathBuf::from("/root"), PathBuf::from(name));
    let keys = HashSet::from([key("source"), key("target")]);
    let owner = BufferOwner::Navigation(9);
    let buffer = |remote_id, owner, closing| BufferLease {
        remote_id,
        lease_ids: HashSet::from([owner]),
        closing,
        version: Vec::new(),
        sync: None,
    };
    let mut active = HashMap::from([
        (key("source"), buffer(7, owner.clone(), false)),
        (key("alias"), buffer(7, BufferOwner::Owned(2), false)),
        (key("target"), buffer(8, owner.clone(), true)),
    ]);
    zed.closes
        .begin(
            Owner::buffer(&owner, &keys),
            request(vec![3; 16], vec![8]),
            BTreeMap::from([(key("source"), 7), (key("target"), 8)]),
        )
        .unwrap();
    assert!(
        release(
            HashSet::from([key("alias")]),
            &BufferOwner::Owned(2),
            &mut active,
            Some(&zed),
            || panic!("shared alias admitted")
        )
        .await
        .is_err()
    );
    assert_eq!(active.len(), 3);
    assert!(receiver.try_recv().is_err());
    let (recovered, ()) = tokio::join!(recover(&keys, &owner, &mut active, &zed), async {
        let envelope = receiver.recv().await.unwrap();
        let Some(Payload::CloseRequest(request)) = &envelope.payload else {
            panic!("not query")
        };
        assert_eq!(request.action, Action::Query as i32);
        assert_eq!(request.buffer_ids, [8]);
        reply(&zed.sync, envelope, Outcome::Closed);
    });
    assert!(recovered.unwrap());
    assert_eq!(active.len(), 1);
    assert!(
        active[&key("alias")]
            .lease_ids
            .contains(&BufferOwner::Owned(2))
    );
    reply(
        &zed.sync,
        receiver.recv().await.unwrap(),
        Outcome::Forgotten,
    );
    let (released, ()) = tokio::join!(
        release(
            HashSet::from([key("alias")]),
            &BufferOwner::Owned(2),
            &mut active,
            Some(&zed),
            || {}
        ),
        async {
            reply(
                &zed.sync,
                receiver.recv().await.unwrap(),
                Outcome::Supported,
            );
            assert_eq!(
                reply(&zed.sync, receiver.recv().await.unwrap(), Outcome::Closed),
                [7]
            );
        }
    );
    released.unwrap();
    assert!(active.is_empty());
    reply(
        &zed.sync,
        receiver.recv().await.unwrap(),
        Outcome::Forgotten,
    );
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn orphan_close_recovers_only_the_original_receipt_after_cancellation() {
    let (mut zed, _) = coordinate_queries::fixture().await;
    let (transport, mut receiver) = sync_native::Transport::new();
    Arc::get_mut(&mut zed).unwrap().sync = transport;
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { release_unowned(&zed, 7).await })
    };
    reply(
        &zed.sync,
        receiver.recv().await.unwrap(),
        Outcome::Supported,
    );
    let effect = receiver.recv().await.unwrap();
    let Some(Payload::CloseRequest(original)) = effect.payload else {
        panic!("not original close");
    };
    assert_eq!(original.action, Action::Close as i32);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let task = {
        let zed = zed.clone();
        tokio::spawn(async move { release_unowned(&zed, 7).await })
    };
    let query = receiver.recv().await.unwrap();
    let Some(Payload::CloseRequest(request)) = &query.payload else {
        panic!("not original close query");
    };
    assert_eq!(request.action, Action::Query as i32);
    assert_eq!(request.operation_id, original.operation_id);
    assert_eq!(request.instance, original.instance);
    assert_eq!(request.buffer_ids, original.buffer_ids);
    reply(&zed.sync, query, Outcome::Closed);
    task.await.unwrap().unwrap();
    assert!(zed.closes.get(&Owner::Unowned(7)).is_none());
    let retirement = receiver.recv().await.unwrap();
    let Some(Payload::CloseRequest(request)) = &retirement.payload else {
        panic!("not retirement");
    };
    assert_eq!(request.action, Action::Forget as i32);
    assert_eq!(request.operation_id, original.operation_id);
    reply(&zed.sync, retirement, Outcome::Forgotten);
    assert!(receiver.try_recv().is_err());
}

pub(crate) fn reply(
    transport: &sync_native::Transport,
    envelope: wire::CowboyBufferSyncEnvelope,
    outcome: Outcome,
) -> Vec<u64> {
    let Some(Payload::CloseRequest(request)) = envelope.payload else {
        panic!("not a native close request")
    };
    let ids = request.buffer_ids;
    let encoded = wire::CowboyBufferSyncEnvelope {
        responding_to: Some(envelope.id),
        payload: Some(Payload::CloseResponse(wire::CowboyCloseBuffersResponse {
            protocol: request.protocol,
            operation_id: request.operation_id,
            instance: if request.instance.is_empty() {
                vec![3; 16]
            } else {
                request.instance
            },
            outcome: outcome as i32,
            buffer_ids: ids.clone(),
        })),
        ..Default::default()
    }
    .encode_to_vec();
    let observed = transport.response(envelope.id, &encoded);
    assert_eq!(observed, request.action != Action::Forget as i32);
    ids
}

// Route other private operations unchanged. Close observations are the actual
// adapter request sets, not fabricated upstream CloseBuffer dispatches.
pub(crate) fn fixture() -> (
    Arc<sync_native::Transport>,
    mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
    mpsc::UnboundedReceiver<Vec<u64>>,
) {
    let (transport, mut receiver) = sync_native::Transport::new();
    let weak = Arc::downgrade(&transport);
    let (forward, forwarded) = mpsc::channel(32);
    let (observed, observations) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(envelope) = receiver.recv().await {
            if let Some(Payload::CloseRequest(request)) = &envelope.payload {
                let action = request.action;
                let outcome = if action == Action::Forget as i32 {
                    Outcome::Forgotten
                } else if request.buffer_ids.is_empty() {
                    Outcome::Supported
                } else {
                    Outcome::Closed
                };
                let Some(transport) = weak.upgrade() else {
                    break;
                };
                let ids = reply(&transport, envelope, outcome);
                if !ids.is_empty() && action == Action::Close as i32 {
                    let _ = observed.send(ids);
                }
            } else if forward.send(envelope).await.is_err() {
                break;
            }
        }
    });
    (transport, forwarded, observations)
}

#[test]
fn close_confirmation_requires_exact_instance_set_protocol_and_outcome() {
    let mut request = request(vec![3; 16], vec![7, 8]);
    request.operation_id = 1;
    let good = wire::CowboyCloseBuffersResponse {
        protocol: 2,
        operation_id: 1,
        instance: vec![3; 16],
        outcome: Outcome::Closed as i32,
        buffer_ids: vec![7, 8],
    };
    assert!(decode(&request, good.clone()).is_ok());
    for case in 0..9 {
        let mut bad = good.clone();
        match case {
            0 => bad.protocol = 1,
            1 => bad.instance[0] ^= 1,
            2 => bad.instance.clear(),
            3 => bad.buffer_ids.clear(),
            4 => bad.buffer_ids.reverse(),
            5 => bad.buffer_ids.push(9),
            6 => bad.outcome = Outcome::Supported as i32,
            7 => bad.outcome = Outcome::Refused as i32,
            _ => bad.outcome = 99,
        }
        assert!(decode(&request, bad).is_err(), "case {case}");
    }
    assert!(decode(&super::request(Vec::new(), Vec::new()), good).is_err());
}

#[test]
fn complete_alias_plan_closes_each_native_id_once_and_preserves_independent_pins() {
    let owner = BufferOwner::Navigation(1);
    let key = |name: &str| (PathBuf::from("/root"), PathBuf::from(name));
    let buffer = |id, owners| BufferLease {
        remote_id: id,
        lease_ids: owners,
        version: Vec::new(),
        sync: None,
        closing: false,
    };
    let mut active = HashMap::from([
        (
            key("a"),
            buffer(7, HashSet::from([BufferOwner::Navigation(1)])),
        ),
        (
            key("alias-a"),
            buffer(7, HashSet::from([BufferOwner::Navigation(1)])),
        ),
        (
            key("b"),
            buffer(
                8,
                HashSet::from([BufferOwner::Navigation(1), BufferOwner::Owned(2)]),
            ),
        ),
    ]);
    let keys = active.keys().cloned().collect();
    assert_eq!(
        Plan::checked(keys, &owner, &active).unwrap().native_ids,
        [7]
    );
    assert!(Plan::checked(HashSet::from([key("missing")]), &owner, &active).is_err());
    assert!(Plan::checked(HashSet::new(), &owner, &active).is_err());
    active.get_mut(&key("alias-a")).unwrap().closing = true;
    assert!(Plan::checked(active.keys().cloned().collect(), &owner, &active).is_err());
}

#[tokio::test]
async fn unsupported_close_does_not_dispatch_or_remove_the_original_owner() {
    let (mut zed, mut upstream) = coordinate_queries::fixture().await;
    let (transport, mut receiver) = sync_native::Transport::new();
    Arc::get_mut(&mut zed).unwrap().sync = transport;
    let buffers: Buffers = Arc::default();
    let key = (PathBuf::from("/root"), PathBuf::from("a"));
    buffers.active.write().await.insert(
        key.clone(),
        BufferLease {
            remote_id: 7,
            lease_ids: HashSet::from([BufferOwner::Owned(1)]),
            version: Vec::new(),
            sync: None,
            closing: false,
        },
    );
    let task = {
        let (buffers, zed, key) = (buffers.clone(), zed.clone(), key.clone());
        tokio::spawn(async move {
            crate::close_buffer_at(
                key.0,
                key.1,
                BufferOwner::Owned(1),
                &buffers,
                Some(&zed),
                || panic!("unsupported pair admitted release"),
            )
            .await
        })
    };
    let probe = receiver.recv().await.unwrap();
    reply(&zed.sync, probe, Outcome::Refused);
    assert!(task.await.unwrap().is_err());
    let active = buffers.active.read().await;
    assert!(!active[&key].closing);
    assert_eq!(active[&key].lease_ids.len(), 1);
    assert!(upstream.try_recv().is_err());
    assert!(receiver.try_recv().is_err());
}

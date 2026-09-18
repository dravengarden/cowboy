//! Synthetic private transport; final acceptance also uses the real static pair.
use super::*;

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
            protocol: 1,
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
    assert!(transport.response(envelope.id, &encoded));
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
                let outcome = if request.buffer_ids.is_empty() {
                    Outcome::Supported
                } else {
                    Outcome::Closed
                };
                let Some(transport) = weak.upgrade() else {
                    break;
                };
                let ids = reply(&transport, envelope, outcome);
                if !ids.is_empty() {
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
    let request = request(vec![3; 16], vec![7, 8]);
    let good = wire::CowboyCloseBuffersResponse {
        protocol: 1,
        instance: vec![3; 16],
        outcome: Outcome::Closed as i32,
        buffer_ids: vec![7, 8],
    };
    assert!(decode(&request, good.clone()).is_ok());
    for case in 0..9 {
        let mut bad = good.clone();
        match case {
            0 => bad.protocol = 2,
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

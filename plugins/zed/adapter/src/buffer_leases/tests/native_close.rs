use super::*;
use crate::native_close::tests::reply;
use crate::sync_native::{Transport, wire};
use crate::{Zed, coordinate_queries};
use prost::Message as _;
use tokio::sync::mpsc;
use wire::cowboy_buffer_sync_envelope::Payload;
use wire::cowboy_close_buffers_response::Outcome;

async fn fixture() -> (
    Fixture,
    LeaseRef,
    Zed,
    mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
) {
    let f = Fixture::new().await;
    let lease = f.prepare().await;
    f.request(Request::OpenBufferLease {
        lease: lease.clone(),
    })
    .await
    .unwrap();
    f.buffers
        .active
        .write()
        .await
        .values_mut()
        .next()
        .unwrap()
        .remote_id = 7;
    let (mut zed, _) = coordinate_queries::fixture().await;
    let (transport, receiver) = Transport::new();
    Arc::get_mut(&mut zed).unwrap().sync = transport;
    (f, lease, zed, receiver)
}

fn release(f: &Fixture, zed: &Zed, lease: &LeaseRef) -> tokio::task::JoinHandle<Result<Response>> {
    let (worktrees, buffers, zed, lease) = (
        f.worktrees.clone(),
        f.buffers.clone(),
        zed.clone(),
        lease.clone(),
    );
    tokio::spawn(async move {
        respond(
            Request::ReleaseBufferLease { lease },
            &worktrees,
            &buffers,
            Some(&zed),
        )
        .await
    })
}

async fn admission(
    zed: &Zed,
    receiver: &mut mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
) -> wire::CowboyBufferSyncEnvelope {
    let probe = receiver.recv().await.unwrap();
    assert!(reply(&zed.sync, probe, Outcome::Supported).is_empty());
    receiver.recv().await.unwrap()
}

fn closed(envelope: &wire::CowboyBufferSyncEnvelope) -> wire::CowboyCloseBuffersResponse {
    let Some(Payload::CloseRequest(request)) = &envelope.payload else {
        panic!("not close");
    };
    wire::CowboyCloseBuffersResponse {
        protocol: 1,
        instance: request.instance.clone(),
        outcome: Outcome::Closed as i32,
        buffer_ids: request.buffer_ids.clone(),
    }
}

fn deliver(zed: &Zed, id: u32, payload: Payload) -> bool {
    zed.sync.response(
        id,
        &wire::CowboyBufferSyncEnvelope {
            responding_to: Some(id),
            payload: Some(payload),
            ..Default::default()
        }
        .encode_to_vec(),
    )
}

async fn uncertain(f: &Fixture, zed: &Zed, lease: &LeaseRef) {
    for request in [
        Request::QueryBufferLease {
            lease: lease.clone(),
        },
        Request::OpenBufferLease {
            lease: lease.clone(),
        },
        Request::ReleaseBufferLease {
            lease: lease.clone(),
        },
    ] {
        assert_eq!(
            state(
                respond(request, &f.worktrees, &f.buffers, Some(zed))
                    .await
                    .unwrap()
            ),
            LeaseState::Unknown
        );
    }
    let next = f.prepare().await;
    assert!(
        respond(
            Request::OpenBufferLease {
                lease: next.clone()
            },
            &f.worktrees,
            &f.buffers,
            Some(zed)
        )
        .await
        .is_err()
    );
    assert_eq!(
        state(
            f.request(Request::QueryBufferLease { lease: next })
                .await
                .unwrap()
        ),
        LeaseState::Prepared
    );
    let active = f.buffers.active.read().await;
    assert_eq!(active.len(), 1);
    assert!(active.values().next().unwrap().closing);
    assert!(crate::sync_owners::ensure_admission(&f.buffers, &active).is_err());
    assert!(crate::sync_owners::ensure_readable(active.values().next().unwrap()).is_err());
    assert!(zed.diagnostics.lock().unwrap().revision(7).is_ok());
}

#[tokio::test]
async fn lost_native_close_observer_keeps_original_owner_and_late_reply_cannot_clear_fence() {
    let (f, lease, zed, mut receiver) = fixture().await;
    let task = release(&f, &zed, &lease);
    let effect = admission(&zed, &mut receiver).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(!deliver(
        &zed,
        effect.id,
        Payload::CloseResponse(closed(&effect))
    ));
    uncertain(&f, &zed, &lease).await;
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn wrong_native_close_replies_never_complete_or_replay_release() {
    for case in 0..4 {
        let (f, lease, zed, mut receiver) = fixture().await;
        let task = release(&f, &zed, &lease);
        let effect = admission(&zed, &mut receiver).await;
        let mut response = closed(&effect);
        match case {
            0 => response.instance[0] ^= 1,
            1 => response.buffer_ids.clear(),
            2 => response.outcome = Outcome::Refused as i32,
            _ => response.outcome = Outcome::Supported as i32,
        }
        assert!(deliver(&zed, effect.id, Payload::CloseResponse(response)));
        assert!(task.await.unwrap().is_err());
        uncertain(&f, &zed, &lease).await;
        assert!(receiver.try_recv().is_err());
    }
}

#[tokio::test]
async fn real_native_close_deadline_keeps_unknown_without_retry_or_cleanup() {
    let (f, lease, zed, mut receiver) = fixture().await;
    let task = release(&f, &zed, &lease);
    admission(&zed, &mut receiver).await;
    let error = tokio::time::timeout(Duration::from_secs(32), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("observation timed out"));
    uncertain(&f, &zed, &lease).await;
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn cancellation_during_effect_free_probe_does_not_admit_close() {
    let (f, lease, zed, mut receiver) = fixture().await;
    let task = release(&f, &zed, &lease);
    let probe = receiver.recv().await.unwrap();
    let Some(Payload::CloseRequest(request)) = probe.payload else {
        panic!("not probe");
    };
    assert!(request.buffer_ids.is_empty());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        state(
            f.request(Request::QueryBufferLease {
                lease: lease.clone()
            })
            .await
            .unwrap()
        ),
        LeaseState::Open
    );
    assert!(
        !f.buffers
            .active
            .read()
            .await
            .values()
            .next()
            .unwrap()
            .closing
    );
    assert!(receiver.try_recv().is_err());
    let task = release(&f, &zed, &lease);
    let effect = admission(&zed, &mut receiver).await;
    assert_eq!(reply(&zed.sync, effect, Outcome::Closed), [7]);
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Released);
    assert!(f.buffers.active.read().await.is_empty());
    assert!(zed.diagnostics.lock().unwrap().revision(7).is_err());
    assert_eq!(
        state(
            f.request(Request::ReleaseBufferLease { lease })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn independently_confirmed_close_does_not_clear_another_unknown_close() {
    let (f, lease, zed, mut receiver) = fixture().await;
    std::fs::write(f.root.join("other.txt"), "other\n").unwrap();
    let Response::BufferLease { lease: other, .. } = f
        .request(Request::PrepareBuffer {
            worktree: f.root.clone(),
            path: "other.txt".into(),
        })
        .await
        .unwrap()
    else {
        panic!("not prepared");
    };
    f.request(Request::OpenBufferLease {
        lease: other.clone(),
    })
    .await
    .unwrap();
    let task = release(&f, &zed, &lease);
    admission(&zed, &mut receiver).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let task = release(&f, &zed, &other);
    let effect = admission(&zed, &mut receiver).await;
    assert_eq!(reply(&zed.sync, effect, Outcome::Closed), [2]);
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Released);
    uncertain(&f, &zed, &lease).await;
    assert!(receiver.try_recv().is_err());
}

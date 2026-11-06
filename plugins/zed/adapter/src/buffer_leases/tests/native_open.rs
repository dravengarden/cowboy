use super::*;
use crate::native_open::tests::{ack, opened, reply, share};
use crate::sync_native::wire::{self, cowboy_buffer_sync_envelope::Payload};
use crate::{Zed, coordinate_queries};
use prost::Message as _;
use wire::cowboy_open_attempt_response::Outcome;

type Outbound = tokio::sync::mpsc::UnboundedReceiver<proto::Envelope>;

/// Run `work` while answering recovery's reachability barrier with an error:
/// the outcome stays unresolved. Any other native dispatch fails the test.
async fn unreachable_recovery<T>(
    zed: &Zed,
    outbound: &mut Outbound,
    work: impl Future<Output = T>,
) -> T {
    tokio::pin!(work);
    loop {
        tokio::select! {
            value = &mut work => return value,
            Some(message) = outbound.recv() => {
                assert!(
                    matches!(message.payload, Some(proto::envelope::Payload::Ping(_))),
                    "recovery may only query: {message:?}"
                );
                reply(zed, &message, proto::envelope::Payload::Error(proto::Error {
                    message: "unreachable".into(),
                    ..Default::default()
                })).await;
            }
        }
    }
}

fn spawn(f: &Fixture, zed: &Zed, request: Request) -> tokio::task::JoinHandle<Result<Response>> {
    let (worktrees, buffers, zed) = (f.worktrees.clone(), f.buffers.clone(), zed.clone());
    tokio::spawn(async move { respond(request, &worktrees, &buffers, Some(&zed)).await })
}

async fn request(f: &Fixture, zed: &Zed, request: Request) -> Result<Response> {
    respond(request, &f.worktrees, &f.buffers, Some(zed)).await
}

async fn prepare_other(f: &Fixture) -> LeaseRef {
    std::fs::write(f.root.join("other.txt"), "other\n").unwrap();
    let Response::BufferLease { lease, .. } = f
        .request(Request::PrepareBuffer {
            worktree: f.root.clone(),
            path: "other.txt".into(),
        })
        .await
        .unwrap()
    else {
        panic!("not prepared")
    };
    lease
}

async fn assert_unknown(f: &Fixture, zed: &Zed, lease: &LeaseRef) {
    for request_value in [
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
            state(request(f, zed, request_value).await.unwrap()),
            LeaseState::Unknown
        );
    }
    let next = f.prepare().await;
    assert!(
        request(
            f,
            zed,
            Request::OpenBufferLease {
                lease: next.clone()
            }
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
    assert!(
        request(
            f,
            zed,
            Request::OpenBuffer {
                worktree: f.root.clone(),
                path: "file.txt".into(),
                lease_id: "replacement".into(),
            }
        )
        .await
        .is_err()
    );
    assert!(f.buffers.native_open.check().is_err());
}

#[tokio::test]
async fn cancelled_acquisition_at_every_native_await_cannot_be_adopted_or_replayed() {
    for stage in 0..3 {
        let f = Fixture::new().await;
        let (zed, mut outbound) = coordinate_queries::fixture().await;
        let lease = f.prepare().await;
        let task = spawn(
            &f,
            &zed,
            Request::OpenBufferLease {
                lease: lease.clone(),
            },
        );
        let opening = outbound.recv().await.unwrap();
        if stage > 0 {
            if stage == 2 {
                share(&zed, 8);
            }
            reply(&zed, &opening, opened(8)).await;
            if stage == 2 {
                let registration = outbound.recv().await.unwrap();
                assert!(matches!(
                    registration.payload,
                    Some(proto::envelope::Payload::RegisterBufferWithLanguageServers(
                        _
                    ))
                ));
            } else {
                tokio::task::yield_now().await;
            }
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        Box::pin(unreachable_recovery(
            &zed,
            &mut outbound,
            assert_unknown(&f, &zed, &lease),
        ))
        .await;
        assert!(f.buffers.active.read().await.is_empty());
        assert!(
            outbound.try_recv().is_err(),
            "cancel dispatched native cleanup/replay"
        );
        // A late real reply cannot commit or clear the abandoned attempt.
        for (id, sender) in zed.pending.lock().await.drain() {
            assert!(
                sender
                    .send(proto::Envelope {
                        responding_to: Some(id),
                        payload: Some(ack()),
                        ..Default::default()
                    })
                    .is_err()
            );
        }
        assert!(f.buffers.native_open.check().is_err());
    }
}

#[tokio::test]
async fn invalid_open_and_registration_replies_leave_the_same_unknown_fence() {
    for stage in 0..3 {
        let f = Fixture::new().await;
        let (zed, mut outbound) = coordinate_queries::fixture().await;
        let lease = f.prepare().await;
        let task = spawn(
            &f,
            &zed,
            Request::OpenBufferLease {
                lease: lease.clone(),
            },
        );
        let opening = outbound.recv().await.unwrap();
        match stage {
            0 => reply(&zed, &opening, ack()).await,
            1 => reply(&zed, &opening, opened(0)).await,
            _ => {
                share(&zed, 8);
                reply(&zed, &opening, opened(8)).await;
                let registration = outbound.recv().await.unwrap();
                reply(&zed, &registration, opened(8)).await;
            }
        }
        assert!(task.await.unwrap().is_err());
        Box::pin(unreachable_recovery(
            &zed,
            &mut outbound,
            assert_unknown(&f, &zed, &lease),
        ))
        .await;
        assert!(outbound.try_recv().is_err());
    }
}

#[tokio::test]
async fn commit_ends_fence_only_after_original_owner_is_registered_and_retained() {
    let f = Fixture::new().await;
    let (zed, mut outbound) = coordinate_queries::fixture().await;
    let lease = f.prepare().await;
    let task = spawn(
        &f,
        &zed,
        Request::OpenBufferLease {
            lease: lease.clone(),
        },
    );
    let opening = outbound.recv().await.unwrap();
    share(&zed, 8);
    reply(&zed, &opening, opened(8)).await;
    let registration = outbound.recv().await.unwrap();
    assert!(f.buffers.native_open.check().is_err());
    reply(&zed, &registration, ack()).await;
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
    assert!(f.buffers.native_open.check().is_ok());
    assert_eq!(
        f.buffers
            .active
            .read()
            .await
            .values()
            .next()
            .unwrap()
            .remote_id,
        8
    );
    // A second explicit owner of the committed key needs no native I/O.
    let other = f.prepare().await;
    assert_eq!(
        state(
            request(&f, &zed, Request::OpenBufferLease { lease: other })
                .await
                .unwrap()
        ),
        LeaseState::Open
    );
    assert!(outbound.try_recv().is_err());
}

#[tokio::test]
async fn unknown_legacy_open_blocks_new_acquisitions_but_not_existing_safe_reads_or_release() {
    let f = Fixture::new().await;
    let (zed, mut outbound) = coordinate_queries::fixture().await;
    let known = f.prepare().await;
    f.request(Request::OpenBufferLease {
        lease: known.clone(),
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
    let prepared = prepare_other(&f).await;
    let task = spawn(
        &f,
        &zed,
        Request::OpenBuffer {
            worktree: f.root.clone(),
            path: "other.txt".into(),
            lease_id: "legacy".into(),
        },
    );
    let opening = outbound.recv().await.unwrap();
    reply(&zed, &opening, ack()).await;
    assert!(task.await.unwrap().is_err());
    // Its recovery query finds the server unreachable: still refused.
    assert!(
        Box::pin(unreachable_recovery(
            &zed,
            &mut outbound,
            request(&f, &zed, Request::OpenBufferLease { lease: prepared })
        ))
        .await
        .is_err()
    );
    assert!(
        request(
            &f,
            &zed,
            Request::BufferNavigate {
                worktree: f.root.clone(),
                path: "file.txt".into(),
                row: 0,
                column: 1,
                kind: crate::NavigationKind::Definition,
            }
        )
        .await
        .is_err()
    );
    let content = zed.diagnostics.lock().unwrap().content(7).unwrap();
    assert!(
        request(
            &f,
            &zed,
            Request::PrepareBufferSync {
                lease: known.clone(),
                purpose: crate::sync_owners::Purpose::RefreshFromDisk,
                content: content.clone(),
            }
        )
        .await
        .is_err()
    );
    assert!(
        request(
            &f,
            &zed,
            Request::PrepareBufferNavigation {
                lease: known.clone(),
                content: content.clone(),
                position: crate::content_reads::Point { row: 0, column: 1 },
                kind: crate::NavigationKind::Definition,
            }
        )
        .await
        .is_err()
    );
    assert_text_read(&f, &zed, &known, content).await;
    assert!(outbound.try_recv().is_err());
    assert_eq!(
        state(
            request(&f, &zed, Request::ReleaseBufferLease { lease: known })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
    assert!(zed.diagnostics.lock().unwrap().revision(7).is_err());
    assert!(outbound.try_recv().is_err());
    assert!(
        f.buffers.native_open.check().is_err(),
        "unrelated release cleared uncertainty"
    );
}

async fn assert_text_read(
    f: &Fixture,
    zed: &Zed,
    lease: &LeaseRef,
    content: crate::content_reads::Content,
) {
    let response = request(
        f,
        zed,
        Request::ReadBufferLease {
            lease: lease.clone(),
            request: ReadRequest::Text {
                content,
                page: crate::text_reads::Page::Start {},
            },
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        response,
        Response::BufferLeaseRead {
            result: ReadOutput::Text { .. },
            ..
        }
    ));
}

async fn answer_attempt(
    zed: &Zed,
    private: &mut tokio::sync::mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
    message_id: u32,
    outcome: Outcome,
    buffer_id: u64,
) {
    let envelope = private.recv().await.unwrap();
    let Some(Payload::OpenAttemptRequest(query)) = &envelope.payload else {
        panic!("not an open attempt query: {envelope:?}")
    };
    assert_eq!((query.protocol, query.message_id), (1, message_id));
    let encoded = wire::CowboyBufferSyncEnvelope {
        responding_to: Some(envelope.id),
        payload: Some(Payload::OpenAttemptResponse(
            wire::CowboyOpenAttemptResponse {
                protocol: 1,
                outcome: outcome as i32,
                buffer_id,
            },
        )),
        ..Default::default()
    }
    .encode_to_vec();
    assert!(zed.sync.response(envelope.id, &encoded));
}

/// An owned open abandoned after its `OpenBufferByPath` was queued.
async fn abandoned_open(f: &Fixture, zed: &Zed, outbound: &mut Outbound) -> u32 {
    let lease = f.prepare().await;
    let task = spawn(f, zed, Request::OpenBufferLease { lease });
    let opening = outbound.recv().await.unwrap();
    assert!(matches!(
        opening.payload,
        Some(proto::envelope::Payload::OpenBufferByPath(_))
    ));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(f.buffers.native_open.check().is_err());
    opening.id
}

async fn barrier(zed: &Zed, outbound: &mut Outbound) {
    let ping = outbound.recv().await.unwrap();
    assert!(matches!(
        ping.payload,
        Some(proto::envelope::Payload::Ping(_))
    ));
    reply(zed, &ping, ack()).await;
}

async fn ordinary_open(zed: &Zed, outbound: &mut Outbound, id: u64) {
    let opening = outbound.recv().await.unwrap();
    assert!(matches!(
        opening.payload,
        Some(proto::envelope::Payload::OpenBufferByPath(_))
    ));
    share(zed, id);
    reply(zed, &opening, opened(id)).await;
    let registration = outbound.recv().await.unwrap();
    reply(zed, &registration, ack()).await;
}

#[tokio::test]
async fn recovery_releases_the_orphan_an_abandoned_open_acquired() {
    let f = Fixture::new().await;
    let (sync, mut private, mut closed) = crate::native_close::tests::fixture();
    let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
    let message_id = abandoned_open(&f, &zed, &mut outbound).await;
    let next = prepare_other(&f).await;
    let task = spawn(&f, &zed, Request::OpenBufferLease { lease: next });
    barrier(&zed, &mut outbound).await;
    answer_attempt(&zed, &mut private, message_id, Outcome::Opened, 8).await;
    // Confirmed close of exactly the orphan, then an ordinary open.
    assert_eq!(closed.recv().await.unwrap(), vec![8]);
    ordinary_open(&zed, &mut outbound, 9).await;
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
    assert!(f.buffers.native_open.check().is_ok());
    assert!(outbound.try_recv().is_err());
}

#[tokio::test]
async fn recovery_clears_failed_and_unreceived_opens_without_any_effect() {
    for outcome in [Outcome::Failed, Outcome::Unknown] {
        let f = Fixture::new().await;
        let (sync, mut private, mut closed) = crate::native_close::tests::fixture();
        let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
        let message_id = abandoned_open(&f, &zed, &mut outbound).await;
        let next = prepare_other(&f).await;
        let task = spawn(&f, &zed, Request::OpenBufferLease { lease: next });
        barrier(&zed, &mut outbound).await;
        answer_attempt(&zed, &mut private, message_id, outcome, 0).await;
        ordinary_open(&zed, &mut outbound, 9).await;
        assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
        assert!(closed.try_recv().is_err(), "nothing to release");
    }
}

#[tokio::test]
async fn recovery_never_closes_an_alias_held_by_another_owner() {
    let f = Fixture::new().await;
    let (sync, mut private, mut closed) = crate::native_close::tests::fixture();
    let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
    // One owner already holds native buffer 8.
    let held = f.prepare().await;
    let task = spawn(&f, &zed, Request::OpenBufferLease { lease: held });
    ordinary_open(&zed, &mut outbound, 8).await;
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
    let message_id = {
        std::fs::write(f.root.join("alias.txt"), "fixture\n").unwrap();
        let Response::BufferLease { lease, .. } = f
            .request(Request::PrepareBuffer {
                worktree: f.root.clone(),
                path: "alias.txt".into(),
            })
            .await
            .unwrap()
        else {
            panic!("not prepared")
        };
        let task = spawn(&f, &zed, Request::OpenBufferLease { lease });
        let opening = outbound.recv().await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        opening.id
    };
    let next = prepare_other(&f).await;
    let task = spawn(&f, &zed, Request::OpenBufferLease { lease: next });
    barrier(&zed, &mut outbound).await;
    answer_attempt(&zed, &mut private, message_id, Outcome::Opened, 8).await;
    ordinary_open(&zed, &mut outbound, 9).await;
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
    assert!(closed.try_recv().is_err(), "an alias is not an orphan");
}

#[tokio::test]
async fn a_pending_open_stays_fenced() {
    let f = Fixture::new().await;
    let (sync, mut private, mut closed) = crate::native_close::tests::fixture();
    let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
    let message_id = abandoned_open(&f, &zed, &mut outbound).await;
    let next = prepare_other(&f).await;
    let task = spawn(&f, &zed, Request::OpenBufferLease { lease: next });
    barrier(&zed, &mut outbound).await;
    answer_attempt(&zed, &mut private, message_id, Outcome::Pending, 0).await;
    let error = task.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("still pending"), "{error:#}");
    assert!(f.buffers.native_open.check().is_err());
    assert!(closed.try_recv().is_err());
    assert!(outbound.try_recv().is_err());
}

async fn answer_target(
    zed: &Zed,
    private: &mut tokio::sync::mpsc::Receiver<wire::CowboyBufferSyncEnvelope>,
    state: wire::cowboy_open_target_response::State,
    buffer_id: u64,
) {
    let envelope = private.recv().await.unwrap();
    let Some(Payload::OpenTargetRequest(query)) = &envelope.payload else {
        panic!("not an open target query: {envelope:?}")
    };
    assert_eq!((query.protocol, query.path.as_str()), (1, "other.txt"));
    let encoded = wire::CowboyBufferSyncEnvelope {
        responding_to: Some(envelope.id),
        payload: Some(Payload::OpenTargetResponse(
            wire::CowboyOpenTargetResponse {
                protocol: 1,
                state: state as i32,
                buffer_id,
            },
        )),
        ..Default::default()
    }
    .encode_to_vec();
    assert!(zed.sync.response(envelope.id, &encoded));
}

#[tokio::test]
async fn a_reservation_refuses_only_opens_that_could_alias_it() {
    use wire::cowboy_open_target_response::State as Target;
    for (target, buffer_id, admitted) in [
        (Target::NotOpen, 0, true),
        (Target::Open, 5, true),
        (Target::Open, 8, false),
        (Target::Loading, 0, false),
    ] {
        let f = Fixture::new().await;
        let (sync, mut private, _closed) = crate::native_close::tests::fixture();
        let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
        let held = f.prepare().await;
        let task = spawn(&f, &zed, Request::OpenBufferLease { lease: held });
        ordinary_open(&zed, &mut outbound, 8).await;
        assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
        // Native buffer 8 is now closing: its ID is reserved.
        f.buffers
            .active
            .write()
            .await
            .values_mut()
            .next()
            .unwrap()
            .closing = true;
        let next = prepare_other(&f).await;
        let task = spawn(
            &f,
            &zed,
            Request::OpenBufferLease {
                lease: next.clone(),
            },
        );
        answer_target(&zed, &mut private, target, buffer_id).await;
        if admitted {
            // Checked again under the active-map write lock before dispatch.
            answer_target(&zed, &mut private, target, buffer_id).await;
            ordinary_open(&zed, &mut outbound, 9).await;
            assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
        } else {
            let error = task.await.unwrap().unwrap_err();
            assert!(
                error.to_string().contains("reserved native buffer"),
                "{error:#}"
            );
            assert!(outbound.try_recv().is_err(), "a refused open dispatched");
            assert_eq!(
                state(
                    f.request(Request::QueryBufferLease { lease: next })
                        .await
                        .unwrap()
                ),
                LeaseState::Prepared
            );
        }
    }
}

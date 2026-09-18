use super::*;
use crate::native_open::tests::{ack, opened, reply, share};
use crate::{Zed, coordinate_queries};

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
        assert_unknown(&f, &zed, &lease).await;
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
        assert_unknown(&f, &zed, &lease).await;
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
    assert!(
        request(&f, &zed, Request::OpenBufferLease { lease: prepared })
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
    assert!(matches!(
        outbound.recv().await.unwrap().payload,
        Some(proto::envelope::Payload::CloseBuffer(_))
    ));
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

use super::*;
use crate::buffer_leases::{LeaseRef, LeaseState};

async fn prepare(f: &Fixture, navigation: &NavigationRef) -> LeaseRef {
    let Response::BufferLease {
        lease,
        state: LeaseState::Prepared,
        ..
    } = f
        .request(Request::PrepareNavigationBuffer {
            navigation: navigation.clone(),
            destination: 0,
            content: f.content(8),
        })
        .await
        .unwrap()
    else {
        panic!("not prepared")
    };
    lease
}

fn lease_state(response: Response) -> LeaseState {
    let Response::BufferLease { lease, state, .. } = response else {
        panic!("not a lease")
    };
    drop(lease);
    state
}

async fn fixture() -> (Fixture, NavigationRef) {
    let mut f = Fixture::new().await;
    f.target(8, "target").await;
    let nav = f.prepare().await;
    f.execute(&nav, &[8]).await.unwrap();
    (f, nav)
}

#[tokio::test]
async fn unknown_native_open_fences_both_handoff_stages_without_erasing_the_group() {
    let (mut f, nav) = fixture().await;
    let child = prepare(&f, &nav).await;
    std::fs::write(f.root.join("uncertain"), "uncertain\n").unwrap();
    let task = f.spawn(Request::OpenBuffer {
        worktree: f.root.clone(),
        path: "uncertain".into(),
        lease_id: "unknown-open".into(),
    });
    let opening = f.outbound.recv().await.unwrap();
    assert!(matches!(
        opening.payload,
        Some(proto::envelope::Payload::OpenBufferByPath(_))
    ));
    crate::native_open::tests::reply(&f.zed, &opening, crate::native_open::tests::ack()).await;
    assert!(task.await.unwrap().is_err());
    assert!(
        f.request(Request::PrepareNavigationBuffer {
            navigation: nav.clone(),
            destination: 0,
            content: f.content(8),
        })
        .await
        .is_err()
    );
    assert!(
        f.request(Request::OpenBufferLease {
            lease: child.clone()
        })
        .await
        .is_err()
    );
    assert_eq!(
        lease_state(
            f.request(Request::QueryBufferLease { lease: child })
                .await
                .unwrap()
        ),
        LeaseState::Prepared
    );
    assert!(matches!(
        state(f.request(action(&nav, Action::Query)).await.unwrap()),
        State::Retained { .. }
    ));
    assert!(f.outbound.try_recv().is_err());
    f.request(action(&nav, Action::Release)).await.unwrap();
    assert!(f.buffers.native_open.check().is_err());
}

#[tokio::test]
async fn independent_handoff_survives_group_release_and_can_navigate_again() {
    let (mut f, nav) = fixture().await;
    let first = prepare(&f, &nav).await;
    let second = prepare(&f, &nav).await;
    // target has never existed on disk: neither preparation nor open may look
    // it up, or issue native OpenBuffer/registration a second time.
    assert_eq!(
        f.buffers.active.read().await[&(f.root.clone(), "target".into())]
            .lease_ids
            .len(),
        1
    );
    for lease in [&first, &second, &first] {
        assert_eq!(
            lease_state(
                f.request(Request::OpenBufferLease {
                    lease: lease.clone()
                })
                .await
                .unwrap()
            ),
            LeaseState::Open
        );
    }
    assert!(f.outbound.try_recv().is_err());
    f.request(action(&nav, Action::Release)).await.unwrap();
    f.request(Request::ReleaseBufferLease {
        lease: first.clone(),
    })
    .await
    .unwrap();
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(
        f.buffers.active.read().await[&(f.root.clone(), "target".into())]
            .lease_ids
            .len(),
        1
    );
    // No source-view lifetime is borrowed after handoff. Only the original
    // target lease can prepare the next navigation, with fresh content/point.
    let Response::OwnedBufferNavigation {
        navigation: next,
        state: State::Prepared,
        ..
    } = f
        .request(Request::PrepareBufferNavigation {
            lease: second.clone(),
            content: f.content(8),
            position: Point { row: 0, column: 1 },
            kind: NavigationKind::Definition,
        })
        .await
        .unwrap()
    else {
        panic!("not prepared")
    };
    f.execute(&next, &[7]).await.unwrap();
    f.request(action(&next, Action::Release)).await.unwrap();
    f.request(Request::ReleaseBufferLease {
        lease: second.clone(),
    })
    .await
    .unwrap();
    let Some(proto::envelope::Payload::CloseBuffer(close)) =
        f.outbound.recv().await.unwrap().payload
    else {
        panic!("not close")
    };
    assert_eq!(close.buffer_id, 8);
    for lease in [first, second] {
        assert_eq!(
            lease_state(
                f.request(Request::QueryBufferLease { lease })
                    .await
                    .unwrap()
            ),
            LeaseState::Released
        );
    }
    assert!(f.outbound.try_recv().is_err());
}

#[tokio::test]
async fn released_group_cannot_be_replaced_between_handoff_prepare_and_open() {
    let (mut f, nav) = fixture().await;
    let lease = prepare(&f, &nav).await;
    f.request(action(&nav, Action::Release)).await.unwrap();
    f.outbound.recv().await.unwrap(); // original target CloseBuffer
    f.target(8, "target").await;
    let replacement = f.prepare().await;
    f.execute(&replacement, &[8]).await.unwrap();
    assert!(
        f.request(Request::OpenBufferLease {
            lease: lease.clone()
        })
        .await
        .is_err()
    );
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(
        lease_state(
            f.request(Request::QueryBufferLease {
                lease: lease.clone()
            })
            .await
            .unwrap()
        ),
        LeaseState::Prepared
    );
    f.request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
    assert_eq!(
        f.buffers.active.read().await[&(f.root.clone(), "target".into())]
            .lease_ids
            .len(),
        1
    );
    f.request(action(&replacement, Action::Release))
        .await
        .unwrap();
}

#[tokio::test]
async fn handoff_rechecks_target_epoch_after_waiting_for_the_active_map() {
    let (mut f, nav) = fixture().await;
    let lease = prepare(&f, &nav).await;
    let held = f.buffers.active.write().await;
    let task = f.spawn(Request::OpenBufferLease {
        lease: lease.clone(),
    });
    wait_for_navigation_lock(&f).await;
    let before = f.content(8);
    f.aba(8);
    assert_eq!(before, f.content(8));
    drop(held);
    assert!(task.await.unwrap().is_err());
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(
        lease_state(
            f.request(Request::QueryBufferLease {
                lease: lease.clone()
            })
            .await
            .unwrap()
        ),
        LeaseState::Prepared
    );
    f.request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
    f.request(action(&nav, Action::Release)).await.unwrap();
}

async fn wait_for_navigation_lock(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while f.buffers.navigations.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelled_handoff_waiter_has_no_effect_and_keeps_its_preallocated_id() {
    let (mut f, nav) = fixture().await;
    let lease = prepare(&f, &nav).await;
    let held = f.buffers.active.write().await;
    let task = f.spawn(Request::OpenBufferLease {
        lease: lease.clone(),
    });
    wait_for_navigation_lock(&f).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(held);
    assert_eq!(
        lease_state(
            f.request(Request::QueryBufferLease {
                lease: lease.clone()
            })
            .await
            .unwrap()
        ),
        LeaseState::Prepared
    );
    assert_eq!(
        f.buffers.active.read().await[&(f.root.clone(), "target".into())]
            .lease_ids
            .len(),
        1
    );
    // There was no native dispatch or local pin change to replay.
    f.request(Request::OpenBufferLease {
        lease: lease.clone(),
    })
    .await
    .unwrap();
    f.request(action(&nav, Action::Release)).await.unwrap();
    assert!(f.outbound.try_recv().is_err());
    f.request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
}

#[tokio::test]
async fn disconnected_handoff_observer_keeps_an_independent_queryable_owner() {
    use tokio::io::AsyncWriteExt as _;
    let (mut f, nav) = fixture().await;
    let lease = prepare(&f, &nav).await;
    let held = f.buffers.active.write().await;
    let (mut client, peer) = tokio::net::UnixStream::pair().unwrap();
    let task = tokio::spawn(crate::handle(
        peer,
        f.worktrees.clone(),
        f.buffers.clone(),
        Some(f.zed.clone()),
    ));
    let mut wire = serde_json::to_vec(&Request::OpenBufferLease {
        lease: lease.clone(),
    })
    .unwrap();
    wire.push(b'\n');
    client.write_all(&wire).await.unwrap();
    wait_for_navigation_lock(&f).await;
    drop(client);
    drop(held);
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        lease_state(
            f.request(Request::QueryBufferLease {
                lease: lease.clone()
            })
            .await
            .unwrap()
        ),
        LeaseState::Open
    );
    f.request(action(&nav, Action::Release)).await.unwrap();
    assert!(f.outbound.try_recv().is_err());
    f.request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
}

#[tokio::test]
async fn handoff_capacity_is_the_same_bounded_non_recycled_buffer_registry() {
    let (mut f, nav) = fixture().await;
    let first = prepare(&f, &nav).await;
    for _ in 1..1023 {
        prepare(&f, &nav).await;
    }
    assert!(
        f.request(Request::PrepareNavigationBuffer {
            navigation: nav.clone(),
            destination: 0,
            content: f.content(8)
        })
        .await
        .is_err()
    );
    assert!(f.outbound.try_recv().is_err());
    f.request(Request::ReleaseBufferLease {
        lease: first.clone(),
    })
    .await
    .unwrap();
    assert_ne!(prepare(&f, &nav).await, first);
    assert!(
        f.request(Request::OpenBufferLease { lease: first })
            .await
            .is_err()
    );
    f.request(action(&nav, Action::Release)).await.unwrap();
}

#[tokio::test]
async fn handoff_requires_the_original_result_and_exact_content() {
    let (mut f, nav) = fixture().await;
    let mut changed = f.content(8);
    changed.utf8_bytes += 1;
    for (destination, content) in [(0, changed), (1, f.content(8))] {
        assert!(
            f.request(Request::PrepareNavigationBuffer {
                navigation: nav.clone(),
                destination,
                content
            })
            .await
            .is_err()
        );
    }
    let pending = f.prepare().await;
    assert!(
        f.request(Request::PrepareNavigationBuffer {
            navigation: pending.clone(),
            destination: 0,
            content: f.content(8)
        })
        .await
        .is_err()
    );
    assert!(f.outbound.try_recv().is_err());
    f.request(action(&pending, Action::Release)).await.unwrap();
    f.request(action(&nav, Action::Release)).await.unwrap();
    assert!(
        serde_json::from_value::<Request>(serde_json::json!({
            "type":"prepareNavigationBuffer", "navigation":nav, "destination":0,
            "content":{"sha256":"a".repeat(64),"utf8Bytes":0}, "path":"replacement"
        }))
        .is_err()
    );
}

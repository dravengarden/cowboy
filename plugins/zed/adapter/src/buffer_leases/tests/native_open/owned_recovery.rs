use super::*;

async fn abandon(f: &Fixture, zed: &Zed, outbound: &mut Outbound, lease: &LeaseRef) -> u32 {
    let task = spawn(
        f,
        zed,
        Request::OpenBufferLease {
            lease: lease.clone(),
        },
    );
    let opening = outbound.recv().await.unwrap();
    assert!(matches!(
        opening.payload,
        Some(proto::envelope::Payload::OpenBufferByPath(_))
    ));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    opening.id
}

#[tokio::test]
async fn query_observes_original_open_without_replaying_or_starting_cleanup() {
    for outcome in [
        Outcome::Failed,
        Outcome::Unknown,
        Outcome::Pending,
        Outcome::Opened,
    ] {
        let f = Fixture::new().await;
        let (sync, mut private, mut closed) = crate::native_close::tests::fixture();
        let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
        let lease = f.prepare().await;
        let original = abandon(&f, &zed, &mut outbound, &lease).await;
        let task = spawn(
            &f,
            &zed,
            Request::QueryBufferLease {
                lease: lease.clone(),
            },
        );
        barrier(&zed, &mut outbound).await;
        answer_attempt(&zed, &mut private, original, outcome, 8).await;
        let released = matches!(outcome, Outcome::Failed | Outcome::Unknown);
        assert_eq!(
            state(task.await.unwrap().unwrap()),
            if released {
                LeaseState::Released
            } else {
                LeaseState::Unknown
            }
        );
        assert_eq!(f.buffers.native_open.check().is_ok(), released);
        assert!(
            closed.try_recv().is_err(),
            "Query must not start orphan cleanup"
        );
        assert!(private.try_recv().is_err());
        assert!(outbound.try_recv().is_err(), "Query must not replay Open");
    }
}

#[tokio::test]
async fn recovered_lease_cannot_query_or_clear_a_later_open_attempt() {
    let f = Fixture::new().await;
    let (sync, mut private, mut closed) = crate::native_close::tests::fixture();
    let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
    let first = f.prepare().await;
    let original = abandon(&f, &zed, &mut outbound, &first).await;
    let next = prepare_other(&f).await;
    let task = spawn(
        &f,
        &zed,
        Request::OpenBufferLease {
            lease: next.clone(),
        },
    );
    barrier(&zed, &mut outbound).await;
    answer_attempt(&zed, &mut private, original, Outcome::Failed, 0).await;
    let later = outbound.recv().await.unwrap();
    assert!(matches!(
        later.payload,
        Some(proto::envelope::Payload::OpenBufferByPath(_))
    ));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        state(
            request(&f, &zed, Request::QueryBufferLease { lease: first })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
    assert!(
        f.buffers.native_open.check().is_err(),
        "later attempt is still unresolved"
    );
    assert!(private.try_recv().is_err());
    assert!(outbound.try_recv().is_err());
    let task = spawn(&f, &zed, Request::QueryBufferLease { lease: next });
    barrier(&zed, &mut outbound).await;
    answer_attempt(&zed, &mut private, later.id, Outcome::Pending, 0).await;
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Unknown);
    assert!(closed.try_recv().is_err());
}

#[tokio::test]
async fn confirmed_orphan_cleanup_retires_only_its_abandoned_lease() {
    let f = Fixture::new().await;
    let (sync, mut private, mut closed) = crate::native_close::tests::fixture();
    let (zed, mut outbound) = coordinate_queries::fixture_with(sync).await;
    let first = f.prepare().await;
    let original = abandon(&f, &zed, &mut outbound, &first).await;
    let next = prepare_other(&f).await;
    let task = spawn(
        &f,
        &zed,
        Request::OpenBufferLease {
            lease: next.clone(),
        },
    );
    barrier(&zed, &mut outbound).await;
    answer_attempt(&zed, &mut private, original, Outcome::Opened, 8).await;
    assert_eq!(closed.recv().await.unwrap(), vec![8]);
    ordinary_open(&zed, &mut outbound, 9).await;
    assert_eq!(state(task.await.unwrap().unwrap()), LeaseState::Open);
    for _ in 0..2 {
        assert_eq!(
            state(
                request(
                    &f,
                    &zed,
                    Request::QueryBufferLease {
                        lease: first.clone()
                    }
                )
                .await
                .unwrap()
            ),
            LeaseState::Released
        );
        assert_eq!(
            state(
                request(
                    &f,
                    &zed,
                    Request::QueryBufferLease {
                        lease: next.clone()
                    }
                )
                .await
                .unwrap()
            ),
            LeaseState::Open
        );
    }
    assert!(closed.try_recv().is_err());
    assert!(outbound.try_recv().is_err());
    let active = f.buffers.active.read().await;
    assert_eq!(active.len(), 1);
    assert_eq!(active.values().next().unwrap().remote_id, 9);
}

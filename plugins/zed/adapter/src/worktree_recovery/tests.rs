use super::*;
use prost::Message;

#[test]
fn original_kind_and_target_are_required_and_absence_never_proves_no_effect() {
    let op = Operation::new(Kind::Remove, Some(77)).unwrap();
    let reply = |kind, outcome, worktree_id| wire::CowboyWorktreeAttemptResponse {
        protocol: 1,
        kind: kind as i32,
        outcome: outcome as i32,
        worktree_id,
    };
    assert_eq!(
        op.decode(&reply(Kind::Remove, Outcome::Completed, 77))
            .unwrap(),
        Observation::Completed(77)
    );
    assert!(
        op.decode(&reply(Kind::Add, Outcome::Completed, 77))
            .is_err()
    );
    assert!(
        op.decode(&reply(Kind::Remove, Outcome::Completed, 78))
            .is_err()
    );
    assert!(
        op.decode(&reply(Kind::Remove, Outcome::Completed, 0))
            .is_err()
    );
    assert_eq!(
        op.decode(&reply(Kind::Unspecified, Outcome::Unknown, 0))
            .unwrap(),
        Observation::Unknown
    );
    assert_eq!(
        op.decode(&reply(Kind::Unspecified, Outcome::Forgotten, 0))
            .unwrap(),
        Observation::Unknown
    );
    assert_eq!(
        op.decode(&reply(Kind::Remove, Outcome::Uncertain, 0))
            .unwrap(),
        Observation::Unknown
    );
    assert!(
        op.decode(&reply(Kind::Unspecified, Outcome::Supported, 0))
            .is_err()
    );
    assert!(
        op.decode(&reply(Kind::Remove, Outcome::Unknown, 77))
            .is_err()
    );
    assert!(Operation::new(Kind::Remove, None).is_err());
    assert!(Operation::new(Kind::Add, Some(77)).is_err());
}

#[tokio::test]
async fn interrupted_add_only_queries_original_registration_and_never_replays() {
    let (transport, mut private) = crate::sync_native::Transport::new();
    let (zed, mut outbound) = crate::coordinate_queries::fixture_with(transport).await;
    let mut op = Operation::new(Kind::Add, None).unwrap();
    let add = || {
        proto::envelope::Payload::AddWorktree(proto::AddWorktree {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            path: "/workspace".into(),
            visible: true,
        })
    };
    let original = {
        let effect = op.execute(&zed, add());
        tokio::pin!(effect);
        tokio::select! {
            _ = &mut effect => panic!("reply was withheld"),
            request = outbound.recv() => request.unwrap().id,
        }
    };
    assert!(op.execute(&zed, add()).await.is_err());
    assert!(outbound.try_recv().is_err());
    let (replacement, mut foreign) = crate::coordinate_queries::fixture().await;
    assert!(op.observe(&replacement).await.is_err());
    assert!(foreign.try_recv().is_err());
    for (outcome, expected) in [
        (Outcome::Pending, Observation::Pending),
        (Outcome::Completed, Observation::Completed(77)),
    ] {
        let (result, ()) = tokio::join!(op.observe(&zed), async {
            let ping = outbound.recv().await.unwrap();
            assert!(matches!(
                ping.payload,
                Some(proto::envelope::Payload::Ping(_))
            ));
            crate::native_open::tests::reply(
                &zed,
                &ping,
                proto::envelope::Payload::Ack(proto::Ack {}),
            )
            .await;
            let request = private.recv().await.unwrap();
            assert!(
                matches!(&request.payload, Some(Payload::WorktreeAttemptRequest(q)) if q.message_id == original)
            );
            let encoded = wire::CowboyBufferSyncEnvelope {
                responding_to: Some(request.id),
                payload: Some(Payload::WorktreeAttemptResponse(
                    wire::CowboyWorktreeAttemptResponse {
                        protocol: 1,
                        kind: Kind::Add as i32,
                        outcome: outcome as i32,
                        worktree_id: if outcome == Outcome::Completed { 77 } else { 0 },
                    },
                )),
                ..Default::default()
            }
            .encode_to_vec();
            assert!(zed.sync.response(request.id, &encoded));
        });
        assert_eq!(result.unwrap(), expected);
        assert!(outbound.try_recv().is_err());
    }
}

#[tokio::test]
async fn support_requires_its_own_explicit_native_reply() {
    for outcome in [Outcome::Unknown, Outcome::Supported] {
        let (transport, mut private) = crate::sync_native::Transport::new();
        let (zed, mut outbound) = crate::coordinate_queries::fixture_with(transport).await;
        let (result, ()) = tokio::join!(support(&zed), async {
            let request = private.recv().await.unwrap();
            assert!(
                matches!(&request.payload, Some(Payload::WorktreeAttemptRequest(q)) if q.message_id == 0)
            );
            let encoded = wire::CowboyBufferSyncEnvelope {
                responding_to: Some(request.id),
                payload: Some(Payload::WorktreeAttemptResponse(
                    wire::CowboyWorktreeAttemptResponse {
                        protocol: 1,
                        kind: Kind::Unspecified as i32,
                        outcome: outcome as i32,
                        worktree_id: 0,
                    },
                )),
                ..Default::default()
            }
            .encode_to_vec();
            assert!(zed.sync.response(request.id, &encoded));
        });
        assert_eq!(result.is_ok(), outcome == Outcome::Supported);
        assert!(outbound.try_recv().is_err());
    }
}

use super::*;

#[tokio::test]
async fn typed_native_refusal_never_becomes_empty_success_or_clears_unknown() {
    use wire::cowboy_buffer_sync_envelope::Payload;
    use wire::cowboy_navigation_response::{Outcome, Refusal};
    for reason in [
        Refusal::Budget,
        Refusal::Source,
        Refusal::Target,
        Refusal::LanguageServer,
        Refusal::Deadline,
    ] {
        let mut f = Fixture::new().await;
        let nav = f.prepare().await;
        let task = f.spawn(action(&nav, Action::Execute));
        let request = f.navigation.recv().await.unwrap();
        let reply = wire::CowboyBufferSyncEnvelope {
            responding_to: Some(request.id),
            payload: Some(Payload::NavigationResponse(
                wire::CowboyNavigationResponse {
                    protocol: 1,
                    outcome: Outcome::Refused as i32,
                    refusal: reason as i32,
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        assert!(f.zed.sync.response(request.id, &reply.encode_to_vec()));
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<crate::navigation_native::NativeRefusal>()
                .unwrap()
                .0,
            reason
        );
        for action_kind in [Action::Query, Action::Execute, Action::Release] {
            assert!(matches!(
                state(f.request(action(&nav, action_kind)).await.unwrap()),
                State::Unknown
            ));
        }
        assert!(f.navigation.try_recv().is_err());
        assert!(f.outbound.try_recv().is_err());
        let active = f.buffers.active.read().await;
        assert!(crate::sync_owners::ensure_admission(&f.buffers, &active).is_err());
    }
}

#[tokio::test]
async fn navigation_reply_can_precede_the_original_target_state_and_last_chunk() {
    let mut f = Fixture::new().await;
    let nav = f.prepare().await;
    let task = f.spawn(action(&nav, Action::Execute));
    let request = f.navigation.recv().await.unwrap();
    f.reply_navigation(request, definitions(&[8]));
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    assert!(f.outbound.try_recv().is_err());
    f.zed.buffer_files.write().await.insert(
        8,
        proto::File {
            worktree_id: 1,
            path: "target".into(),
            ..Default::default()
        },
    );
    // Leave a valid native base pending, then separately deliver its last chunk.
    f.zed
        .diagnostics
        .lock()
        .unwrap()
        .observe(&proto::envelope::Payload::CreateBufferForPeer(
            proto::CreateBufferForPeer {
                variant: Some(proto::create_buffer_for_peer::Variant::State(
                    proto::BufferState {
                        id: 8,
                        base_text: "a🙂z\n".into(),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
        ));
    f.zed.events.send(proto::Envelope::default()).unwrap();
    tokio::task::yield_now().await;
    assert!(!task.is_finished());
    assert!(f.outbound.try_recv().is_err());
    let chunk = proto::envelope::Payload::CreateBufferForPeer(proto::CreateBufferForPeer {
        variant: Some(proto::create_buffer_for_peer::Variant::Chunk(
            proto::BufferChunk {
                buffer_id: 8,
                is_last: true,
                ..Default::default()
            },
        )),
        ..Default::default()
    });
    f.zed.diagnostics.lock().unwrap().observe(&chunk);
    f.zed
        .events
        .send(proto::Envelope {
            payload: Some(chunk),
            ..Default::default()
        })
        .unwrap();
    ack_registration(&f.zed, f.outbound.recv().await.unwrap()).await;
    assert!(matches!(
        state(task.await.unwrap().unwrap()),
        State::Retained { .. }
    ));
    assert!(f.outbound.try_recv().is_err());
    f.request(action(&nav, Action::Release)).await.unwrap();
}

#[tokio::test]
async fn target_registration_is_one_use_and_retains_unknown_pins_on_failure() {
    for case in 0..3 {
        let mut f = Fixture::new().await;
        f.target(8, "target").await;
        let nav = f.prepare().await;
        let task = f.spawn(action(&nav, Action::Execute));
        let request = f.navigation.recv().await.unwrap();
        f.reply_navigation(request, definitions(&[8, 8]));
        let registration = f.outbound.recv().await.unwrap();
        match case {
            0 => {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            1 => {
                f.aba(8);
                ack_registration(&f.zed, registration).await;
                assert!(task.await.unwrap().is_err());
            }
            _ => {
                f.zed
                    .pending
                    .lock()
                    .await
                    .remove(&registration.id)
                    .unwrap()
                    .send(proto::Envelope {
                        payload: Some(proto::envelope::Payload::Error(proto::Error::default())),
                        ..Default::default()
                    })
                    .unwrap();
                assert!(task.await.unwrap().is_err());
            }
        }
        for action_kind in [Action::Query, Action::Execute, Action::Release] {
            assert!(matches!(
                state(f.request(action(&nav, action_kind)).await.unwrap()),
                State::Unknown
            ));
        }
        let registry = f.buffers.navigations.lock().await;
        assert_eq!(registry.slots[&1].targets.len(), 2);
        assert_eq!(registry.slots[&1].targets[0].remote_id, 8);
        let active = f.buffers.active.read().await;
        assert!(crate::sync_owners::ensure_admission(&f.buffers, &active).is_err());
        assert!(
            active[&(f.root.clone(), "target".into())]
                .lease_ids
                .contains(&BufferOwner::Navigation(1))
        );
        assert!(f.outbound.try_recv().is_err());
    }
}

#[tokio::test]
async fn late_admission_rechecks_preparation_deadline_before_native_dispatch() {
    let mut f = Fixture::new().await;
    f.prepare().await;
    let mut registry = f.buffers.navigations.lock().await;
    let slot = registry.slots.get_mut(&1).unwrap();
    slot.prepared_at -= PREPARE_TTL * 2;
    assert!(execute(slot, 1, &f.buffers, &f.zed).await.is_err());
    assert!(matches!(slot.phase, Phase::Prepared));
    assert!(f.outbound.try_recv().is_err());
    assert_eq!(
        f.buffers
            .active
            .read()
            .await
            .values()
            .next()
            .unwrap()
            .lease_ids
            .len(),
        1
    );
}

#[tokio::test]
async fn target_edit_undo_during_read_cannot_deliver_stale_positions() {
    let mut f = Fixture::new().await;
    f.target(8, "target").await;
    let nav = f.prepare().await;
    f.execute(&nav, &[8]).await.unwrap();
    let task = f.spawn(Request::ReadBufferNavigation {
        navigation: nav.clone(),
        destination: 0,
        content: f.content(8),
        query: Query::Symbols {},
    });
    let native = f.outbound.recv().await.unwrap();
    f.aba(8);
    coordinate_queries::reply(&f.zed, native, Vec::new()).await;
    assert!(task.await.unwrap().is_err());
    assert!(matches!(
        state(f.request(action(&nav, Action::Query)).await.unwrap()),
        State::Retained { .. }
    ));
    assert!(f.outbound.try_recv().is_err());
    f.request(action(&nav, Action::Release)).await.unwrap();
}

#[tokio::test]
async fn cancellation_is_unknown_and_fences_new_native_admission_without_replay() {
    let mut f = Fixture::new().await;
    let nav = f.prepare().await;
    let task = f.spawn(action(&nav, Action::Execute));
    f.navigation.recv().await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    f.buffers
        .navigations
        .lock()
        .await
        .slots
        .get_mut(&1)
        .unwrap()
        .prepared_at -= PREPARE_TTL * 2;
    for action_kind in [Action::Query, Action::Execute, Action::Release] {
        assert!(matches!(
            state(f.request(action(&nav, action_kind)).await.unwrap()),
            State::Unknown
        ));
    }
    assert!(f.outbound.try_recv().is_err());
    assert!(
        crate::sync_owners::ensure_admission(&f.buffers, &*f.buffers.active.read().await).is_err()
    );
    f.request(Request::ReleaseBufferLease {
        lease: f.lease.clone(),
    })
    .await
    .unwrap();
    assert!(f.outbound.try_recv().is_err());
    assert!(
        crate::sync_owners::ensure_admission(&f.buffers, &*f.buffers.active.read().await).is_err()
    );
}

#[tokio::test]
async fn changed_source_during_native_query_retains_unknown_not_empty_success() {
    let mut f = Fixture::new().await;
    f.target(8, "target").await;
    let nav = f.prepare().await;
    let task = f.spawn(action(&nav, Action::Execute));
    let request = f.navigation.recv().await.unwrap();
    f.aba(7);
    f.reply_navigation(request, definitions(&[8]));
    assert!(task.await.unwrap().is_err());
    assert!(matches!(
        state(f.request(action(&nav, Action::Query)).await.unwrap()),
        State::Unknown
    ));
    assert_eq!(f.buffers.active.read().await.len(), 1);
    assert!(
        crate::sync_owners::ensure_admission(&f.buffers, &*f.buffers.active.read().await).is_err()
    );
}

#[tokio::test]
async fn invalid_or_excessive_native_destinations_never_publish_partial_success() {
    for case in 0..7 {
        let mut f = Fixture::new().await;
        f.target(8, "target").await;
        let nav = f.prepare().await;
        if case == 6 {
            f.buffers.active.write().await.insert(
                (f.root.clone(), "target".into()),
                crate::BufferLease {
                    lease_ids: [BufferOwner::Owned(2)].into(),
                    remote_id: 999,
                    version: vec![],
                    sync: None,
                    closing: false,
                },
            );
        }
        let task = f.spawn(action(&nav, Action::Execute));
        let request = f.navigation.recv().await.unwrap();
        let mut responses = definitions(&[8]);
        match case {
            0 => f.zed.buffer_files.write().await.get_mut(&8).unwrap().path = "../outside".into(),
            1 => {
                f.zed
                    .buffer_files
                    .write()
                    .await
                    .get_mut(&8)
                    .unwrap()
                    .worktree_id = 2;
            }
            2 => responses = definitions(&vec![8; MAX_DESTINATIONS + 1]),
            3 => responses[0].response = None,
            4 => responses = definitions(&[8, 999]),
            5 => {
                let ids: Vec<_> = (8..41).collect();
                for id in &ids {
                    f.target(*id, &format!("target-{id}")).await;
                }
                responses = definitions(&ids);
            }
            _ => {}
        }
        f.reply_navigation(request, responses);
        assert!(task.await.unwrap().is_err(), "case {case}");
        assert!(matches!(
            state(f.request(action(&nav, Action::Query)).await.unwrap()),
            State::Unknown
        ));
        assert!(
            f.buffers
                .active
                .read()
                .await
                .values()
                .all(|buffer| !buffer.lease_ids.contains(&BufferOwner::Navigation(1)))
        );
        assert!(f.outbound.try_recv().is_err());
    }
}

#[tokio::test]
async fn capacity_expiry_and_disjoint_ids_cannot_recover_or_replay_native_effects() {
    let f = Fixture::new().await;
    let first = f.prepare().await;
    for _ in 1..MAX_NAVIGATIONS {
        f.prepare().await;
    }
    assert!(
        f.request(Request::PrepareBufferNavigation {
            lease: f.lease.clone(),
            content: f.content(7),
            position: Point { row: 0, column: 1 },
            kind: NavigationKind::Definition,
        })
        .await
        .is_err()
    );
    for id in [
        "0000000000000001",
        "nav:0000000000000000",
        "nav:000000000000FFFF",
        "nav:000000000000ffff",
        "nav:1",
    ] {
        assert!(
            f.request(action(
                &NavigationRef {
                    id: id.into(),
                    ..first.clone()
                },
                Action::Query
            ))
            .await
            .is_err()
        );
    }
    assert!(
        f.request(action(
            &NavigationRef {
                instance: "f".repeat(32),
                ..first.clone()
            },
            Action::Query
        ))
        .await
        .is_err()
    );
    let mut wire = serde_json::to_value(&f.lease).unwrap();
    wire["id"] = serde_json::json!(first.id);
    let fake_lease: buffer_leases::LeaseRef = serde_json::from_value(wire).unwrap();
    assert!(
        f.request(Request::QueryBufferLease { lease: fake_lease })
            .await
            .is_err()
    );
    {
        let mut registry = f.buffers.navigations.lock().await;
        for slot in registry.slots.values_mut() {
            slot.prepared_at -= PREPARE_TTL * 2;
        }
    }
    assert!(matches!(
        state(f.request(action(&first, Action::Execute)).await.unwrap()),
        State::Released
    ));
    assert_eq!(f.prepare().await.id, "nav:0000000000000021");
}

#[tokio::test]
async fn native_close_refusal_keeps_release_uncertainty_without_partial_local_removal() {
    let mut f = Fixture::new().await;
    f.target(8, "target").await;
    let nav = f.prepare().await;
    f.execute(&nav, &[8]).await.unwrap();
    let (transport, mut receiver) = crate::sync_native::Transport::new();
    Arc::get_mut(&mut f.zed).unwrap().sync = transport;
    let task = f.spawn(action(&nav, Action::Release));
    crate::native_close::tests::reply(
        &f.zed.sync,
        receiver.recv().await.unwrap(),
        wire::cowboy_close_buffers_response::Outcome::Supported,
    );
    assert_eq!(
        crate::native_close::tests::reply(
            &f.zed.sync,
            receiver.recv().await.unwrap(),
            wire::cowboy_close_buffers_response::Outcome::Refused
        ),
        [8]
    );
    assert!(task.await.unwrap().is_err());
    release_unknown(&f, &nav, &["target"]).await;
    assert!(receiver.try_recv().is_err());
    let registry = f.buffers.navigations.lock().await;
    let saved = &registry.slots[&1].targets;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].remote_id, 8);
    assert_eq!(saved[0].key, (f.root.clone(), PathBuf::from("target")));
}

async fn release_unknown(f: &Fixture, nav: &NavigationRef, targets: &[&str]) {
    for action_kind in [Action::Release, Action::Execute, Action::Query] {
        assert!(matches!(
            state(f.request(action(nav, action_kind)).await.unwrap()),
            State::ReleaseUnknown
        ));
    }
    let active = f.buffers.active.read().await;
    let source = &active[&(f.root.clone(), "source".into())];
    assert!(source.lease_ids.contains(&BufferOwner::Navigation(1)));
    assert!(!source.closing);
    for target in targets {
        let buffer = &active[&(f.root.clone(), (*target).into())];
        assert!(buffer.lease_ids.contains(&BufferOwner::Navigation(1)));
        assert!(buffer.closing);
        assert!(crate::sync_owners::ensure_readable(buffer).is_err());
    }
    assert!(crate::sync_owners::ensure_admission(&f.buffers, &active).is_err());
}

#[tokio::test]
async fn cancelled_group_close_keeps_all_original_pins_and_late_reply_cannot_finish_it() {
    use wire::cowboy_buffer_sync_envelope::Payload;
    use wire::cowboy_close_buffers_response::Outcome;
    let mut f = Fixture::new().await;
    f.target(8, "first").await;
    f.target(9, "second").await;
    let nav = f.prepare().await;
    f.execute(&nav, &[8, 9, 9]).await.unwrap();
    let (transport, mut receiver) = crate::sync_native::Transport::new();
    Arc::get_mut(&mut f.zed).unwrap().sync = transport;
    let task = f.spawn(action(&nav, Action::Release));
    crate::native_close::tests::reply(
        &f.zed.sync,
        receiver.recv().await.unwrap(),
        Outcome::Supported,
    );
    let effect = receiver.recv().await.unwrap();
    let Some(Payload::CloseRequest(request)) = effect.payload else {
        panic!("not close")
    };
    assert_eq!(request.buffer_ids, [8, 9]);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let late = wire::CowboyBufferSyncEnvelope {
        responding_to: Some(effect.id),
        payload: Some(Payload::CloseResponse(wire::CowboyCloseBuffersResponse {
            protocol: 1,
            instance: request.instance,
            outcome: Outcome::Closed as i32,
            buffer_ids: request.buffer_ids,
        })),
        ..Default::default()
    };
    assert!(!f.zed.sync.response(effect.id, &late.encode_to_vec()));
    release_unknown(&f, &nav, &["first", "second"]).await;
    assert!(receiver.try_recv().is_err());
    assert!(f.outbound.try_recv().is_err());
}

#[test]
fn navigation_requests_are_closed_and_cannot_disguise_paths_or_effects_as_reads() {
    use serde_json::json;
    for request in [
        json!({"type":"bufferNavigation", "navigation":{"instance":"a", "id":"b"}, "action":"replay"}),
        json!({"type":"bufferNavigation", "navigation":{"instance":"a", "id":"b"}, "action":"execute", "worktree":"/tmp"}),
        json!({"kind":"navigation", "position":{"row":0,"column":0}}),
    ] {
        assert!(serde_json::from_value::<Request>(request).is_err());
    }
    assert!(serde_json::from_value::<Query>(json!({"kind":"navigate"})).is_err());
}

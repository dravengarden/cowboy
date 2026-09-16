use super::*;

fn base(cache: &mut Cache, text: &str) {
    cache.observe(&proto::envelope::Payload::CreateBufferForPeer(
        proto::CreateBufferForPeer {
            variant: Some(proto::create_buffer_for_peer::Variant::State(
                proto::BufferState {
                    id: 7,
                    base_text: text.into(),
                    ..proto::BufferState::default()
                },
            )),
            ..proto::CreateBufferForPeer::default()
        },
    ));
    cache.observe(&proto::envelope::Payload::CreateBufferForPeer(
        proto::CreateBufferForPeer {
            variant: Some(proto::create_buffer_for_peer::Variant::Chunk(
                proto::BufferChunk {
                    buffer_id: 7,
                    is_last: true,
                    ..Default::default()
                },
            )),
            ..Default::default()
        },
    ));
}

fn update(cache: &mut Cache, server: u64, timestamp: u32, diagnostics: Vec<proto::Diagnostic>) {
    cache.observe(&proto::envelope::Payload::UpdateBuffer(
        proto::UpdateBuffer {
            buffer_id: 7,
            operations: vec![proto::Operation {
                variant: Some(proto::operation::Variant::UpdateDiagnostics(
                    proto::UpdateDiagnostics {
                        server_id: server,
                        lamport_timestamp: timestamp,
                        diagnostics,
                        ..proto::UpdateDiagnostics::default()
                    },
                )),
            }],
            ..proto::UpdateBuffer::default()
        },
    ));
}

fn diagnostic() -> proto::Diagnostic {
    proto::Diagnostic {
        start: Some(crate::position_anchor(7, 1)),
        end: Some(crate::position_anchor(7, 5)),
        message: "fixture diagnostic".into(),
        severity: 1,
        ..proto::Diagnostic::default()
    }
}

#[test]
fn native_events_distinguish_unobserved_empty_and_observed_diagnostics() {
    let mut cache = Cache::default();
    base(&mut cache, "a🙂z\n");
    let revision = cache.revision(7).unwrap();
    assert!(matches!(
        cache.read(7, revision).unwrap().0,
        Status::Unobserved
    ));
    update(&mut cache, 1, 10, vec![diagnostic()]);
    let (state, diagnostics) = cache.read(7, revision).unwrap();
    assert!(matches!(state, Status::Observed));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].start.column, 1);
    assert_eq!(diagnostics[0].end.column, 3);
    update(&mut cache, 1, 9, vec![]);
    assert_eq!(
        cache.read(7, revision).unwrap().1.len(),
        1,
        "stale update erased newer diagnostics"
    );
    update(&mut cache, 2, 11, vec![diagnostic()]);
    assert_eq!(cache.read(7, revision).unwrap().1.len(), 2);
    update(&mut cache, 1, 12, vec![]);
    update(&mut cache, 2, 12, vec![]);
    let (state, diagnostics) = cache.read(7, revision).unwrap();
    assert!(matches!(state, Status::Observed));
    assert!(diagnostics.is_empty());
    assert_eq!(cache.diagnostic_bytes, 0);
}

#[test]
fn edits_and_state_replacement_cannot_reuse_old_anchor_coordinates() {
    let mut cache = Cache::default();
    base(&mut cache, "a🙂z\n");
    let revision = cache.revision(7).unwrap();
    update(&mut cache, 1, 1, vec![diagnostic()]);
    cache.observe(&proto::envelope::Payload::UpdateBuffer(
        proto::UpdateBuffer {
            buffer_id: 7,
            operations: vec![proto::Operation {
                variant: Some(proto::operation::Variant::Edit(
                    proto::operation::Edit::default(),
                )),
            }],
            ..proto::UpdateBuffer::default()
        },
    ));
    assert!(cache.read(7, revision).is_err());
    assert!(
        cache
            .anchor_offset(7, &crate::position_anchor(7, 1))
            .is_err()
    );
    assert_eq!((cache.text_bytes, cache.diagnostic_bytes), (0, 0));
    base(&mut cache, "new base");
    assert!(cache.read(7, revision).is_err());
    assert!(cache.read(7, cache.revision(7).unwrap()).is_ok());
    cache.remove(7);
    assert!(cache.revision(7).is_err());
    assert_eq!((cache.text_bytes, cache.diagnostic_bytes), (0, 0));
}

#[test]
fn missing_foreign_nonbase_and_mid_character_anchors_fail_closed() {
    for anchor in [
        None,
        Some(crate::position_anchor(8, 1)),
        Some(crate::position_anchor(7, 2)),
        Some(proto::Anchor {
            timestamp: 2,
            ..crate::position_anchor(7, 1)
        }),
    ] {
        let mut cache = Cache::default();
        base(&mut cache, "a🙂z\n");
        let revision = cache.revision(7).unwrap();
        let mut value = diagnostic();
        value.start = anchor;
        update(&mut cache, 1, 1, vec![value]);
        assert!(cache.read(7, revision).is_err());
        assert!(cache.diagnostic_bytes <= "fixture diagnostic".len());
    }
}

#[test]
fn text_diagnostic_and_server_budgets_are_not_unbounded_caches() {
    let mut cache = Cache::default();
    base(&mut cache, &"a".repeat(MAX_TEXT + 1));
    assert!(cache.revision(7).is_err());
    assert_eq!(cache.text_bytes, 0);
    base(&mut cache, "abcdef");
    let revision = cache.revision(7).unwrap();
    update(&mut cache, 1, 1, vec![diagnostic(); MAX_DIAGNOSTICS + 1]);
    assert!(cache.read(7, revision).is_err());
    assert_eq!(cache.diagnostic_bytes, 0);
    base(&mut cache, "abcdef");
    for server in 0..33 {
        update(&mut cache, server, 1, vec![]);
    }
    assert!(cache.revision(7).is_err());
    base(&mut cache, "abcdef");
    let mut large = diagnostic();
    large.message = "x".repeat(MAX_DIAGNOSTIC_TEXT + 1);
    update(&mut cache, 1, 1, vec![large]);
    assert!(cache.revision(7).is_err());
}

#[test]
fn batched_coordinates_match_utf16_boundaries_including_eof() {
    let text = "a🙂\n汉z\n";
    let offsets: Vec<_> = text
        .char_indices()
        .map(|(offset, _)| u64::try_from(offset).unwrap())
        .chain(std::iter::once(u64::try_from(text.len()).unwrap()))
        .collect();
    let values: Vec<_> = offsets
        .iter()
        .map(|offset| proto::Diagnostic {
            start: Some(crate::position_anchor(7, *offset)),
            end: Some(crate::position_anchor(7, *offset)),
            ..proto::Diagnostic::default()
        })
        .collect();
    let mirror = Mirror::new(7, text).unwrap();
    let converted: Vec<_> = capture(&values)
        .unwrap()
        .iter()
        .map(|value| convert(&mirror, value).unwrap())
        .collect();
    for (entry, offset) in converted.iter().zip(offsets) {
        let expected = crate::offset_to_utf16_point(text, offset).unwrap();
        assert_eq!(
            (entry.start.row, entry.start.column),
            (expected.row, expected.column)
        );
    }
}

#[test]
fn buffer_update_ack_and_reload_invalidation_follow_the_pinned_protocol() {
    let mut cache = Cache::default();
    base(&mut cache, "abcdef");
    let revision = cache.revision(7).unwrap();
    let reload = proto::envelope::Payload::BufferReloaded(proto::BufferReloaded {
        buffer_id: 7,
        ..proto::BufferReloaded::default()
    });
    assert!(!crate::peer_request_requires_ack(&reload));
    cache.observe(&reload);
    assert!(cache.read(7, revision).is_err());
    assert_eq!(cache.text_bytes, 6);
    assert!(cache.read(7, cache.revision(7).unwrap()).is_ok());
    assert!(crate::peer_request_requires_ack(
        &proto::envelope::Payload::UpdateBuffer(proto::UpdateBuffer::default())
    ));
    assert!(crate::peer_request_requires_ack(
        &proto::envelope::Payload::Ping(proto::Ping {})
    ));
    assert!(!crate::peer_request_requires_ack(
        &proto::envelope::Payload::LspQueryResponse(proto::LspQueryResponse::default())
    ));
}

#[test]
fn initial_share_and_reload_wait_for_history_without_reopening() {
    let mut cache = Cache::default();
    base(&mut cache, "a🙂z\n");
    cache.buffers.get_mut(&7).unwrap().shared = false;
    assert!(cache.revision(7).is_err());
    cache.buffers.get_mut(&7).unwrap().shared = true;
    let old = cache.revision(7).unwrap();
    let mut peer = crate::coordinates::tests::peer("a🙂z\n", 1);
    let edit = crate::coordinates::tests::wire(&peer.edit([(0..0, "汉\n")]));
    cache.observe(&proto::envelope::Payload::BufferReloaded(
        proto::BufferReloaded {
            buffer_id: 7,
            version: peer
                .version()
                .iter()
                .map(|entry| proto::VectorClockEntry {
                    replica_id: u32::from(entry.replica_id.as_u16()),
                    timestamp: entry.value,
                })
                .collect(),
            ..Default::default()
        },
    ));
    assert!(cache.revision(7).is_err());
    assert!(cache.buffers[&7].text.is_some());
    cache.operations(7, std::slice::from_ref(&edit));
    let new = cache.revision(7).unwrap();
    assert!(cache.read(7, old).is_err());
    cache.operations(7, &[edit]);
    assert_eq!(cache.revision(7).unwrap(), new, "duplicate advanced epoch");
    assert!(cache.read(7, new).is_ok());
}

#[test]
fn diagnostics_keep_original_anchors_but_resolve_against_edited_content() {
    let mut cache = Cache::default();
    base(&mut cache, "a🙂z\n");
    update(&mut cache, 1, 1, vec![diagnostic()]);
    let old = cache.revision(7).unwrap();
    let mut peer = crate::coordinates::tests::peer("a🙂z\n", 1);
    let operation = crate::coordinates::tests::wire(&peer.edit([(0..0, "汉\n")]));
    cache.operations(7, &[operation]);
    assert!(cache.check(7, old).is_err());
    let (_, entries) = cache.read(7, cache.revision(7).unwrap()).unwrap();
    assert_eq!(
        (
            entries[0].start.row,
            entries[0].start.column,
            entries[0].end.column
        ),
        (1, 1, 3)
    );
}

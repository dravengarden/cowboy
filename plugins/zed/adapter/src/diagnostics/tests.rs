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
        assert_eq!(cache.diagnostic_bytes, 0);
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
    let converted = convert(text, 7, &values).unwrap();
    for (entry, offset) in converted.iter().zip(offsets) {
        let expected = crate::offset_to_utf16_point(text, offset).unwrap();
        assert_eq!(
            (entry.start.row, entry.start.column),
            (expected.row, expected.column)
        );
    }
}

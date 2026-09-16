use super::*;

pub(crate) fn peer(base: &str, replica: u16) -> text::Buffer {
    text::Buffer::new(
        text::ReplicaId::new(replica),
        text::BufferId::new(7).unwrap(),
        base,
    )
}

pub(crate) fn wire(operation: &text::Operation) -> proto::Operation {
    let version = |version: &clock::Global| {
        version
            .iter()
            .map(|stamp| proto::VectorClockEntry {
                replica_id: u32::from(stamp.replica_id.as_u16()),
                timestamp: stamp.value,
            })
            .collect()
    };
    proto::Operation {
        variant: Some(match operation {
            text::Operation::Edit(edit) => {
                proto::operation::Variant::Edit(proto::operation::Edit {
                    replica_id: u32::from(edit.timestamp.replica_id.as_u16()),
                    lamport_timestamp: edit.timestamp.value,
                    version: version(&edit.version),
                    ranges: edit
                        .ranges
                        .iter()
                        .map(|range| proto::Range {
                            start: range.start.0 as u64,
                            end: range.end.0 as u64,
                        })
                        .collect(),
                    new_text: edit.new_text.iter().map(ToString::to_string).collect(),
                })
            }
            text::Operation::Undo(undo) => {
                let mut counts: Vec<_> = undo
                    .counts
                    .iter()
                    .map(|(stamp, count)| proto::UndoCount {
                        replica_id: u32::from(stamp.replica_id.as_u16()),
                        lamport_timestamp: stamp.value,
                        count: *count,
                    })
                    .collect();
                counts.sort_by_key(|count| (count.replica_id, count.lamport_timestamp));
                proto::operation::Variant::Undo(proto::operation::Undo {
                    replica_id: u32::from(undo.timestamp.replica_id.as_u16()),
                    lamport_timestamp: undo.timestamp.value,
                    version: version(&undo.version),
                    counts,
                })
            }
        }),
    }
}

fn same(mirror: &Mirror, source: &text::Buffer) {
    let content = source.text();
    assert_eq!(mirror.buffer.text(), content);
    assert_eq!(mirror.buffer.version(), source.version());
    for offset in (0..=content.len()).filter(|offset| content.is_char_boundary(*offset)) {
        let point = crate::offset_to_utf16_point(&content, offset as u64).unwrap();
        let anchor = mirror.position(point.row, point.column).unwrap();
        assert_eq!(mirror.offset(&anchor).unwrap(), offset as u64);
        let native = source.anchor_before(offset);
        assert_eq!(
            (anchor.replica_id, anchor.timestamp, anchor.offset),
            (
                u32::from(native.timestamp().replica_id.as_u16()),
                native.timestamp().value,
                u64::from(native.offset)
            )
        );
    }
}

#[test]
fn edited_inserted_deleted_and_undone_anchors_follow_native_history() {
    let base = "a🙂z\n汉字\n";
    let mut source = peer(base, 1);
    let mut mirror = Mirror::new(7, base).unwrap();
    let old = mirror.position(0, 3).unwrap();
    let first = source.edit([(1..1, "new\n")]);
    source.finalize_last_transaction();
    mirror.apply(&wire(&first), MAX_HISTORY).unwrap();
    same(&mirror, &source);
    assert_eq!(mirror.offset(&old).unwrap(), 9);
    let inserted = mirror.position(1, 0).unwrap();
    assert_ne!(inserted.timestamp, 1);
    let second = source.edit([(1..9, "界")]);
    source.finalize_last_transaction();
    mirror.apply(&wire(&second), MAX_HISTORY).unwrap();
    same(&mirror, &source);
    assert_eq!(mirror.offset(&old).unwrap(), 4);
    let undo = source.undo().unwrap().1;
    mirror.apply(&wire(&undo), MAX_HISTORY).unwrap();
    same(&mirror, &source);
    assert_eq!(mirror.offset(&old).unwrap(), 9);
    let redo = source.redo().unwrap().1;
    mirror.apply(&wire(&redo), MAX_HISTORY).unwrap();
    same(&mirror, &source);
}

#[test]
fn concurrent_edits_and_tombstones_use_the_exact_upstream_order() {
    let base = "a🙂b汉c\n";
    let mut left = peer(base, 1);
    let mut right = peer(base, 2);
    let mut first = Mirror::new(7, base).unwrap();
    let mut second = Mirror::new(7, base).unwrap();
    for index in 0..80 {
        let a = left.edit([(0..0, if index % 2 == 0 { "🙂" } else { "汉" })]);
        let end = right.text().chars().next().unwrap().len_utf8();
        let b = right.edit([(0..end, "x")]);
        left.apply_ops([b.clone()]);
        right.apply_ops([a.clone()]);
        for operation in [&a, &b] {
            first.apply(&wire(operation), MAX_HISTORY).unwrap();
        }
        for operation in [&b, &a] {
            second.apply(&wire(operation), MAX_HISTORY).unwrap();
        }
        same(&first, &left);
        same(&second, &right);
        assert_eq!(left.text(), right.text());
    }
}

#[test]
fn positions_are_exact_utf16_including_empty_and_final_empty_lines() {
    for base in ["", "\n", "a🙂\n汉z\n", "😀"] {
        same(&Mirror::new(7, base).unwrap(), &peer(base, 1));
    }
    let mirror = Mirror::new(7, "a🙂\n").unwrap();
    for (row, column) in [(0, 2), (0, 4), (1, 1), (2, 0), (u32::MAX, u32::MAX)] {
        assert!(mirror.position(row, column).is_err());
    }
    assert!(Mirror::new(0, "a").is_err());
    assert!(Mirror::new(7, "a\r\n").is_err());
}

#[test]
fn duplicates_do_not_charge_or_mutate_but_conflicting_identity_fails() {
    let mut source = peer("abc", 1);
    let mut mirror = Mirror::new(7, "abc").unwrap();
    let mut operation = wire(&source.edit([(0..1, "界")]));
    assert!(mirror.apply(&operation, MAX_HISTORY).unwrap());
    let bytes = mirror.bytes();
    assert!(!mirror.apply(&operation, 0).unwrap());
    assert_eq!(mirror.bytes(), bytes);
    let Some(proto::operation::Variant::Edit(edit)) = &mut operation.variant else {
        unreachable!()
    };
    edit.new_text[0] = "different".into();
    assert!(mirror.apply(&operation, MAX_HISTORY).is_err());
}

#[test]
fn corrupt_ranges_versions_anchors_and_budgets_fail_before_native_apply() {
    let mut source = peer("a🙂z", 1);
    let valid = wire(&source.edit([(1..5, "x")]));
    for case in 0..9 {
        let mut mirror = Mirror::new(7, "a🙂z").unwrap();
        let mut operation = valid.clone();
        let Some(proto::operation::Variant::Edit(edit)) = &mut operation.variant else {
            unreachable!()
        };
        match case {
            0 => edit.ranges[0].start = 2,
            1 => edit.ranges[0].end = 4,
            2 => edit.ranges[0].end = u64::MAX,
            3 => edit.new_text.clear(),
            4 => edit.version.push(edit.version[0].clone()),
            5 => edit.version[0].timestamp += 1,
            6 => edit.replica_id = MAX_REPLICA + 1,
            7 => edit.new_text[0] = "\r\n".into(),
            8 => edit.version.clear(),
            _ => unreachable!(),
        }
        assert!(
            mirror.apply(&operation, MAX_HISTORY).is_err(),
            "case {case}"
        );
        assert_eq!(mirror.buffer.text(), "a🙂z");
    }
    let mut mirror = Mirror::new(7, "a🙂z").unwrap();
    assert!(mirror.apply(&valid, 0).is_err());
    assert!(Mirror::new(7, &"x".repeat(MAX_HISTORY + 1)).is_err());
    let anchor = mirror.position(0, 1).unwrap();
    for bad in [
        proto::Anchor {
            buffer_id: Some(8),
            ..anchor.clone()
        },
        proto::Anchor {
            offset: 2,
            ..anchor.clone()
        },
        proto::Anchor {
            timestamp: 2,
            ..anchor.clone()
        },
        proto::Anchor {
            offset: u64::MAX,
            ..anchor.clone()
        },
        proto::Anchor { bias: 77, ..anchor },
    ] {
        assert!(mirror.offset(&bad).is_err());
    }
}

#[test]
fn tombstone_utf8_boundaries_and_missing_predecessors_fail_closed() {
    let mut source = peer("a🙂z", 1);
    let mut mirror = Mirror::new(7, "a🙂z").unwrap();
    let deletion = wire(&source.edit([(1..5, "")]));
    let mut next = wire(&source.edit([(1..1, "界")]));
    assert!(mirror.apply(&next, MAX_HISTORY).is_err());
    mirror.apply(&deletion, MAX_HISTORY).unwrap();
    let Some(proto::operation::Variant::Edit(edit)) = &mut next.variant else {
        unreachable!()
    };
    edit.ranges[0].start = 3;
    edit.ranges[0].end = 3;
    assert!(mirror.apply(&next, MAX_HISTORY).is_err());
    assert_eq!(mirror.buffer.text(), "az");
}

#[test]
fn local_server_replica_and_multiple_edit_ranges_round_trip() {
    let mut source = peer("a🙂z\n汉字\n", 0);
    let mut mirror = Mirror::new(7, &source.text()).unwrap();
    for _ in 0..30 {
        let len = source.len();
        let operation = source.edit([(0..1, "b"), (len..len, "新\n")]);
        source.finalize_last_transaction();
        mirror.apply(&wire(&operation), MAX_HISTORY).unwrap();
        same(&mirror, &source);
        let undo = source.undo().unwrap().1;
        mirror.apply(&wire(&undo), MAX_HISTORY).unwrap();
        same(&mirror, &source);
    }
}

#[test]
fn undo_cannot_target_an_unknown_operation_or_another_undo() {
    let mut source = peer("abc", 1);
    let mut mirror = Mirror::new(7, "abc").unwrap();
    mirror
        .apply(&wire(&source.edit([(0..1, "x")])), MAX_HISTORY)
        .unwrap();
    let undo = source.undo().unwrap().1;
    mirror.apply(&wire(&undo), MAX_HISTORY).unwrap();
    let redo = wire(&source.redo().unwrap().1);
    for target in [undo.timestamp().value, u32::MAX, 1] {
        let mut invalid = redo.clone();
        let Some(proto::operation::Variant::Undo(undo)) = &mut invalid.variant else {
            unreachable!()
        };
        undo.counts[0].lamport_timestamp = target;
        assert!(mirror.apply(&invalid, MAX_HISTORY).is_err());
    }
    mirror.apply(&redo, MAX_HISTORY).unwrap();
    same(&mirror, &source);
}

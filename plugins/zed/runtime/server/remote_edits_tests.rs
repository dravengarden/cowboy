// SPDX-License-Identifier: GPL-3.0-or-later
//! Real native text engines and the local BufferStore RPC admission path.
use super::*;
use gpui::TestAppContext;
use language::cowboy_replacement as budget;

fn peer(base: &str, replica: u16) -> text::Buffer {
    text::Buffer::new(ReplicaId::new(replica), BufferId::new(1).unwrap(), base)
}

fn wire(op: text::Operation) -> proto::Operation {
    language::proto::serialize_operation(&language::Operation::Buffer(op))
}

fn apply(
    buffer: &Entity<Buffer>,
    ops: Vec<proto::Operation>,
    cx: &mut TestAppContext,
) -> Result<()> {
    buffer.update(cx, |buffer, cx| buffer.cowboy_apply_remote_updates(ops, cx))
}

fn unchanged(buffer: &Entity<Buffer>, ops: Vec<proto::Operation>, cx: &mut TestAppContext) {
    let before = buffer.read_with(cx, |b, _| {
        (
            b.text(),
            b.version(),
            b.is_dirty(),
            b.saved_version().clone(),
        )
    });
    assert!(apply(buffer, ops, cx).is_err());
    buffer.read_with(cx, |b, _| {
        assert_eq!(
            (
                b.text(),
                b.version(),
                b.is_dirty(),
                b.saved_version().clone()
            ),
            before
        );
        assert!(!b.has_deferred_ops());
    });
}

#[gpui::test]
fn cowboy_remote_edits_and_undo_deduplicate_exact_history(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local("base🙂", cx));
    let mut peer = peer("base🙂", 1);
    let edit = wire(peer.edit([(0..4, "changed")]));
    peer.finalize_last_transaction();
    let undo = wire(peer.undo().unwrap().1);
    apply(&buffer, vec![edit.clone(), undo.clone(), edit.clone()], cx).unwrap();
    assert_eq!(buffer.read_with(cx, |b, _| b.text()), "base🙂");
    let version = buffer.read_with(cx, |b, _| b.version());
    apply(&buffer, vec![edit, undo], cx).unwrap();
    assert_eq!(buffer.read_with(cx, |b, _| b.version()), version);
}

#[gpui::test]
fn cowboy_remote_edits_refuse_conflicts_and_whole_malformed_batches(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local("base🙂", cx));
    let mut peer = peer("base🙂", 1);
    let edit = wire(peer.edit([(0..0, "first")]));
    let next = wire(peer.edit([(0..0, "second")]));
    for case in 0..8 {
        let mut bad = next.clone();
        let proto::operation::Variant::Edit(value) = bad.variant.as_mut().unwrap() else {
            unreachable!()
        };
        match case {
            0 => value.replica_id = 65537,
            1 => value.lamport_timestamp = u32::MAX,
            2 => value.version.push(proto::VectorClockEntry {
                replica_id: 65536,
                timestamp: 1,
            }),
            3 => value.version.push(value.version[0].clone()),
            4 => value.new_text.clear(),
            5 => value.new_text[0] = "not\rnormalized".into(),
            6 => value.ranges[0].end = u64::MAX,
            _ => value.ranges[0] = proto::Range { start: 2, end: 1 },
        }
        unchanged(&buffer, vec![edit.clone(), bad], cx);
    }
    unchanged(&buffer, vec![edit.clone(), proto::Operation::default()], cx);
    unchanged(&buffer, vec![edit.clone(); 129], cx);
    apply(&buffer, vec![edit.clone()], cx).unwrap();
    let mut changed = edit.clone();
    let proto::operation::Variant::Edit(value) = changed.variant.as_mut().unwrap() else {
        unreachable!()
    };
    value.new_text[0] = "same id, different text".into();
    unchanged(&buffer, vec![next, changed], cx);
}

#[gpui::test]
fn cowboy_remote_edits_refuse_missing_causal_history_without_defer(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local("base", cx));
    let mut peer = peer("base", 1);
    let first = wire(peer.edit([(0..0, "1")]));
    let second = wire(peer.edit([(0..0, "2")]));
    unchanged(&buffer, vec![second.clone()], cx);
    apply(&buffer, vec![first], cx).unwrap();
    let mut omitted = second.clone();
    let proto::operation::Variant::Edit(value) = omitted.variant.as_mut().unwrap() else {
        unreachable!()
    };
    value.version.retain(|entry| entry.replica_id != 1);
    unchanged(&buffer, vec![omitted], cx);
    apply(&buffer, vec![second], cx).unwrap();
    assert_eq!(buffer.read_with(cx, |b, _| b.text()), "21base");
}

#[gpui::test]
fn cowboy_remote_edits_validate_unicode_tombstones_and_concurrent_versions(
    cx: &mut TestAppContext,
) {
    let buffer = cx.new(|cx| Buffer::local("a🙂z", cx));
    let mut first = peer("a🙂z", 1);
    let mut second = peer("a🙂z", 2);
    let delete = first.edit([(1..5, "")]);
    let concurrent = second.edit([(5..6, "Z")]);
    apply(
        &buffer,
        vec![wire(delete.clone()), wire(concurrent.clone())],
        cx,
    )
    .unwrap();
    assert_eq!(buffer.read_with(cx, |b, _| b.text()), "aZ");
    first.apply_ops([concurrent]);
    let good = wire(first.edit([(1..1, "ok")]));
    let mut bad = good.clone();
    let proto::operation::Variant::Edit(value) = bad.variant.as_mut().unwrap() else {
        unreachable!()
    };
    value.ranges[0] = proto::Range { start: 2, end: 3 };
    unchanged(&buffer, vec![bad], cx);
    apply(&buffer, vec![good], cx).unwrap();
    assert_eq!(buffer.read_with(cx, |b, _| b.text()), "aokZ");
}

#[gpui::test]
fn cowboy_remote_edits_bound_aggregate_history_and_keep_duplicate_capacity(
    cx: &mut TestAppContext,
) {
    let buffer = cx.new(|cx| Buffer::local("", cx));
    let mut peer = peer("", 1);
    let first = wire(peer.edit([(0..0, "a".repeat(budget::MAX_TEXT))]));
    apply(&buffer, vec![first], cx).unwrap();
    let second = wire(peer.edit([(0..budget::MAX_TEXT, "b".repeat(budget::MAX_TEXT))]));
    apply(&buffer, vec![second.clone()], cx).unwrap();
    apply(&buffer, vec![second], cx).unwrap();
    let exceeded = wire(peer.edit([(0..budget::MAX_TEXT, "c")]));
    unchanged(&buffer, vec![exceeded], cx);
    assert_eq!(
        buffer.read_with(cx, |b, _| b.text()),
        "b".repeat(budget::MAX_TEXT)
    );
}

#[gpui::test]
fn cowboy_remote_edits_count_undo_before_mutating_any_prefix(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local("a", cx));
    let mut peer = peer("a", 1);
    let mut batch = Vec::new();
    for i in 0..budget::MAX_OPERATIONS - 1 {
        batch.push(wire(
            peer.edit([(0..1, if i % 2 == 0 { "b" } else { "a" })]),
        ));
        peer.finalize_last_transaction();
        if batch.len() == 128 {
            apply(&buffer, std::mem::take(&mut batch), cx).unwrap();
        }
    }
    apply(&buffer, batch, cx).unwrap();
    let final_edit = wire(peer.edit([(0..1, "z")]));
    peer.finalize_last_transaction();
    let undo = wire(peer.undo().unwrap().1);
    unchanged(&buffer, vec![final_edit.clone(), undo.clone()], cx);
    apply(&buffer, vec![final_edit.clone()], cx).unwrap();
    apply(&buffer, vec![final_edit], cx).unwrap();
    unchanged(&buffer, vec![undo], cx);
}

#[gpui::test]
fn cowboy_remote_edits_refuse_oversized_visible_result_and_readonly(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local("a".repeat(budget::MAX_TEXT), cx));
    let mut writer = peer(&"a".repeat(budget::MAX_TEXT), 1);
    unchanged(&buffer, vec![wire(writer.edit([(0..0, "x")]))], cx);
    let buffer = cx.new(|cx| Buffer::local("a", cx));
    buffer.update(cx, |b, cx| b.set_capability(Capability::ReadOnly, cx));
    unchanged(&buffer, vec![wire(peer("a", 1).edit([(0..0, "x")]))], cx);
}

#[gpui::test]
async fn cowboy_remote_edits_unknown_and_expired_ids_do_not_allocate_queues(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        let settings = settings::SettingsStore::test(cx);
        cx.set_global(settings);
    });
    let fs = fs::FakeFs::new(cx.executor());
    let trees = cx.new(|_| WorktreeStore::local(true, fs.clone(), Default::default()));
    let store = cx.new(|cx| BufferStore::local(trees.clone(), cx));
    for id in 1..=1024 {
        let result = BufferStore::handle_update_buffer(
            store.clone(),
            TypedEnvelope {
                message_id: id as u32,
                received_at: std::time::Instant::now(),
                payload: proto::UpdateBuffer {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    buffer_id: id,
                    operations: vec![],
                },
                sender_id: PeerId::default(),
                original_sender_id: None,
            },
            cx.to_async(),
        )
        .await;
        assert!(result.is_err());
    }
    store.read_with(cx, |store, _| assert!(store.opened_buffers.is_empty()));
    let buffer = cx.new(|cx| Buffer::local("retained", cx));
    let id = buffer.read_with(cx, |buffer, _| buffer.remote_id());
    store.update(cx, |store, cx| {
        store.add_buffer(buffer.clone(), cx).unwrap();
        store
            .shared_buffers
            .entry(PeerId::default())
            .or_default()
            .insert(
                id,
                SharedBuffer {
                    buffer: buffer.clone(),
                    lsp_handle: None,
                },
            );
    });
    for (sender, project, accepted) in [
        (PeerId::default(), proto::REMOTE_SERVER_PROJECT_ID, true),
        (
            PeerId { owner_id: 0, id: 1 },
            proto::REMOTE_SERVER_PROJECT_ID,
            false,
        ),
        (
            PeerId::default(),
            proto::REMOTE_SERVER_PROJECT_ID + 1,
            false,
        ),
    ] {
        assert_eq!(
            BufferStore::handle_update_buffer(
                store.clone(),
                TypedEnvelope {
                    message_id: 1,
                    received_at: std::time::Instant::now(),
                    sender_id: sender,
                    original_sender_id: None,
                    payload: proto::UpdateBuffer {
                        project_id: project,
                        buffer_id: id.into(),
                        operations: vec![]
                    },
                },
                cx.to_async()
            )
            .await
            .is_ok(),
            accepted
        );
    }
    store.update(cx, |store, _| store.shared_buffers.clear());
    drop(buffer);
    cx.update(|_| {});
    assert!(
        BufferStore::handle_update_buffer(
            store.clone(),
            TypedEnvelope {
                message_id: 1,
                received_at: std::time::Instant::now(),
                sender_id: PeerId::default(),
                original_sender_id: None,
                payload: proto::UpdateBuffer {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    buffer_id: id.into(),
                    operations: vec![]
                },
            },
            cx.to_async()
        )
        .await
        .is_err()
    );
    store.read_with(cx, |store, _| assert!(store.opened_buffers.is_empty()));
}

#[gpui::test]
fn cowboy_remote_edits_require_transitive_history_and_known_undo_targets(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local("x", cx));
    let mut first = peer("x", 1);
    let first_op = first.edit([(0..0, "1")]);
    let mut second = peer("x", 2);
    second.apply_ops([first_op.clone()]);
    let second_op = second.edit([(0..0, "2")]);
    let mut third = peer("x", 3);
    third.apply_ops([first_op.clone(), second_op.clone()]);
    apply(&buffer, vec![wire(first_op), wire(second_op)], cx).unwrap();
    let mut incomplete = wire(third.edit([(0..0, "3")]));
    let proto::operation::Variant::Edit(edit) = incomplete.variant.as_mut().unwrap() else {
        unreachable!()
    };
    edit.version.retain(|entry| entry.replica_id != 1);
    unchanged(&buffer, vec![incomplete], cx);
    third.finalize_last_transaction();
    let mut bad = wire(third.undo().unwrap().1);
    let proto::operation::Variant::Undo(undo) = bad.variant.as_mut().unwrap() else {
        unreachable!()
    };
    undo.counts[0].replica_id = 65539;
    unchanged(&buffer, vec![bad], cx);
}

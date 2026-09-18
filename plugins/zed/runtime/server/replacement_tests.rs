// SPDX-License-Identifier: GPL-3.0-or-later
//! Actual local loaders, bounded diffs and atomic history refusal.
use super::*;
use fs::FakeFs;
use gpui::TestAppContext;
use language::cowboy_replacement::{self as budget, Refusal};
use util::rel_path::rel_path;

struct Fixture {
    fs: Arc<FakeFs>,
    _store: Entity<BufferStore>,
    buffer: Entity<Buffer>,
}

impl Fixture {
    async fn new(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/cowboy",
            serde_json::json!({"replacement.txt": "before🙂\n"}),
        )
        .await;
        let trees = cx.new(|_| WorktreeStore::local(true, fs.clone(), Default::default()));
        let store = cx.new(|cx| BufferStore::local(trees.clone(), cx));
        let (tree, _) = trees
            .update(cx, |trees, cx| {
                trees.find_or_create_worktree("/cowboy", true, cx)
            })
            .await
            .unwrap();
        let worktree_id = tree.read_with(cx, |tree, _| tree.id());
        let buffer = store
            .update(cx, |store, cx| {
                store.open_buffer(
                    ProjectPath {
                        worktree_id,
                        path: rel_path("replacement.txt").into(),
                    },
                    cx,
                )
            })
            .await
            .unwrap();
        fs.pause_events();
        Self {
            fs,
            _store: store,
            buffer,
        }
    }
}

fn diff(
    buffer: &Entity<Buffer>,
    text: String,
    cx: &mut TestAppContext,
) -> Task<Result<budget::Replacement, Refusal>> {
    buffer.update(cx, |buffer, cx| {
        let job = budget::acquire(cx).unwrap();
        buffer.cowboy_diff(text, job, cx).unwrap()
    })
}

#[gpui::test]
async fn cowboy_replacement_capacity_refuses_reload_and_releases_failed_load(
    cx: &mut TestAppContext,
) {
    let f = Fixture::new(cx).await;
    let other = Fixture::new(cx).await;
    let jobs = cx.update(|cx| {
        (0..budget::MAX_JOBS)
            .map(|_| budget::acquire(cx).unwrap())
            .collect::<Vec<_>>()
    });
    assert!(cx.update(budget::acquire).is_err());
    let before = f.buffer.read_with(cx, |b, _| (b.text(), b.version()));
    assert!(f.buffer.update(cx, |b, cx| b.reload(cx)).await.is_err());
    assert!(other.buffer.update(cx, |b, cx| b.reload(cx)).await.is_err());
    f.buffer.read_with(cx, |b, _| {
        assert_eq!((b.text(), b.version()), before);
        assert!(b.cowboy_can_sync());
    });
    assert_eq!(cx.read(budget::in_use), budget::MAX_JOBS);
    drop(jobs);
    f.fs.insert_file("/cowboy/replacement.txt", vec![b'a'; budget::MAX_TEXT + 1])
        .await;
    assert!(f.buffer.update(cx, |b, cx| b.reload(cx)).await.is_err());
    assert_eq!(cx.read(budget::in_use), 0);
    f.fs.insert_file("/cowboy/replacement.txt", b"later\n".to_vec())
        .await;
    f.buffer.update(cx, |b, cx| b.reload(cx)).await.unwrap();
    assert_eq!(cx.read(budget::in_use), 0);
    assert_eq!(f.buffer.read_with(cx, |b, _| b.text()), "later\n");
}

#[gpui::test]
async fn cowboy_replacement_actual_loader_and_ready_result_own_charge(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    let task = f.buffer.update(cx, |buffer, cx| {
        let job = budget::acquire(cx).unwrap();
        buffer
            .file()
            .unwrap()
            .as_local()
            .unwrap()
            .cowboy_load_bytes(job, cx)
    });
    cx.run_until_parked();
    assert_eq!(cx.read(budget::in_use), 1);
    let loaded = task.await.unwrap();
    assert_eq!(loaded.value, "before🙂\n".as_bytes());
    drop(f);
    cx.update(|_| {});
    assert_eq!(cx.read(budget::in_use), 1);
    let alias = loaded.job.clone();
    drop(loaded);
    assert_eq!(cx.read(budget::in_use), 1);
    std::thread::spawn(move || drop(alias)).join().unwrap();
    assert_eq!(cx.read(budget::in_use), 0);
}

#[gpui::test]
async fn cowboy_replacement_ready_diff_outlives_source_and_cancelled_observer(
    cx: &mut TestAppContext,
) {
    let buffer = cx.new(|cx| Buffer::local("original", cx));
    let task = diff(&buffer, "next".into(), cx);
    cx.run_until_parked();
    assert_eq!(cx.read(budget::in_use), 1);
    let replacement = task.await.unwrap();
    drop(buffer);
    cx.update(|_| {});
    assert_eq!(cx.read(budget::in_use), 1);
    drop(replacement);
    assert_eq!(cx.read(budget::in_use), 0);
    let buffer = cx.new(|cx| Buffer::local("original", cx));
    let task = diff(&buffer, "next".into(), cx);
    drop(task);
    cx.run_until_parked();
    assert_eq!(cx.read(budget::in_use), 0);
}

#[gpui::test]
async fn cowboy_replacement_checks_entity_and_edit_undo_aba_before_mutation(
    cx: &mut TestAppContext,
) {
    let first = cx.new(|cx| Buffer::local("same", cx));
    let second = cx.new(|cx| Buffer::local("same", cx));
    let replacement = diff(&first, "new".into(), cx).await.unwrap();
    assert_eq!(
        second
            .update(cx, |b, cx| b.cowboy_apply_replacement(replacement, cx))
            .unwrap_err(),
        Refusal::Changed
    );
    let replacement = diff(&first, "new".into(), cx).await.unwrap();
    first.update(cx, |buffer, cx| {
        buffer.finalize_last_transaction();
        buffer.edit([(0..0, "edit")], None, cx);
        buffer.finalize_last_transaction();
        buffer.undo(cx);
        assert_eq!(buffer.text(), "same");
        let version = buffer.version();
        assert_eq!(
            buffer
                .cowboy_apply_replacement(replacement, cx)
                .unwrap_err(),
            Refusal::Changed
        );
        assert_eq!(buffer.version(), version);
    });
    assert_eq!(cx.read(budget::in_use), 0);
}

#[gpui::test]
async fn cowboy_replacement_bounds_history_bytes_before_undo_or_saved_state_changes(
    cx: &mut TestAppContext,
) {
    let buffer = cx.new(|cx| Buffer::local("", cx));
    for byte in ['a', 'b'] {
        let replacement = diff(&buffer, byte.to_string().repeat(budget::MAX_TEXT), cx)
            .await
            .unwrap();
        buffer
            .update(cx, |b, cx| b.cowboy_apply_replacement(replacement, cx))
            .unwrap();
    }
    // Exactly 8 MiB retained text fits. A no-op still fits without growing history.
    let unchanged = diff(&buffer, "b".repeat(budget::MAX_TEXT), cx)
        .await
        .unwrap();
    buffer
        .update(cx, |b, cx| b.cowboy_apply_replacement(unchanged, cx))
        .unwrap();
    let replacement = diff(&buffer, "c".into(), cx).await.unwrap();
    buffer.update(cx, |b, cx| {
        let version = b.version();
        let saved = b.saved_version().clone();
        assert_eq!(
            b.cowboy_apply_replacement(replacement, cx).unwrap_err(),
            Refusal::History
        );
        assert_eq!(b.version(), version);
        assert_eq!(b.saved_version(), &saved);
        assert_eq!(b.len(), budget::MAX_TEXT);
        // No capacity eviction/pruning: the original undo transaction is intact.
        b.undo(cx);
        assert_eq!(b.text(), "a".repeat(budget::MAX_TEXT));
    });
    assert_eq!(cx.read(budget::in_use), 0);
}

#[gpui::test]
async fn cowboy_replacement_bounds_operations_including_undo(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local("", cx));
    buffer.update(cx, |b, cx| {
        for i in 0..budget::MAX_OPERATIONS {
            b.edit([(0..b.len(), if i % 2 == 0 { "a" } else { "b" })], None, cx);
        }
        assert!(b.cowboy_check_replacement().is_ok());
    });
    let replacement = diff(&buffer, "c".into(), cx).await.unwrap();
    buffer.update(cx, |b, cx| {
        let version = b.version();
        assert_eq!(
            b.cowboy_apply_replacement(replacement, cx).unwrap_err(),
            Refusal::History
        );
        assert_eq!(b.version(), version);
        // An upstream writer is intentionally outside this finite admission.
        // Its extra undo must be seen, not hidden by an optimistic local counter.
        b.undo(cx);
        assert_eq!(b.cowboy_check_replacement(), Err(Refusal::History));
    });
}

#[gpui::test]
async fn cowboy_replacement_diff_cap_refuses_whole_result_not_a_prefix(cx: &mut TestAppContext) {
    let old = (0..=budget::MAX_DIFF_EDITS)
        .map(|i| format!("old-{i}\nanchor-{i}\n"))
        .collect::<String>();
    let new = (0..=budget::MAX_DIFF_EDITS)
        .map(|i| format!("new-{i}\nanchor-{i}\n"))
        .collect::<String>();
    let buffer = cx.new(|cx| Buffer::local(old.clone(), cx));
    assert!(matches!(diff(&buffer, new, cx).await, Err(Refusal::Input)));
    assert_eq!(buffer.read_with(cx, |b, _| b.text()), old);
    assert_eq!(cx.read(budget::in_use), 0);
    let exact = (0..=budget::MAX_DIFF_EDITS)
        .map(|i| {
            format!(
                "{}-{i}\nanchor-{i}\n",
                if i == budget::MAX_DIFF_EDITS {
                    "old"
                } else {
                    "new"
                }
            )
        })
        .collect::<String>();
    let replacement = diff(&buffer, exact.clone(), cx).await.unwrap();
    assert_eq!(replacement.diff().edits.len(), budget::MAX_DIFF_EDITS);
    buffer
        .update(cx, |b, cx| b.cowboy_apply_replacement(replacement, cx))
        .unwrap();
    assert_eq!(buffer.read_with(cx, |b, _| b.text()), exact);
}

#[gpui::test]
async fn cowboy_replacement_counts_aggregate_history_parts(cx: &mut TestAppContext) {
    let buffer = cx.new(|cx| Buffer::local(".".repeat(budget::MAX_PARTS), cx));
    buffer.update(cx, |b, cx| {
        b.edit(
            (0..budget::MAX_PARTS / 2).map(|i| (i * 2..i * 2 + 1, "x")),
            None,
            cx,
        );
        assert!(b.cowboy_check_replacement().is_ok());
    });
    let before = buffer.read_with(cx, |b, _| (b.text(), b.version()));
    let replacement = diff(&buffer, format!("{}tail", before.0), cx)
        .await
        .unwrap();
    assert_eq!(
        buffer
            .update(cx, |b, cx| b.cowboy_apply_replacement(replacement, cx))
            .unwrap_err(),
        Refusal::History
    );
    assert_eq!(buffer.read_with(cx, |b, _| (b.text(), b.version())), before);
}

#[gpui::test]
fn cowboy_replacement_refuses_dense_vectors_and_deferred_history(cx: &mut TestAppContext) {
    for deferred in [false, true] {
        let buffer = cx.new(|cx| Buffer::local("x", cx));
        buffer.update(cx, |b, cx| {
            let mut peer = text::Buffer::new(
                ReplicaId::new(if deferred { 1 } else { 256 }),
                b.remote_id(),
                "x",
            );
            let mut operation = peer.edit([(0..0, "y")]);
            if deferred {
                let text::Operation::Edit(edit) = &mut operation else {
                    unreachable!()
                };
                edit.version.observe(clock::Lamport {
                    replica_id: ReplicaId::new(2),
                    value: 99,
                });
            }
            b.apply_ops([language::Operation::Buffer(operation)], cx);
            let before = (b.text(), b.version());
            assert_eq!(b.cowboy_check_replacement(), Err(Refusal::History));
            assert_eq!((b.text(), b.version()), before);
        });
    }
}

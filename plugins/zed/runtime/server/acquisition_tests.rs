// SPDX-License-Identifier: GPL-3.0-or-later
//! Admission follows actual initial loads/results and native buffer entities.
use super::*;
use fs::FakeFs;
use gpui::TestAppContext;
use language::cowboy_buffer_budget as budget;
use util::rel_path::rel_path;

struct Fixture {
    fs: Arc<FakeFs>,
    store: Entity<BufferStore>,
    tree: Entity<Worktree>,
}

impl Fixture {
    async fn new(root: &str, cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            if cx.try_global::<settings::SettingsStore>().is_none() {
                let settings = settings::SettingsStore::test(cx);
                cx.set_global(settings);
            }
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(root, serde_json::json!({"a": "kept🙂\n"}))
            .await;
        let trees = cx.new(|_| WorktreeStore::local(true, fs.clone(), Default::default()));
        let store = cx.new(|cx| BufferStore::local(trees.clone(), cx));
        let (tree, _) = trees
            .update(cx, |trees, cx| {
                trees.find_or_create_worktree(root, true, cx)
            })
            .await
            .unwrap();
        Self { fs, store, tree }
    }

    fn open(&self, name: &str, cx: &mut TestAppContext) -> Task<Result<Entity<Buffer>>> {
        let path = ProjectPath {
            worktree_id: self.tree.read_with(cx, |tree, _| tree.id()),
            path: rel_path(name).into(),
        };
        self.store
            .update(cx, |store, cx| store.open_buffer(path, cx))
    }

    fn empty_indexes(&self, cx: &TestAppContext) {
        self.store.read_with(cx, |store, _| {
            assert!(store.opened_buffers.is_empty());
            assert!(store.path_to_buffer_id.is_empty());
            assert!(store.non_searchable_buffers.is_empty());
            assert!(store.loading_buffers.is_empty());
            let BufferStoreState::Local(local) = &store.state else {
                panic!("not local");
            };
            assert!(local.local_buffer_ids_by_entry_id.is_empty());
        });
    }
}

fn used(cx: &mut TestAppContext) -> usize {
    // GPUI releases zero-reference entities at the end of an update turn.
    cx.update(|_| {});
    cx.read(budget::in_use)
}

fn capacity<T>(result: Result<T>) {
    let Err(error) = result else {
        panic!("native acquisition exceeded capacity");
    };
    assert!(error.downcast_ref::<budget::CapacityExceeded>().is_some());
}

#[gpui::test]
async fn cowboy_acquisition_shared_capacity_and_reuse(cx: &mut TestAppContext) {
    let first = Fixture::new("/cowboy", cx).await;
    let second = Fixture::new("/other", cx).await;
    let mut buffers = vec![
        first.open("a", cx).await.unwrap(),
        second.open("a", cx).await.unwrap(),
    ];
    for i in 2..budget::MAX_BUFFERS {
        let fixture = if i % 2 == 0 { &first } else { &second };
        buffers.push(
            fixture
                .store
                .update(cx, |store, cx| store.create_buffer(None, false, cx))
                .await
                .unwrap(),
        );
    }
    assert_eq!(used(cx), budget::MAX_BUFFERS);
    let alias = first.open("a", cx).await.unwrap();
    assert_eq!(alias, buffers[0]);
    assert_eq!(used(cx), budget::MAX_BUFFERS);
    cx.run_until_parked();
    let metadata = first.fs.metadata_call_count();
    capacity(first.open("refused", cx).await);
    assert_eq!(first.fs.metadata_call_count(), metadata);
    first.store.read_with(cx, |store, _| {
        assert!(store.loading_buffers.is_empty());
    });
    capacity(
        second
            .store
            .update(cx, |store, cx| store.create_buffer(None, true, cx))
            .await,
    );
    drop(buffers.remove(0));
    assert_eq!(used(cx), budget::MAX_BUFFERS);
    drop(alias);
    assert_eq!(used(cx), budget::MAX_BUFFERS - 1);
    // New independent acquisition, not a replay of the refused attempt.
    buffers.push(first.open("independent", cx).await.unwrap());
    assert_eq!(used(cx), budget::MAX_BUFFERS);
    drop(buffers);
    assert_eq!(used(cx), 0);
    first.empty_indexes(cx);
    second.empty_indexes(cx);
}

#[gpui::test]
async fn cowboy_acquisition_pending_dedup_and_lost_observer(cx: &mut TestAppContext) {
    let f = Fixture::new("/cowboy", cx).await;
    let held = cx.update(|cx| {
        (1..budget::MAX_BUFFERS)
            .map(|_| budget::acquire(cx).unwrap())
            .collect::<Vec<_>>()
    });
    let first = f.open("a", cx);
    let second = f.open("a", cx);
    assert_eq!(used(cx), budget::MAX_BUFFERS);
    f.store.read_with(cx, |store, _| {
        assert_eq!(store.loading_buffers.len(), 1);
    });
    capacity(f.open("another", cx).await);
    drop(first);
    let buffer = second.await.unwrap();
    assert_eq!(used(cx), budget::MAX_BUFFERS);
    assert_eq!(buffer.read_with(cx, |buffer, _| buffer.text()), "kept🙂\n");
    drop(buffer);
    assert_eq!(used(cx), budget::MAX_BUFFERS - 1);
    drop(held);
    assert_eq!(used(cx), 0);
    f.empty_indexes(cx);
}

#[gpui::test]
async fn cowboy_acquisition_loaded_result_keeps_charge(cx: &mut TestAppContext) {
    let f = Fixture::new("/cowboy", cx).await;
    let permit = cx.update(|cx| budget::acquire(cx).unwrap());
    let task = f.tree.update(cx, |tree, cx| {
        tree.cowboy_load_file(rel_path("a"), permit.clone(), cx)
    });
    drop(permit);
    // The background result owns the charge even before anyone receives it.
    cx.run_until_parked();
    assert_eq!(used(cx), 1);
    let loaded = task.await.unwrap();
    assert_eq!(loaded.text, "kept🙂\n");
    assert_eq!(used(cx), 1);
    drop(f);
    assert_eq!(used(cx), 1);
    drop(loaded);
    assert_eq!(used(cx), 0);
}

#[gpui::test]
async fn cowboy_acquisition_failures_and_cancelled_untitled_return_capacity(
    cx: &mut TestAppContext,
) {
    let f = Fixture::new("/cowboy", cx).await;
    f.fs.insert_file("/cowboy/oversized", vec![b'x'; 4 * 1024 * 1024 + 1])
        .await;
    assert!(f.open("oversized", cx).await.is_err());
    assert_eq!(used(cx), 0);
    f.empty_indexes(cx);
    let task = f
        .store
        .update(cx, |store, cx| store.create_buffer(None, false, cx));
    assert_eq!(used(cx), 1);
    drop(task);
    cx.run_until_parked();
    assert_eq!(used(cx), 0);
    f.empty_indexes(cx);
    let buffer = f.open("a", cx).await.unwrap();
    assert_eq!(used(cx), 1);
    drop(f);
    // Dropping a store/worktree observer does not release a live entity.
    assert_eq!(used(cx), 1);
    drop(buffer);
    assert_eq!(used(cx), 0);
}

#[gpui::test]
async fn cowboy_acquisition_released_indexes_do_not_accumulate(cx: &mut TestAppContext) {
    let f = Fixture::new("/cowboy", cx).await;
    for i in 0..budget::MAX_BUFFERS * 2 {
        let name = format!("file-{i}");
        f.fs.insert_file(format!("/cowboy/{name}"), b"contents\n".to_vec())
            .await;
        let buffer = f.open(&name, cx).await.unwrap();
        let id = buffer.read_with(cx, |buffer, _| buffer.remote_id());
        f.store.update(cx, |store, _| {
            store.non_searchable_buffers.insert(id);
        });
        assert_eq!(used(cx), 1);
        drop(buffer);
        assert_eq!(used(cx), 0);
        f.empty_indexes(cx);
    }
}

#[gpui::test]
fn cowboy_acquisition_cloned_background_charge_drops_once(cx: &mut TestAppContext) {
    let permit = cx.update(|cx| budget::acquire(cx).unwrap());
    let a = permit.clone();
    let b = a.clone();
    drop(permit);
    assert_eq!(used(cx), 1);
    std::thread::spawn(move || drop(a)).join().unwrap();
    assert_eq!(used(cx), 1);
    std::thread::spawn(move || drop(b)).join().unwrap();
    assert_eq!(used(cx), 0);
}

#[gpui::test]
async fn cowboy_acquisition_old_release_preserves_replacement_indexes(cx: &mut TestAppContext) {
    let f = Fixture::new("/cowboy", cx).await;
    let original = f.open("a", cx).await.unwrap();
    let (id, file) =
        original.read_with(cx, |buffer, _| (buffer.remote_id(), buffer.file().cloned()));
    drop(original);
    // Deliberately re-register an equal native ID before GPUI flushes the old
    // entity's release. Only the entity identity can distinguish this ABA.
    let replacement = f.store.update(cx, |store, cx| {
        let permit = budget::acquire(cx).unwrap();
        let buffer = cx.new(|_| {
            Buffer::build(
                text::Buffer::new(ReplicaId::LOCAL, id, "replacement"),
                file,
                Capability::ReadWrite,
            )
            .cowboy_with_acquisition(permit)
        });
        store.add_buffer(buffer.clone(), cx).unwrap();
        buffer
    });
    assert_eq!(used(cx), 1);
    f.store.read_with(cx, |store, _| {
        assert_eq!(store.get(id), Some(replacement.clone()));
        assert!(store.path_to_buffer_id.values().any(|value| *value == id));
    });
    drop(replacement);
    assert_eq!(used(cx), 0);
    f.empty_indexes(cx);
}

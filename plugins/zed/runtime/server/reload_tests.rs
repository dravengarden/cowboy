// SPDX-License-Identifier: GPL-3.0-or-later
//! The actual LocalFile/reload path, not the separate conditional sync loader.
use super::*;
use fs::{FakeFs, Fs as _, RemoveOptions};
use gpui::TestAppContext;
use util::rel_path::rel_path;

const LIMIT: usize = 4 * 1024 * 1024;
const ORIGINAL: &str = "before🙂\n";

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
        fs.insert_tree("/cowboy", serde_json::json!({"reload.txt": ORIGINAL}))
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
                        path: rel_path("reload.txt").into(),
                    },
                    cx,
                )
            })
            .await
            .unwrap();
        // Isolate the reload invocation from file-object replacement/watchers.
        fs.pause_events();
        Self {
            fs,
            _store: store,
            buffer,
        }
    }

    async fn write(&self, bytes: Vec<u8>) {
        self.fs.insert_file("/cowboy/reload.txt", bytes).await;
    }

    fn unchanged(&self, version: &clock::Global, cx: &TestAppContext) {
        self.buffer.read_with(cx, |buffer, _| {
            assert_eq!(buffer.text(), ORIGINAL);
            assert_eq!(&buffer.version(), version);
            assert!(!buffer.is_dirty());
            // Also proves failure did not leave a completed reload_task lodged.
            assert!(buffer.cowboy_can_sync());
        });
    }
}

#[gpui::test]
async fn cowboy_reload_refuses_raw_growth_and_decoded_expansion(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    let version = f.buffer.read_with(cx, |buffer, _| buffer.version());
    let mut expanded = vec![0xff, 0xfe];
    for _ in 0..1_500_000 {
        expanded.extend_from_slice(&[0x00, 0x08]);
    }
    for bytes in [vec![b'a'; LIMIT + 1], expanded, vec![0; 1024]] {
        f.write(bytes.clone()).await;
        assert!(
            f.buffer
                .update(cx, |buffer, cx| buffer.reload(cx))
                .await
                .is_err()
        );
        f.unchanged(&version, cx);
        assert_eq!(
            f.fs.load_bytes("/cowboy/reload.txt".as_ref())
                .await
                .unwrap(),
            bytes
        );
    }
    // An independent later invocation is usable. Nothing retries automatically.
    f.write(b"later\n".to_vec()).await;
    f.buffer
        .update(cx, |buffer, cx| buffer.reload(cx))
        .await
        .unwrap();
    f.buffer.read_with(cx, |buffer, _| {
        assert_eq!(buffer.text(), "later\n");
        assert_ne!(buffer.version(), version);
        assert!(buffer.cowboy_can_sync());
    });
}

#[gpui::test]
async fn cowboy_reload_bounds_forced_decoding_and_accepts_exact_limit(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    let version = f.buffer.read_with(cx, |buffer, _| buffer.version());
    // A forced single-byte encoding expands each byte into a three-byte scalar.
    f.write(vec![0x80; 1_500_000]).await;
    assert!(
        f.buffer
            .update(cx, |buffer, cx| {
                buffer.reload_with_encoding(encoding_rs::WINDOWS_1252, cx)
            })
            .await
            .is_err()
    );
    f.unchanged(&version, cx);
    let exact = vec![b'x'; LIMIT];
    f.write(exact.clone()).await;
    f.buffer
        .update(cx, |buffer, cx| buffer.reload(cx))
        .await
        .unwrap();
    f.buffer.read_with(cx, |buffer, _| {
        assert_eq!(buffer.text().as_bytes(), exact);
        assert!(!buffer.is_dirty());
        assert!(buffer.cowboy_can_sync());
    });
}

#[gpui::test]
async fn cowboy_reload_clears_absent_and_no_file_tasks(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    let version = f.buffer.read_with(cx, |buffer, _| buffer.version());
    f.fs.remove_file("/cowboy/reload.txt".as_ref(), RemoveOptions::default())
        .await
        .unwrap();
    assert!(
        f.buffer
            .update(cx, |buffer, cx| buffer.reload(cx))
            .await
            .is_err()
    );
    f.unchanged(&version, cx);
    let untitled = cx.new(|cx| Buffer::local("", cx));
    assert!(
        untitled
            .update(cx, |buffer, cx| buffer.reload(cx))
            .await
            .is_err()
    );
    assert!(untitled.read_with(cx, |buffer, _| buffer.cowboy_can_sync()));
}

#[gpui::test]
async fn cowboy_reload_replacement_and_lost_observer_do_not_stick(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    f.write(b"latest\n".to_vec()).await;
    let (old, current) = f
        .buffer
        .update(cx, |buffer, cx| (buffer.reload(cx), buffer.reload(cx)));
    assert!(old.await.is_err());
    current.await.unwrap();
    f.buffer.read_with(cx, |buffer, _| {
        assert_eq!(buffer.text(), "latest\n");
        assert!(buffer.cowboy_can_sync());
    });
    f.write(vec![b'a'; LIMIT + 1]).await;
    drop(f.buffer.update(cx, |buffer, cx| buffer.reload(cx)));
    cx.run_until_parked();
    f.buffer.read_with(cx, |buffer, _| {
        assert_eq!(buffer.text(), "latest\n");
        assert!(buffer.cowboy_can_sync());
    });
}

#[gpui::test]
async fn cowboy_reload_completion_cannot_retire_an_observers_new_task(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    f.write(b"observed\n".to_vec()).await;
    let (started, next) = oneshot::channel();
    let mut started = Some(started);
    let _subscription = f.buffer.update(cx, |_, cx| {
        cx.subscribe(&cx.entity(), move |buffer, _, event, cx| {
            if matches!(event, BufferEvent::Reloaded)
                && let Some(started) = started.take()
            {
                started.send(buffer.reload(cx)).ok();
            }
        })
    });
    f.buffer
        .update(cx, |buffer, cx| buffer.reload(cx))
        .await
        .unwrap();
    next.await
        .unwrap()
        .await
        .expect("old cleanup cancelled the observer's new reload");
    f.buffer.read_with(cx, |buffer, _| {
        assert_eq!(buffer.text(), "observed\n");
        assert!(buffer.cowboy_can_sync());
    });
}

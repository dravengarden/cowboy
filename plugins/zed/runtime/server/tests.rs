// SPDX-License-Identifier: GPL-3.0-or-later
//! Compiled only into the native project test binary, never the shipped server.
use super::*;
use fs::FakeFs;
use gpui::TestAppContext;
use util::rel_path::rel_path;

pub(super) struct Barrier {
    reached: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

pub(super) async fn pause(
    this: &WeakEntity<BufferStore>,
    cx: &mut AsyncApp,
    stage: usize,
) -> Result<(), Refusal> {
    if let Some(barrier) = this
        .update(cx, |this, _| this.cowboy_sync.barriers[stage].take())
        .map_err(|_| Refusal::Changed)?
    {
        let _ = barrier.reached.send(());
        barrier.resume.await.map_err(|_| Refusal::Changed)?;
    }
    Ok(())
}

struct Fixture {
    store: Entity<BufferStore>,
    buffer: Entity<Buffer>,
    instance: Vec<u8>,
    expected: Vec<proto::CowboyBufferSyncVersion>,
}

impl Fixture {
    async fn new(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/cowboy", serde_json::json!({ "test.txt": "before🙂\n" }))
            .await;
        let worktrees = cx.new(|_| WorktreeStore::local(true, fs.clone(), Default::default()));
        let store = cx.new(|cx| BufferStore::local(worktrees.clone(), cx));
        let (worktree, _) = worktrees
            .update(cx, |worktrees, cx| {
                worktrees.find_or_create_worktree("/cowboy", true, cx)
            })
            .await
            .unwrap();
        let id = worktree.read_with(cx, |tree, _| tree.id());
        let buffer = store
            .update(cx, |store, cx| {
                store.open_buffer(
                    ProjectPath {
                        worktree_id: id,
                        path: rel_path("test.txt").into(),
                    },
                    cx,
                )
            })
            .await
            .unwrap();
        store.update(cx, |store, cx| {
            store
                .shared_buffers
                .entry(PeerId::default())
                .or_default()
                .insert(
                    buffer.read(cx).remote_id(),
                    SharedBuffer {
                        buffer: buffer.clone(),
                        lsp_handle: None,
                    },
                );
        });
        let instance = store.read_with(cx, |store, _| store.cowboy_sync.instance.to_vec());
        let expected = buffer.read_with(cx, |buffer, _| {
            serialize_version(&buffer.version())
                .into_iter()
                .map(|entry| proto::CowboyBufferSyncVersion {
                    replica_id: entry.replica_id,
                    timestamp: entry.timestamp,
                })
                .collect()
        });
        // Hold worktree events, not file reads. Native file-object replacement
        // has its own refusal; here we need to isolate actual mutation races.
        fs.pause_events();
        fs.insert_file("/cowboy/test.txt", b"after\n".to_vec())
            .await;
        Self {
            store,
            buffer,
            instance,
            expected,
        }
    }

    fn prepare(&self, cx: &mut TestAppContext) -> Result<proto::CowboyBufferSyncResponse> {
        let buffer_id = self
            .buffer
            .read_with(cx, |buffer, _| buffer.remote_id().into());
        self.store.update(cx, |store, cx| {
            store.cowboy_sync(
                proto::CowboyBufferSync {
                    protocol: 1,
                    instance: self.instance.clone(),
                    action: Action::Prepare as i32,
                    buffer_id,
                    version: self.expected.clone(),
                    content_sha256: Sha256::digest(b"after\n").to_vec(),
                    content_bytes: 6,
                    ..Default::default()
                },
                PeerId::default(),
                cx,
            )
        })
    }

    fn action(
        &self,
        cx: &mut TestAppContext,
        id: u64,
        action: Action,
    ) -> Result<proto::CowboyBufferSyncResponse> {
        self.store.update(cx, |store, cx| {
            store.cowboy_sync(
                proto::CowboyBufferSync {
                    protocol: 1,
                    instance: self.instance.clone(),
                    operation_id: id,
                    action: action as i32,
                    ..Default::default()
                },
                PeerId::default(),
                cx,
            )
        })
    }

    fn barrier(
        &self,
        cx: &mut TestAppContext,
        stage: usize,
    ) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (reached, ready) = oneshot::channel();
        let (resume, wait) = oneshot::channel();
        self.store.update(cx, |store, _| {
            store.cowboy_sync.barriers[stage] = Some(Barrier {
                reached,
                resume: wait,
            })
        });
        (ready, resume)
    }
}

#[gpui::test]
async fn cowboy_edits_undo_and_lost_sharing_refuse_at_each_async_boundary(cx: &mut TestAppContext) {
    for stage in 0..2 {
        for mutation in 0..6 {
            let fixture = Fixture::new(cx).await;
            let ticket = fixture.prepare(cx).unwrap().operation_id;
            let (ready, resume) = fixture.barrier(cx, stage);
            assert_eq!(
                fixture.action(cx, ticket, Action::Apply).unwrap().phase,
                Phase::Pending as i32
            );
            ready.await.unwrap();
            match mutation {
                0 | 1 => fixture.buffer.update(cx, |buffer, cx| {
                    buffer.finalize_last_transaction();
                    buffer.edit([(0..0, "new")], None, cx);
                    buffer.finalize_last_transaction();
                    if mutation == 1 {
                        buffer.undo(cx);
                    }
                }),
                2 => fixture
                    .store
                    .update(cx, |store, _| store.shared_buffers.clear()),
                3 => fixture.buffer.update(cx, |buffer, cx| {
                    buffer.set_capability(Capability::ReadOnly, cx)
                }),
                4 => fixture.store.update(cx, |store, cx| {
                    store
                        .shared_buffers
                        .entry(PeerId { owner_id: 0, id: 1 })
                        .or_default()
                        .insert(
                            fixture.buffer.read(cx).remote_id(),
                            SharedBuffer {
                                buffer: fixture.buffer.clone(),
                                lsp_handle: None,
                            },
                        );
                }),
                _ => fixture.store.update(cx, |store, cx| {
                    let id = File::from_dyn(fixture.buffer.read(cx).file())
                        .unwrap()
                        .worktree
                        .read(cx)
                        .id();
                    store
                        .worktree_store
                        .update(cx, |worktrees, cx| worktrees.remove_worktree(id, cx));
                }),
            }
            let before = fixture
                .buffer
                .read_with(cx, |buffer, _| (buffer.text(), buffer.version()));
            assert_eq!(
                fixture.action(cx, ticket, Action::Retire).unwrap().phase,
                Phase::Pending as i32
            );
            assert_eq!(
                fixture.action(cx, ticket, Action::Apply).unwrap().phase,
                Phase::Pending as i32
            );
            resume.send(()).unwrap();
            cx.executor().run_until_parked();
            let result = fixture.action(cx, ticket, Action::Query).unwrap();
            assert_eq!(
                result.phase,
                Phase::Refused as i32,
                "stage {stage}, mutation {mutation}: {result:?}"
            );
            assert_eq!(
                fixture
                    .buffer
                    .read_with(cx, |buffer, _| (buffer.text(), buffer.version())),
                before
            );
            assert_eq!(
                fixture.action(cx, ticket, Action::Apply).unwrap(),
                result,
                "duplicate effect replayed"
            );
        }
    }
}

#[gpui::test]
async fn cowboy_pending_survives_observation_loss_and_does_not_expire(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx).await;
    let ticket = fixture.prepare(cx).unwrap().operation_id;
    let (ready, resume) = fixture.barrier(cx, 1);
    // The response is deliberately discarded. Only Store ownership keeps the
    // admitted native task alive; no transport request owns its future.
    fixture.action(cx, ticket, Action::Apply).unwrap();
    ready.await.unwrap();
    assert_eq!(cx.read(cowboy_replacement::in_use), 1);
    fixture.store.update(cx, |store, _| {
        store.cowboy_sync.records.get_mut(&ticket).unwrap().created = Instant::now() - PREPARE_TTL
    });
    assert_eq!(
        fixture.action(cx, ticket, Action::Query).unwrap().phase,
        Phase::Pending as i32
    );
    resume.send(()).unwrap();
    cx.executor().run_until_parked();
    let result = fixture.action(cx, ticket, Action::Query).unwrap();
    assert_eq!(result.phase, Phase::Applied as i32);
    assert_eq!(
        fixture.buffer.read_with(cx, |buffer, _| buffer.text()),
        "after\n"
    );
    assert_eq!(fixture.action(cx, ticket, Action::Apply).unwrap(), result);
    assert_eq!(cx.read(cowboy_replacement::in_use), 0);
}

#[gpui::test]
async fn cowboy_sync_budget_refusal_is_terminal_and_not_a_replay_grant(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx).await;
    let ticket = fixture.prepare(cx).unwrap().operation_id;
    let jobs = cx.update(|cx| {
        (0..cowboy_replacement::MAX_JOBS)
            .map(|_| cowboy_replacement::acquire(cx).unwrap())
            .collect::<Vec<_>>()
    });
    let before = fixture.buffer.read_with(cx, |b, _| (b.text(), b.version()));
    let refused = fixture.action(cx, ticket, Action::Apply).unwrap();
    assert_eq!(refused.phase, Phase::Refused as i32);
    assert_eq!(refused.refusal, Refusal::Budget as i32);
    drop(jobs);
    assert_eq!(fixture.action(cx, ticket, Action::Apply).unwrap(), refused);
    assert_eq!(fixture.action(cx, ticket, Action::Query).unwrap(), refused);
    assert_eq!(
        fixture.buffer.read_with(cx, |b, _| (b.text(), b.version())),
        before
    );
    assert_eq!(cx.read(cowboy_replacement::in_use), 0);
    fixture.action(cx, ticket, Action::Retire).unwrap();
}

#[gpui::test]
async fn cowboy_sync_history_refusal_does_not_mark_saved_or_prune(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx).await;
    fixture.buffer.update(cx, |buffer, cx| {
        for i in 0..cowboy_replacement::MAX_OPERATIONS {
            buffer.edit(
                [(0..buffer.len(), if i % 2 == 0 { "a" } else { "b" })],
                None,
                cx,
            );
        }
        // Isolate capacity from dirty-state refusal. This test does not write disk.
        buffer.did_reload(buffer.version(), LineEnding::Unix, buffer.saved_mtime(), cx);
    });
    let expected = fixture.buffer.read_with(cx, |buffer, _| {
        serialize_version(&buffer.version())
            .into_iter()
            .map(|v| proto::CowboyBufferSyncVersion {
                replica_id: v.replica_id,
                timestamp: v.timestamp,
            })
            .collect()
    });
    let fixture = Fixture {
        expected,
        ..fixture
    };
    let ticket = fixture.prepare(cx).unwrap().operation_id;
    let before = fixture.buffer.read_with(cx, |b, _| {
        (b.text(), b.version(), b.saved_version().clone())
    });
    fixture.action(cx, ticket, Action::Apply).unwrap();
    cx.run_until_parked();
    let refused = fixture.action(cx, ticket, Action::Query).unwrap();
    assert_eq!(refused.phase, Phase::Refused as i32);
    assert_eq!(refused.refusal, Refusal::Budget as i32);
    assert_eq!(
        fixture.buffer.read_with(cx, |b, _| (
            b.text(),
            b.version(),
            b.saved_version().clone()
        )),
        before
    );
    assert_eq!(fixture.action(cx, ticket, Action::Apply).unwrap(), refused);
    assert_eq!(cx.read(cowboy_replacement::in_use), 0);
}

#[gpui::test]
async fn cowboy_dirty_prepare_and_unissued_tickets_have_no_effect(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx).await;
    fixture
        .buffer
        .update(cx, |buffer, cx| buffer.edit([(0..0, "unsaved")], None, cx));
    assert!(fixture.prepare(cx).is_err());
    assert!(fixture.action(cx, 1, Action::Query).is_err());
    assert_eq!(
        fixture
            .store
            .read_with(cx, |store, _| store.cowboy_sync.records.len()),
        0
    );
}

#[gpui::test]
async fn cowboy_capacity_expiry_and_single_pending_do_not_recycle_identity(
    cx: &mut TestAppContext,
) {
    let fixture = Fixture::new(cx).await;
    let mut ids = Vec::new();
    for _ in 0..MAX_RECORDS {
        ids.push(fixture.prepare(cx).unwrap().operation_id);
    }
    assert!(fixture.prepare(cx).is_err());
    fixture.store.update(cx, |store, _| {
        store.cowboy_sync.records.get_mut(&ids[0]).unwrap().created = Instant::now() - PREPARE_TTL;
    });
    assert_eq!(
        fixture.action(cx, ids[0], Action::Apply).unwrap().phase,
        Phase::Retired as i32
    );
    let next = fixture.prepare(cx).unwrap().operation_id;
    assert!(next > *ids.last().unwrap());
    let (ready, resume) = fixture.barrier(cx, 0);
    fixture.action(cx, next, Action::Apply).unwrap();
    ready.await.unwrap();
    assert!(fixture.action(cx, ids[1], Action::Apply).is_err());
    assert_eq!(
        fixture.action(cx, ids[1], Action::Query).unwrap().phase,
        Phase::Prepared as i32
    );
    resume.send(()).unwrap();
    cx.executor().run_until_parked();
    assert_eq!(
        fixture.action(cx, next, Action::Query).unwrap().phase,
        Phase::Applied as i32
    );
    fixture.action(cx, ids[1], Action::Apply).unwrap();
    cx.executor().run_until_parked();
    assert_eq!(
        fixture.action(cx, ids[1], Action::Query).unwrap().phase,
        Phase::Refused as i32
    );
}

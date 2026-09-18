// SPDX-License-Identifier: GPL-3.0-or-later
//! Native ownership checks, not a claim that LSP/background work has stopped.
use super::*;
use fs::FakeFs;
use gpui::TestAppContext;
use util::rel_path::rel_path;

struct Fixture {
    store: Entity<BufferStore>,
    buffers: Vec<Entity<Buffer>>,
    instance: Vec<u8>,
    ids: Vec<u64>,
}

#[gpui::test]
async fn cowboy_close_ack_does_not_return_acquisition_capacity(cx: &mut TestAppContext) {
    use language::cowboy_buffer_budget::in_use;
    let f = Fixture::new(cx).await;
    assert_eq!(cx.update(|cx| in_use(cx)), 2);
    let response = f.call(f.ids.clone(), PeerId::default(), cx).unwrap();
    assert_eq!(response.outcome, Outcome::Closed as i32);
    assert_eq!(f.retained(PeerId::default(), cx), 0);
    // Other native handles still own these buffers after the complete ACK.
    assert_eq!(cx.update(|cx| in_use(cx)), 2);
    drop(f.buffers);
    cx.update(|_| {});
    assert_eq!(cx.update(|cx| in_use(cx)), 0);
    f.store.read_with(cx, |store, _| {
        assert!(store.opened_buffers.is_empty());
        assert!(store.path_to_buffer_id.is_empty());
    });
}

impl Fixture {
    async fn new(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        });
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/cowboy", serde_json::json!({"a": "one🙂\n", "b": "two\n"}))
            .await;
        let worktrees = cx.new(|_| WorktreeStore::local(true, fs.clone(), Default::default()));
        let store = cx.new(|cx| BufferStore::local(worktrees.clone(), cx));
        let (worktree, _) = worktrees
            .update(cx, |store, cx| {
                store.find_or_create_worktree("/cowboy", true, cx)
            })
            .await
            .unwrap();
        let worktree_id = worktree.read_with(cx, |tree, _| tree.id());
        let mut buffers = Vec::new();
        for path in [rel_path("a"), rel_path("b")] {
            buffers.push(
                store
                    .update(cx, |store, cx| {
                        store.open_buffer(
                            ProjectPath {
                                worktree_id,
                                path: path.into(),
                            },
                            cx,
                        )
                    })
                    .await
                    .unwrap(),
            );
        }
        let mut ids = Vec::new();
        store.update(cx, |store, cx| {
            for buffer in &buffers {
                let id = buffer.read(cx).remote_id();
                ids.push(id.into());
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
            }
        });
        ids.sort_unstable();
        let instance = store.read_with(cx, |store, _| store.cowboy_close.instance.to_vec());
        Self {
            store,
            buffers,
            instance,
            ids,
        }
    }

    fn call(
        &self,
        ids: Vec<u64>,
        peer: PeerId,
        cx: &mut TestAppContext,
    ) -> Result<proto::CowboyCloseBuffersResponse> {
        self.request(
            proto::CowboyCloseBuffers {
                protocol: 1,
                instance: self.instance.clone(),
                buffer_ids: ids,
                ..Default::default()
            },
            peer,
            cx,
        )
    }

    fn request(
        &self,
        request: proto::CowboyCloseBuffers,
        peer: PeerId,
        cx: &mut TestAppContext,
    ) -> Result<proto::CowboyCloseBuffersResponse> {
        self.store.update(cx, |store, cx| {
            store.cowboy_close_buffers(request, peer, cx)
        })
    }

    fn retained(&self, peer: PeerId, cx: &TestAppContext) -> usize {
        self.store.read_with(cx, |store, _| {
            store.shared_buffers.get(&peer).map_or(0, HashMap::len)
        })
    }
}

#[gpui::test]
async fn cowboy_close_complete_set_and_original_peer_only(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    let other = PeerId { owner_id: 1, id: 2 };
    f.store.update(cx, |store, _| {
        let shared = store.shared_buffers[&PeerId::default()].clone();
        store.shared_buffers.insert(other, shared);
    });
    let response = f.call(f.ids.clone(), PeerId::default(), cx).unwrap();
    assert_eq!(response.outcome, Outcome::Closed as i32);
    assert_eq!(response.buffer_ids, f.ids);
    assert_eq!(response.instance, f.instance);
    assert_eq!(f.retained(PeerId::default(), cx), 0);
    assert_eq!(f.retained(other, cx), 2);
    assert_eq!(
        f.buffers[0].read_with(cx, |buffer, _| buffer.text()),
        "one🙂\n"
    );
    // A second request cannot claim an already absent peer was closed again.
    assert_eq!(
        f.call(f.ids.clone(), PeerId::default(), cx)
            .unwrap()
            .outcome,
        Outcome::Refused as i32
    );
    assert_eq!(f.retained(other, cx), 2);
}

#[gpui::test]
async fn cowboy_close_missing_member_refuses_without_partial_removal(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    let mut ids = f.ids.clone();
    ids.push(ids.last().unwrap() + 1_000_000);
    let response = f.call(ids.clone(), PeerId::default(), cx).unwrap();
    assert_eq!(response.outcome, Outcome::Refused as i32);
    assert_eq!(response.buffer_ids, ids);
    assert_eq!(f.retained(PeerId::default(), cx), 2);
    let foreign = PeerId { owner_id: 3, id: 4 };
    assert_eq!(
        f.call(f.ids.clone(), foreign, cx).unwrap().outcome,
        Outcome::Refused as i32
    );
    assert_eq!(f.retained(PeerId::default(), cx), 2);
}

#[gpui::test]
async fn cowboy_close_strict_probe_instance_and_input_bounds(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    let probe = proto::CowboyCloseBuffers {
        protocol: 1,
        ..Default::default()
    };
    let response = f.request(probe.clone(), PeerId::default(), cx).unwrap();
    assert_eq!(response.outcome, Outcome::Supported as i32);
    assert_eq!(response.instance, f.instance);
    assert!(response.buffer_ids.is_empty());
    for ids in [
        vec![],
        vec![0],
        vec![f.ids[0], f.ids[0]],
        vec![f.ids[1], f.ids[0]],
        (1..=34).collect(),
    ] {
        assert!(f.call(ids, PeerId::default(), cx).is_err());
    }
    for request in [
        proto::CowboyCloseBuffers {
            protocol: 2,
            ..probe.clone()
        },
        proto::CowboyCloseBuffers {
            project_id: 99,
            ..probe.clone()
        },
        proto::CowboyCloseBuffers {
            buffer_ids: f.ids.clone(),
            ..probe.clone()
        },
        proto::CowboyCloseBuffers {
            instance: vec![1; 15],
            buffer_ids: f.ids.clone(),
            ..probe.clone()
        },
        proto::CowboyCloseBuffers {
            instance: f.instance.iter().map(|byte| byte ^ 1).collect(),
            buffer_ids: f.ids.clone(),
            ..probe
        },
    ] {
        assert!(f.request(request, PeerId::default(), cx).is_err());
    }
    assert_eq!(f.retained(PeerId::default(), cx), 2);
}

#[gpui::test]
async fn cowboy_close_observer_loss_does_not_restore_native_ownership(cx: &mut TestAppContext) {
    let f = Fixture::new(cx).await;
    // Discard the original result. Absence is not a saved operation receipt;
    // adapter uncertainty must be retained rather than retrying this command.
    f.call(f.ids.clone(), PeerId::default(), cx).unwrap();
    assert_eq!(f.retained(PeerId::default(), cx), 0);
    assert_eq!(
        f.call(f.ids.clone(), PeerId::default(), cx)
            .unwrap()
            .outcome,
        Outcome::Refused as i32
    );
    let fresh = Fixture::new(cx).await;
    assert_ne!(fresh.instance, f.instance);
    assert!(
        fresh
            .request(
                proto::CowboyCloseBuffers {
                    protocol: 1,
                    instance: f.instance,
                    buffer_ids: fresh.ids.clone(),
                    ..Default::default()
                },
                PeerId::default(),
                cx
            )
            .is_err()
    );
    assert_eq!(fresh.retained(PeerId::default(), cx), 2);
}

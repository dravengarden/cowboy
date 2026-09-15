use super::*;
use crate::{BufferState, Request, ensure_worktree, respond};

#[tokio::test]
async fn disconnected_observer_does_not_lose_the_native_open_record() {
    use tokio::io::AsyncWriteExt as _;
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    let (mut client, peer) = tokio::net::UnixStream::pair().unwrap();
    let task = tokio::spawn(crate::handle(
        peer,
        Arc::clone(&fixture.worktrees),
        Arc::clone(&fixture.buffers),
        None,
    ));
    let mut wire = serde_json::to_vec(&Request::OpenBufferLease {
        lease: lease.clone(),
    })
    .unwrap();
    wire.push(b'\n');
    client.write_all(&wire).await.unwrap();
    drop(client);
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        state(
            fixture
                .request(Request::QueryBufferLease {
                    lease: lease.clone()
                })
                .await
                .unwrap()
        ),
        LeaseState::Open
    );
    std::fs::remove_file(fixture.root.join("file.txt")).unwrap();
    assert_eq!(
        state(
            fixture
                .request(Request::ReleaseBufferLease { lease })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
}

#[tokio::test]
async fn cancelled_native_open_is_unknown_and_never_replayed() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    let held = fixture.buffers.active.write().await;
    let mut registry = fixture.buffers.leases.lock().await;
    let worktrees = fixture.worktrees.read().await;
    let mut opening = Box::pin(open_prevalidated(
        registry.slots.get_mut(&1).unwrap(),
        1,
        worktrees.get(&fixture.root).unwrap().remote_id,
        &fixture.buffers,
        None,
    ));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(opening.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(opening);
    drop(worktrees);
    drop(registry);
    drop(held);
    for request in [
        Request::QueryBufferLease {
            lease: lease.clone(),
        },
        Request::OpenBufferLease {
            lease: lease.clone(),
        },
        Request::ReleaseBufferLease { lease },
    ] {
        assert_eq!(
            state(fixture.request(request).await.unwrap()),
            LeaseState::Unknown
        );
    }
    assert!(fixture.buffers.active.read().await.is_empty());
}

#[tokio::test]
async fn changed_symlink_cannot_retarget_a_prepared_buffer() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    std::fs::write(fixture.root.join("other.txt"), "other\n").unwrap();
    std::fs::remove_file(fixture.root.join("file.txt")).unwrap();
    std::os::unix::fs::symlink("other.txt", fixture.root.join("file.txt")).unwrap();
    assert!(
        fixture
            .request(Request::OpenBufferLease {
                lease: lease.clone()
            })
            .await
            .unwrap_err()
            .to_string()
            .contains("target changed")
    );
    assert!(fixture.buffers.active.read().await.is_empty());
    assert_eq!(
        state(
            fixture
                .request(Request::ReleaseBufferLease { lease })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
}

#[tokio::test]
async fn missing_active_owner_is_not_reported_as_successful_release() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    fixture
        .request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
    fixture.buffers.active.write().await.clear();
    assert!(
        fixture
            .request(Request::ReleaseBufferLease {
                lease: lease.clone()
            })
            .await
            .is_err()
    );
    assert_eq!(
        state(
            fixture
                .request(Request::QueryBufferLease { lease })
                .await
                .unwrap()
        ),
        LeaseState::Open
    );
}

struct Fixture {
    root: PathBuf,
    worktrees: Worktrees,
    buffers: Buffers,
}

impl Fixture {
    async fn new() -> Self {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let root =
            std::env::temp_dir().join(format!("cw-zed-lease-{:032x}", u128::from_be_bytes(nonce)));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file.txt"), "fixture\n").unwrap();
        let fixture = Self {
            root,
            worktrees: Arc::default(),
            buffers: Arc::new(BufferState::default()),
        };
        fixture.ready().await;
        fixture
    }

    async fn ready(&self) {
        ensure_worktree(self.root.clone(), true, false, &self.worktrees, None)
            .await
            .unwrap();
        self.worktrees
            .write()
            .await
            .get_mut(&self.root)
            .unwrap()
            .state = WorktreeState::Ready;
    }

    async fn prepare(&self) -> LeaseRef {
        let response = self
            .request(Request::PrepareBuffer {
                worktree: self.root.clone(),
                path: PathBuf::from("file.txt"),
            })
            .await
            .unwrap();
        let Response::BufferLease {
            lease,
            state: LeaseState::Prepared,
            ..
        } = response
        else {
            panic!("not prepared")
        };
        lease
    }

    async fn request(&self, request: Request) -> Result<Response> {
        respond(request, &self.worktrees, &self.buffers, None).await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

// Assertions deliberately consume their temporary protocol response.
#[allow(clippy::needless_pass_by_value)]
fn state(response: Response) -> LeaseState {
    let Response::BufferLease { state, .. } = response else {
        panic!("wrong response")
    };
    state
}

#[tokio::test]
async fn preparation_has_no_native_effect_and_closed_handles_never_reopen() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    assert!(fixture.buffers.active.read().await.is_empty());
    for _ in 0..2 {
        assert_eq!(
            state(
                fixture
                    .request(Request::ReleaseBufferLease {
                        lease: lease.clone()
                    })
                    .await
                    .unwrap()
            ),
            LeaseState::Released
        );
    }
    assert!(
        fixture
            .request(Request::OpenBufferLease {
                lease: lease.clone()
            })
            .await
            .is_err()
    );
    let next = fixture.prepare().await;
    assert_ne!(lease, next);
    assert_eq!(
        state(
            fixture
                .request(Request::QueryBufferLease { lease })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
}

#[tokio::test]
async fn lost_open_reply_can_be_observed_without_reopening() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    fixture
        .request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
    for request in [
        Request::QueryBufferLease {
            lease: lease.clone(),
        },
        Request::OpenBufferLease {
            lease: lease.clone(),
        },
    ] {
        assert_eq!(
            state(fixture.request(request).await.unwrap()),
            LeaseState::Open
        );
    }
    let all = fixture.buffers.active.read().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all.values().next().unwrap().lease_ids.len(), 1);
}

#[tokio::test]
async fn release_uses_original_buffer_after_file_and_worktree_rename() {
    let fixture = Fixture::new().await;
    let first = fixture.prepare().await;
    let second = fixture.prepare().await;
    for lease in [&first, &second] {
        fixture
            .request(Request::OpenBufferLease {
                lease: lease.clone(),
            })
            .await
            .unwrap();
    }
    std::fs::remove_file(fixture.root.join("file.txt")).unwrap();
    let moved = fixture.root.with_extension("moved");
    std::fs::rename(&fixture.root, &moved).unwrap();
    std::fs::create_dir(&fixture.root).unwrap();
    std::fs::write(fixture.root.join("file.txt"), "replacement\n").unwrap();
    assert_eq!(
        state(
            fixture
                .request(Request::ReleaseBufferLease { lease: first })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
    assert_eq!(
        fixture
            .buffers
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
    assert_eq!(
        state(
            fixture
                .request(Request::ReleaseBufferLease { lease: second })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
    assert!(fixture.buffers.active.read().await.is_empty());
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("file.txt")).unwrap(),
        "replacement\n"
    );
    std::fs::remove_dir(moved).unwrap();
}

#[tokio::test]
async fn legacy_ids_cannot_release_an_owned_buffer() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    fixture
        .request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
    fixture
        .request(Request::CloseBuffer {
            worktree: fixture.root.clone(),
            path: PathBuf::from("file.txt"),
            lease_id: lease.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(fixture.buffers.active.read().await.len(), 1);
    fixture
        .request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
    assert!(fixture.buffers.active.read().await.is_empty());
}

#[tokio::test]
async fn worktree_close_cannot_destroy_an_owned_open_buffer() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    fixture
        .request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
    fixture
        .request(Request::CloseWorktree {
            path: fixture.root.clone(),
        })
        .await
        .unwrap();
    assert_eq!(fixture.worktrees.read().await.len(), 1);
    fixture
        .request(Request::ReleaseBufferLease { lease })
        .await
        .unwrap();
    fixture
        .request(Request::CloseWorktree {
            path: fixture.root.clone(),
        })
        .await
        .unwrap();
    assert!(fixture.worktrees.read().await.is_empty());
}

#[tokio::test]
async fn prepared_worktree_aba_is_not_adopted() {
    let fixture = Fixture::new().await;
    let lease = fixture.prepare().await;
    fixture
        .request(Request::CloseWorktree {
            path: fixture.root.clone(),
        })
        .await
        .unwrap();
    fixture.ready().await;
    let error = fixture
        .request(Request::OpenBufferLease { lease })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("incarnation"));
    assert!(fixture.buffers.active.read().await.is_empty());
}

#[tokio::test]
async fn foreign_and_unissued_handles_are_not_restoration_authority() {
    let fixture = Fixture::new().await;
    let other = Fixture::new().await;
    let lease = fixture.prepare().await;
    other.prepare().await;
    assert!(
        other
            .request(Request::ReleaseBufferLease {
                lease: lease.clone()
            })
            .await
            .is_err()
    );
    for id in [
        "0000000000000000",
        "0000000000000002",
        "000000000000000A",
        "1",
        "not-a-handle",
    ] {
        assert!(
            fixture
                .request(Request::QueryBufferLease {
                    lease: LeaseRef {
                        instance: lease.instance.clone(),
                        id: id.to_owned()
                    },
                })
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn only_effect_free_preparations_expire_and_unknown_never_replays() {
    let fixture = Fixture::new().await;
    let prepared = fixture.prepare().await;
    let opened = fixture.prepare().await;
    let unknown = fixture.prepare().await;
    fixture
        .request(Request::OpenBufferLease {
            lease: opened.clone(),
        })
        .await
        .unwrap();
    {
        let mut registry = fixture.buffers.leases.lock().await;
        registry.slots.get_mut(&3).unwrap().state = Phase::Unknown;
        for slot in registry.slots.values_mut() {
            slot.prepared_at -= PREPARE_TTL;
        }
    }
    assert_eq!(
        state(
            fixture
                .request(Request::QueryBufferLease { lease: prepared })
                .await
                .unwrap()
        ),
        LeaseState::Released
    );
    assert_eq!(
        state(
            fixture
                .request(Request::QueryBufferLease { lease: opened })
                .await
                .unwrap()
        ),
        LeaseState::Open
    );
    for request in [
        Request::OpenBufferLease {
            lease: unknown.clone(),
        },
        Request::ReleaseBufferLease { lease: unknown },
    ] {
        assert_eq!(
            state(fixture.request(request).await.unwrap()),
            LeaseState::Unknown
        );
    }
    assert_eq!(fixture.buffers.active.read().await.len(), 1);
    assert_eq!(fixture.buffers.leases.lock().await.slots.len(), 2);
}

#[tokio::test]
async fn capacity_rejects_before_allocation_and_released_slots_are_reusable() {
    let fixture = Fixture::new().await;
    let mut leases = Vec::new();
    for _ in 0..MAX_LEASES {
        leases.push(fixture.prepare().await);
    }
    let prepare = || Request::PrepareBuffer {
        worktree: fixture.root.clone(),
        path: PathBuf::from("file.txt"),
    };
    assert!(
        fixture
            .request(prepare())
            .await
            .unwrap_err()
            .to_string()
            .contains("capacity")
    );
    assert_eq!(
        fixture.buffers.leases.lock().await.last_id,
        MAX_LEASES as u64
    );
    let oldest = leases.remove(0);
    fixture
        .request(Request::ReleaseBufferLease {
            lease: oldest.clone(),
        })
        .await
        .unwrap();
    let new = fixture.prepare().await;
    assert_ne!(new, oldest);
    assert!(
        fixture
            .request(Request::OpenBufferLease { lease: oldest })
            .await
            .is_err()
    );
    assert!(fixture.buffers.active.read().await.is_empty());
}

#[test]
fn lease_wire_is_closed_and_does_not_accept_retargeting_fields() {
    for input in [
        r#"{"type":"releaseBufferLease","lease":{"instance":"a","id":"1","path":"replacement"}}"#,
        r#"{"type":"openBufferLease","lease":{"instance":"a","id":"1"},"worktree":"replacement"}"#,
        r#"{"type":"releaseBufferLease","lease":{"instance":"a","id":1}}"#,
    ] {
        assert!(serde_json::from_str::<Request>(input).is_err());
    }
}

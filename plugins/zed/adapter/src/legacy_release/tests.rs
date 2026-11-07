use super::*;
use crate::native_close::tests::reply as native_reply;
use crate::sync_native::wire::{
    cowboy_buffer_sync_envelope::Payload, cowboy_close_buffers::Action as CloseAction,
    cowboy_close_buffers_response::Outcome,
};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn preparation_and_execution_refuse_an_unresolved_native_open() {
    let f = Fixture::new().await;
    let prepared = f.prepare().await;
    let pending = f.buffers.native_open.begin().unwrap();
    assert!(
        respond(
            Request::PrepareLegacyRelease {
                worktree: f.root.clone(),
                path: "file".into(),
                lease_id: "owner".into(),
            },
            &f.buffers,
            None
        )
        .await
        .is_err()
    );
    assert!(f.act(&prepared, Action::Execute).await.is_err());
    assert_eq!(
        state(&f.act(&prepared, Action::Query).await.unwrap()),
        State::Prepared
    );
    assert_eq!(f.buffers.active.read().await.len(), 1);
    pending.complete();
    assert_eq!(
        state(&f.act(&prepared, Action::Execute).await.unwrap()),
        State::Released
    );
}

struct Fixture {
    root: PathBuf,
    buffers: Buffers,
    trees: crate::Worktrees,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    async fn new() -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "cw-legacy-release-{:032x}",
            u128::from_be_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), "text").unwrap();
        let f = Self {
            root,
            buffers: Arc::default(),
            trees: Arc::default(),
        };
        crate::respond(
            Request::EnsureWorktree {
                path: f.root.clone(),
                trusted: true,
            },
            &f.trees,
            &f.buffers,
            None,
        )
        .await
        .unwrap();
        f.open().await;
        f
    }
    async fn open(&self) {
        crate::respond(
            Request::OpenBuffer {
                worktree: self.root.clone(),
                path: "file".into(),
                lease_id: "owner".into(),
            },
            &self.trees,
            &self.buffers,
            None,
        )
        .await
        .unwrap();
    }
    async fn prepare(&self) -> Reference {
        let Response::LegacyRelease {
            release,
            state: State::Prepared,
            ..
        } = respond(
            Request::PrepareLegacyRelease {
                worktree: self.root.clone(),
                path: "file".into(),
                lease_id: "owner".into(),
            },
            &self.buffers,
            None,
        )
        .await
        .unwrap()
        else {
            panic!("not prepared")
        };
        release
    }
    async fn act(&self, release: &Reference, action: Action) -> Result<Response> {
        respond(
            Request::LegacyRelease {
                release: release.clone(),
                action,
            },
            &self.buffers,
            None,
        )
        .await
    }
}
fn state(response: &Response) -> State {
    let Response::LegacyRelease { state, .. } = response else {
        panic!("wrong response")
    };
    *state
}

#[tokio::test]
async fn released_reference_never_closes_a_recreated_owner_and_retired_is_not_released() {
    let f = Fixture::new().await;
    let r = f.prepare().await;
    assert_eq!(
        state(&f.act(&r, Action::Query).await.unwrap()),
        State::Prepared
    );
    assert_eq!(
        state(&f.act(&r, Action::Execute).await.unwrap()),
        State::Released
    );
    f.open().await;
    for action in [Action::Execute, Action::Query] {
        assert_eq!(state(&f.act(&r, action).await.unwrap()), State::Released);
    }
    assert_eq!(f.buffers.active.read().await.len(), 1);
    assert_eq!(
        state(&f.act(&r, Action::Retire).await.unwrap()),
        State::Retired
    );
    assert_eq!(
        state(&f.act(&r, Action::Query).await.unwrap()),
        State::Retired
    );
    assert!(f.act(&r, Action::Execute).await.is_err());
    assert_eq!(f.buffers.active.read().await.len(), 1);
}

#[tokio::test]
async fn prepared_reference_rejects_replaced_pin_even_when_native_id_is_reused() {
    let f = Fixture::new().await;
    let r = f.prepare().await;
    crate::legacy_buffers::close_buffer(
        f.root.clone(),
        "file".into(),
        "owner".into(),
        &f.buffers,
        None,
    )
    .await
    .unwrap();
    f.open().await;
    assert!(f.act(&r, Action::Execute).await.is_err());
    assert_eq!(f.buffers.active.read().await.len(), 1);
    assert_eq!(
        state(&f.act(&r, Action::Query).await.unwrap()),
        State::Retired
    );
    let foreign = Reference {
        instance: "f".repeat(32),
        id: r.id.clone(),
    };
    assert!(f.act(&foreign, Action::Query).await.is_err());
}

#[tokio::test]
async fn lost_close_reply_queries_only_the_original_close_and_blocks_legacy_replay() {
    let f = Fixture::new().await;
    let r = f.prepare().await;
    let (transport, mut private) = crate::sync_native::Transport::new();
    let (zed, _outbound) = crate::coordinate_queries::fixture_with(transport).await;
    let task = {
        let buffers = f.buffers.clone();
        let zed = zed.clone();
        let release = r.clone();
        tokio::spawn(async move {
            respond(
                Request::LegacyRelease {
                    release,
                    action: Action::Execute,
                },
                &buffers,
                Some(&zed),
            )
            .await
        })
    };
    let probe = private.recv().await.unwrap();
    native_reply(&zed.sync, probe, Outcome::Supported);
    let closing = private.recv().await.unwrap();
    let Some(Payload::CloseRequest(original)) = closing.payload else {
        panic!("not close")
    };
    assert_eq!(original.action, CloseAction::Close as i32);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        crate::legacy_buffers::open_buffer(
            f.root.clone(),
            "file".into(),
            "owner".into(),
            &f.trees,
            &f.buffers,
            Some(&zed)
        )
        .await
        .is_err()
    );
    assert!(
        crate::legacy_buffers::close_buffer(
            f.root.clone(),
            "file".into(),
            "owner".into(),
            &f.buffers,
            Some(&zed)
        )
        .await
        .is_err()
    );
    assert!(f.act(&r, Action::Retire).await.is_err());
    assert_eq!(
        state(&f.act(&r, Action::Execute).await.unwrap()),
        State::Unknown
    );
    assert!(private.try_recv().is_err());
    for outcome in [Outcome::Unknown, Outcome::Closed] {
        let (result, ()) = tokio::join!(
            respond(
                Request::LegacyRelease {
                    release: r.clone(),
                    action: Action::Query
                },
                &f.buffers,
                Some(&zed)
            ),
            async {
                let envelope = private.recv().await.unwrap();
                let Some(Payload::CloseRequest(query)) = &envelope.payload else {
                    panic!("not query")
                };
                assert_eq!(query.action, CloseAction::Query as i32);
                assert_eq!(query.operation_id, original.operation_id);
                assert_eq!(query.instance, original.instance);
                assert_eq!(query.buffer_ids, original.buffer_ids);
                native_reply(&zed.sync, envelope, outcome);
            }
        );
        if outcome == Outcome::Unknown {
            assert!(result.is_err());
            assert_eq!(f.buffers.active.read().await.len(), 1);
        } else {
            assert_eq!(state(&result.unwrap()), State::Released);
            assert!(f.buffers.active.read().await.is_empty());
        }
    }
    assert_eq!(
        state(&f.act(&r, Action::Query).await.unwrap()),
        State::Released
    );
}

#[tokio::test]
async fn preparation_retries_reuse_the_original_reference_even_at_capacity() {
    let f = Fixture::new().await;
    let first = f.prepare().await;
    for _ in 0..CAPACITY {
        let retry = f.prepare().await;
        assert_eq!(retry.instance, first.instance);
        assert_eq!(retry.id, first.id);
    }
    {
        let mut registry = f.buffers.legacy_releases.lock().await;
        let legacy = f.buffers.legacy.lock().await;
        for id in 2..=CAPACITY as u64 {
            registry.entries.insert(
                id,
                Entry {
                    captured: legacy
                        .capture((f.root.clone(), "file".into(), "owner".into()))
                        .unwrap(),
                    state: State::Unknown,
                    buffer: None,
                },
            );
        }
        registry.last = CAPACITY as u64;
    }
    assert_eq!(f.prepare().await.id, first.id);
    assert_eq!(
        state(&f.act(&first, Action::Query).await.unwrap()),
        State::Prepared
    );
    assert_eq!(
        f.buffers.legacy_releases.lock().await.entries.len(),
        CAPACITY
    );
    assert_eq!(
        state(&f.act(&first, Action::Execute).await.unwrap()),
        State::Released
    );
    // Removing the original pin must not prune unknown or completed receipts.
    assert_eq!(
        state(&f.act(&first, Action::Query).await.unwrap()),
        State::Released
    );
    assert_eq!(
        f.buffers.legacy_releases.lock().await.entries.len(),
        CAPACITY
    );
    let unknown = Reference {
        instance: first.instance.clone(),
        id: format!("{:016x}", 2),
    };
    assert!(f.act(&unknown, Action::Retire).await.is_err());
    f.open().await;
    assert!(
        respond(
            Request::PrepareLegacyRelease {
                worktree: f.root.clone(),
                path: "file".into(),
                lease_id: "owner".into(),
            },
            &f.buffers,
            None
        )
        .await
        .is_err()
    );
}

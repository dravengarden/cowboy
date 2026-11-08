//! Synthetic ownership model; native journal acceptance is separate.
use super::*;
use std::sync::Arc;

struct Fixture {
    root: PathBuf,
    trees: Worktrees,
    buffers: Buffers,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    async fn new(interactive: bool) -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let root =
            std::env::temp_dir().join(format!("cw-acquire-{:032x}", u128::from_be_bytes(nonce)));
        tokio::fs::create_dir(&root).await.unwrap();
        tokio::fs::write(root.join("file"), "text").await.unwrap();
        let f = Self {
            root,
            trees: Arc::default(),
            buffers: Arc::default(),
        };
        f.ready_tree(interactive).await;
        f
    }
    async fn ready_tree(&self, interactive: bool) {
        crate::trust::ensure_worktree(
            self.root.clone(),
            true,
            interactive,
            &self.trees,
            &self.buffers,
            None,
        )
        .await
        .unwrap();
        self.trees.write().await.get_mut(&self.root).unwrap().state = crate::WorktreeState::Ready;
    }
    async fn prepare(&self, owner: &str) -> Reference {
        let response = respond(
            Request::PrepareAgentAcquisition {
                worktree: self.root.clone(),
                path: "file".into(),
                lease_id: owner.into(),
            },
            &self.trees,
            &self.buffers,
            None,
        )
        .await
        .unwrap();
        let Response::AgentAcquisition { acquisition, .. } = response else {
            panic!("not acquisition")
        };
        acquisition
    }
    async fn act(&self, acquisition: &Reference, action: Action) -> Result<State> {
        let response = respond(
            Request::AgentAcquisition {
                acquisition: acquisition.clone(),
                action,
            },
            &self.trees,
            &self.buffers,
            None,
        )
        .await?;
        let Response::AgentAcquisition { state, .. } = response else {
            panic!("not acquisition")
        };
        Ok(state)
    }
}

#[tokio::test]
async fn final_automatic_owner_retires_tree_but_shared_owner_keeps_it() {
    let f = Fixture::new(false).await;
    let first = f.prepare("first").await;
    assert_eq!(f.prepare("first").await.id, first.id);
    assert!(f.buffers.active.read().await.is_empty());
    assert_eq!(f.act(&first, Action::Execute).await.unwrap(), State::Ready);
    let second = f.prepare("second").await;
    assert_eq!(f.act(&second, Action::Execute).await.unwrap(), State::Ready);
    assert_eq!(
        f.act(&first, Action::Release).await.unwrap(),
        State::Released
    );
    assert_eq!(f.trees.read().await.len(), 1);
    assert_eq!(f.buffers.active.read().await.len(), 1);
    assert_eq!(
        f.act(&second, Action::Release).await.unwrap(),
        State::Released
    );
    assert!(f.trees.read().await.is_empty());
    assert!(f.buffers.active.read().await.is_empty());
    f.ready_tree(false).await;
    let later = f.prepare("later").await;
    assert_eq!(f.act(&later, Action::Execute).await.unwrap(), State::Ready);
    for action in [Action::Execute, Action::Query, Action::Release] {
        assert_eq!(f.act(&first, action).await.unwrap(), State::Released);
    }
    assert_eq!(f.buffers.active.read().await.len(), 1);
    assert_eq!(f.act(&first, Action::Retire).await.unwrap(), State::Retired);
    assert!(f.act(&first, Action::Release).await.is_err());
}

#[tokio::test]
async fn interactive_worktree_and_foreign_reference_are_preserved() {
    let f = Fixture::new(true).await;
    let reference = f.prepare("automatic").await;
    assert!(f.act(&reference, Action::Retire).await.is_err());
    assert_eq!(
        f.act(&reference, Action::Execute).await.unwrap(),
        State::Ready
    );
    let foreign = Reference {
        instance: "f".repeat(32),
        id: reference.id.clone(),
    };
    assert!(f.act(&foreign, Action::Release).await.is_err());
    assert_eq!(
        f.act(&reference, Action::Release).await.unwrap(),
        State::Released
    );
    assert!(f.buffers.active.read().await.is_empty());
    assert_eq!(f.trees.read().await[&f.root].leases, 1);
}

#[tokio::test]
async fn replacement_between_ensure_and_buffer_prepare_is_not_adopted() {
    let f = Fixture::new(false).await;
    let mut captured = None;
    crate::trust::ensure_captured(
        f.root.clone(),
        true,
        false,
        &f.trees,
        &f.buffers,
        None,
        &mut captured,
    )
    .await
    .unwrap();
    let captured = captured.unwrap();
    crate::close_worktree(f.root.clone(), &f.trees, &f.buffers, None)
        .await
        .unwrap();
    f.ready_tree(false).await;
    let Response::BufferLease { lease, .. } = crate::buffer_leases::respond(
        Request::PrepareBuffer {
            worktree: f.root.clone(),
            path: "file".into(),
        },
        &f.trees,
        &f.buffers,
        None,
    )
    .await
    .unwrap() else {
        panic!("not prepared")
    };
    assert!(
        f.buffers
            .leases
            .lock()
            .await
            .open_acquisition(lease.clone(), &captured, &f.trees, &f.buffers, None)
            .await
            .is_err()
    );
    assert!(f.buffers.active.read().await.is_empty());
    crate::buffer_leases::respond(
        Request::ReleaseBufferLease { lease },
        &f.trees,
        &f.buffers,
        None,
    )
    .await
    .unwrap();
    captured
        .release(&f.trees, &f.buffers, None, || panic!("must not dispatch"))
        .await
        .unwrap();
    assert_eq!(f.trees.read().await.len(), 1);
    let later = f.prepare("replacement-owner").await;
    assert_eq!(f.act(&later, Action::Execute).await.unwrap(), State::Ready);
}

#[tokio::test]
async fn preparation_is_idempotent_at_capacity_and_changed_root_does_not_acquire() {
    let f = Fixture::new(false).await;
    let reference = f.prepare("original").await;
    for id in 1..CAPACITY {
        f.prepare(&format!("owner-{id}")).await;
    }
    assert_eq!(f.prepare("original").await.id, reference.id);
    let old = f.root.with_extension("original");
    tokio::fs::rename(&f.root, &old).await.unwrap();
    tokio::fs::create_dir(&f.root).await.unwrap();
    tokio::fs::write(f.root.join("file"), "replacement")
        .await
        .unwrap();
    assert!(f.act(&reference, Action::Execute).await.is_err());
    assert_eq!(
        f.act(&reference, Action::Query).await.unwrap(),
        State::Acquiring
    );
    assert_eq!(
        f.act(&reference, Action::Release).await.unwrap(),
        State::Released
    );
    assert!(f.buffers.active.read().await.is_empty());
    assert_eq!(
        tokio::fs::read_to_string(f.root.join("file"))
            .await
            .unwrap(),
        "replacement"
    );
    tokio::fs::remove_dir_all(&old).await.unwrap();
}

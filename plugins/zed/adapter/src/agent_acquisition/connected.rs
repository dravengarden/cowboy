//! Actual native effects and discarded original responses; no simulated success.
use super::*;
use crate::worktree_recovery::Kind;
use std::{path::Path, sync::Arc};

struct Fixture<'a> {
    zed: &'a Zed,
    root: PathBuf,
    trees: Worktrees,
    buffers: Buffers,
}
impl Fixture<'_> {
    async fn prepare(&self, owner: &str) -> Reference {
        let Response::AgentAcquisition { acquisition, .. } = respond(
            Request::PrepareAgentAcquisition {
                worktree: self.root.clone(),
                path: "file.txt".into(),
                lease_id: owner.into(),
            },
            &self.trees,
            &self.buffers,
            Some(self.zed),
        )
        .await
        .unwrap() else {
            panic!("not prepared")
        };
        acquisition
    }
    async fn act(&self, acquisition: &Reference, action: Action) -> Result<State> {
        let Response::AgentAcquisition { state, .. } = respond(
            Request::AgentAcquisition {
                acquisition: acquisition.clone(),
                action,
            },
            &self.trees,
            &self.buffers,
            Some(self.zed),
        )
        .await?
        else {
            panic!("not acquisition")
        };
        Ok(state)
    }
    async fn absent(&self, id: u64) {
        assert!(
            self.zed
                .request(proto::envelope::Payload::OpenBufferByPath(
                    proto::OpenBufferByPath {
                        project_id: proto::REMOTE_SERVER_PROJECT_ID,
                        worktree_id: id,
                        path: "file.txt".into(),
                    }
                ))
                .await
                .is_err(),
            "retired native worktree can still open buffers"
        );
    }
}

pub(crate) async fn exercise(zed: &Zed, root: &Path) {
    for kind in [Kind::Add, Kind::Trust, Kind::Remove] {
        let workspace = root.join(format!("acquisition-{kind:?}"));
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("file.txt"), "original\n").unwrap();
        let f = Fixture {
            zed,
            root: workspace,
            trees: Arc::default(),
            buffers: Arc::default(),
        };
        let reference = f.prepare("automatic").await;
        zed.sync.worktree_fault.arm(kind);
        if kind == Kind::Remove {
            assert_eq!(
                f.act(&reference, Action::Execute).await.unwrap(),
                State::Ready
            );
            let id = f.trees.read().await[&f.root].remote_id;
            assert!(f.act(&reference, Action::Release).await.is_err());
            assert!(zed.sync.worktree_fault.dropped());
            assert_eq!(
                f.act(&reference, Action::Query).await.unwrap(),
                State::Released
            );
            f.absent(id).await;
        } else {
            assert!(f.act(&reference, Action::Execute).await.is_err());
            assert!(zed.sync.worktree_fault.dropped());
            // Query may wait for actual asynchronous Add registration, but may
            // never continue Trust/Open on behalf of the abandoned Execute.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if let Ok(state) = f.act(&reference, Action::Query).await {
                    assert_eq!(state, State::Acquiring);
                    break;
                }
                assert!(tokio::time::Instant::now() < deadline);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert!(f.buffers.active.read().await.is_empty());
            let id = f.trees.read().await[&f.root].remote_id;
            assert_ne!(id, 0);
            assert_eq!(
                f.act(&reference, Action::Release).await.unwrap(),
                State::Released
            );
            f.absent(id).await;
        }
        assert!(f.trees.read().await.is_empty());
        assert!(f.buffers.active.read().await.is_empty());
        assert_eq!(
            f.act(&reference, Action::Release).await.unwrap(),
            State::Released
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join("file.txt")).unwrap(),
            "original\n"
        );
    }
    println!(
        "native worktree acquisition: real Add/Trust/Remove reply loss, original query recovery, registration before cleanup and retired worktree rejection passed"
    );
}

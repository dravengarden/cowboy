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

async fn observe(
    f: &Fixture<'_>,
    refresh: bool,
) -> (
    crate::source_snapshot::State,
    Option<crate::agent_refresh::Outcome>,
) {
    let Response::BufferLanguage {
        source_state,
        refresh,
        ..
    } = crate::language_observation::buffer_language(
        f.root.clone(),
        "file.txt".into(),
        refresh.then_some(crate::agent_refresh::Refresh::AgentOwned),
        &f.buffers,
        Some(f.zed),
    )
    .await
    .unwrap()
    else {
        panic!("not a language observation")
    };
    (source_state, refresh)
}

/// The actual native reload of an agent-only mirror, and its refusal while
/// another owner shares the buffer.
pub(crate) async fn refresh(zed: &Zed, root: &Path) {
    use crate::agent_refresh::Outcome;
    use crate::source_snapshot::State as Source;
    let workspace = root.join("acquisition-refresh");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("file.txt"), "original\n").unwrap();
    let f = Fixture {
        zed,
        root: workspace,
        trees: Arc::default(),
        buffers: Arc::default(),
    };
    let reference = f.prepare("automatic-refresh").await;
    assert_eq!(
        f.act(&reference, Action::Execute).await.unwrap(),
        State::Ready
    );
    assert_eq!(
        observe(&f, true).await,
        (Source::Matching, Some(Outcome::NotNeeded))
    );
    std::fs::write(f.root.join("file.txt"), "agent edit\n").unwrap();
    assert_eq!(observe(&f, false).await, (Source::Changed, None));
    // A second owner shares the mirror: no reload, text unchanged.
    let legacy = crate::open_buffer(
        f.root.clone(),
        "file.txt".into(),
        "interactive-tab".into(),
        &f.trees,
        &f.buffers,
        Some(zed),
    )
    .await;
    legacy.expect("a second legacy owner joins the same native buffer");
    {
        assert_eq!(
            observe(&f, true).await,
            (Source::Changed, Some(Outcome::Refused))
        );
        crate::close_buffer(
            f.root.clone(),
            "file.txt".into(),
            "interactive-tab".into(),
            &f.buffers,
            Some(zed),
        )
        .await
        .unwrap();
    }
    assert_eq!(observe(&f, true).await.1, Some(Outcome::Reloaded));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while observe(&f, false).await.0 != Source::Matching {
        assert!(
            tokio::time::Instant::now() < deadline,
            "reloaded mirror never matched"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(observe(&f, true).await.1, Some(Outcome::NotNeeded));
    assert_eq!(
        f.act(&reference, Action::Release).await.unwrap(),
        State::Released
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("file.txt")).unwrap(),
        "agent edit\n",
        "refresh must never write the source"
    );
    println!(
        "agent-owned native reload: exclusive-owner reload, shared-owner refusal and no source write passed"
    );
}

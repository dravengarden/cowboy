//! Called by the existing PID/network-isolated, immutable private-server gate.
use super::*;
use crate::{Request, Worktrees, respond};
use std::path::Path;
use std::sync::Arc;

struct Peer<'a> {
    zed: &'a Zed,
    root: &'a Path,
    worktrees: Worktrees,
    buffers: Buffers,
}

const FILE: &str = "owned-sync.txt";
const NEW: &str = "native synchronized汉字\n";

impl Peer<'_> {
    async fn request(&self, request: Request) -> Result<Response> {
        respond(request, &self.worktrees, &self.buffers, Some(self.zed)).await
    }

    async fn owner(&self) -> buffer_leases::LeaseRef {
        let Response::BufferLease { lease, .. } = self
            .request(Request::PrepareBuffer {
                worktree: self.root.to_path_buf(),
                path: FILE.into(),
            })
            .await
            .unwrap()
        else {
            panic!("not prepared")
        };
        self.request(Request::OpenBufferLease {
            lease: lease.clone(),
        })
        .await
        .unwrap();
        lease
    }

    async fn prepare(&self) -> (buffer_leases::LeaseRef, OperationRef) {
        let first = self.owner().await;
        let second = self.owner().await;
        let sync = || Request::PrepareBufferSync {
            lease: first.clone(),
            purpose: Purpose::RefreshFromDisk,
            content: crate::coordinates::Mirror::new(1, NEW)
                .unwrap()
                .content()
                .clone(),
        };
        assert!(self.request(sync()).await.is_err());
        self.request(Request::ReleaseBufferLease { lease: second })
            .await
            .unwrap();
        tokio::fs::write(self.root.join(FILE), NEW).await.unwrap();
        let Response::BufferSync {
            operation,
            state: State::Prepared,
            ..
        } = self.request(sync()).await.unwrap()
        else {
            panic!("not prepared")
        };
        assert!(
            self.request(Request::ReleaseBufferLease {
                lease: first.clone()
            })
            .await
            .is_err()
        );
        assert!(
            self.request(Request::OpenBuffer {
                worktree: self.root.to_path_buf(),
                path: FILE.into(),
                lease_id: "interloper".into()
            })
            .await
            .is_err()
        );
        (first, operation)
    }

    async fn act(&self, operation: &OperationRef, action: Action) -> State {
        let Response::BufferSync { state, .. } = self
            .request(Request::BufferSync {
                operation: operation.clone(),
                action,
            })
            .await
            .unwrap()
        else {
            panic!("not sync")
        };
        state
    }

    async fn applied(&self, operation: &OperationRef) -> Content {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match self.act(operation, Action::Query).await {
                    State::Pending | State::Unknown => {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    State::Applied { content, .. } => break content,
                    other => panic!("native owner effect did not apply: {other:?}"),
                }
            }
        })
        .await
        .unwrap()
    }

    async fn mirror(&self, content: &Content) {
        let native = self
            .buffers
            .active
            .read()
            .await
            .values()
            .next()
            .unwrap()
            .remote_id;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self
                    .zed
                    .diagnostics
                    .lock()
                    .unwrap()
                    .match_content(native, content)
                    .ok()
                    .flatten()
                    .is_some()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }

    async fn finish(&self, owner: buffer_leases::LeaseRef, operation: &OperationRef) {
        self.act(operation, Action::Apply).await;
        let applied = self.applied(operation).await;
        self.mirror(&applied).await;
        let before = self.zed.next_message_id.load(crate::Ordering::Relaxed);
        for _ in 0..2 {
            assert!(matches!(
                self.act(operation, Action::Apply).await,
                State::Applied { .. }
            ));
        }
        assert_eq!(
            self.zed.next_message_id.load(crate::Ordering::Relaxed),
            before
        );
        assert_eq!(
            tokio::fs::read_to_string(self.root.join(FILE))
                .await
                .unwrap(),
            NEW
        );
        let read = self
            .request(Request::ReadBufferLease {
                lease: owner.clone(),
                request: buffer_leases::ReadRequest::Content {
                    content: applied,
                    query: crate::content_reads::Query::Symbols {},
                },
            })
            .await
            .unwrap();
        assert!(matches!(
            read,
            Response::BufferLeaseRead {
                result: buffer_leases::ReadOutput::Content {
                    result: crate::content_reads::Output::Observed { .. },
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            self.act(operation, Action::Retire).await,
            State::Retired
        ));
        tokio::fs::remove_file(self.root.join(FILE)).await.unwrap();
        self.request(Request::ReleaseBufferLease { lease: owner })
            .await
            .unwrap();
        assert!(self.buffers.active.read().await.is_empty());
    }
}

pub(crate) async fn exercise(zed: &Zed, workspace: &Path) {
    let peer = Peer {
        zed,
        root: workspace,
        worktrees: Arc::default(),
        buffers: Arc::default(),
    };
    tokio::fs::write(workspace.join(FILE), "native original🙂\n")
        .await
        .unwrap();
    peer.request(Request::OpenWorktree {
        path: workspace.to_path_buf(),
        trusted: true,
    })
    .await
    .unwrap();
    let (owner, operation) = peer.prepare().await;
    peer.finish(owner, &operation).await;
    println!(
        "adapter exclusive owner, actual native sync, original-content read and no-replay checks passed"
    );
}

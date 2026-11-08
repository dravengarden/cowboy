use super::*;
use crate::buffer_leases::{LeaseRef, LeaseState};
use std::os::unix::fs::MetadataExt;

#[derive(PartialEq, Eq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
}
impl Identity {
    pub(super) async fn read(root: &std::path::Path) -> Result<Self> {
        let metadata = tokio::fs::symlink_metadata(root).await?;
        ensure!(
            metadata.is_dir(),
            "acquisition root is no longer a directory"
        );
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

pub(super) struct Entry {
    pub(super) requested: (PathBuf, PathBuf),
    pub(super) owner: String,
    root: PathBuf,
    file: PathBuf,
    identity: Identity,
    pub(super) state: State,
    worktree: Option<crate::trust::Capture>,
    lease: Option<LeaseRef>,
    buffer_released: bool,
    buffer: Option<Box<serde_json::Value>>,
}

impl Entry {
    pub(super) fn new(
        requested: (PathBuf, PathBuf),
        owner: String,
        root: PathBuf,
        file: PathBuf,
        identity: Identity,
    ) -> Self {
        Self {
            requested,
            owner,
            root,
            file,
            identity,
            state: State::Prepared,
            worktree: None,
            lease: None,
            buffer_released: false,
            buffer: None,
        }
    }

    pub(super) fn reply(&self, reference: Reference) -> Response {
        super::reply(
            reference,
            self.state,
            if self.state == State::Ready {
                self.buffer.clone()
            } else {
                None
            },
        )
    }

    pub(super) async fn execute(
        &mut self,
        worktrees: &Worktrees,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<()> {
        // This transition precedes every await, including effect-free validation.
        self.state = State::Acquiring;
        ensure!(
            Identity::read(&self.root).await? == self.identity,
            "prepared acquisition root changed"
        );
        let ready = crate::trust::ensure_captured(
            self.root.clone(),
            true,
            false,
            worktrees,
            buffers,
            zed,
            &mut self.worktree,
        )
        .await?;
        ensure!(
            matches!(
                ready,
                Response::Worktree {
                    state: crate::WorktreeState::Ready,
                    ..
                }
            ),
            "original acquisition worktree is not ready"
        );
        ensure!(
            Identity::read(&self.root).await? == self.identity,
            "acquisition root changed while opening"
        );
        let prepared = crate::buffer_leases::respond(
            Request::PrepareBuffer {
                worktree: self.root.clone(),
                path: self.file.clone(),
            },
            worktrees,
            buffers,
            zed,
        )
        .await?;
        let Response::BufferLease {
            lease,
            state: LeaseState::Prepared,
            ..
        } = prepared
        else {
            anyhow::bail!("acquisition buffer was not prepared");
        };
        self.lease = Some(lease.clone());
        let opened = buffers
            .leases
            .lock()
            .await
            .open_acquisition(
                lease.clone(),
                self.worktree
                    .as_ref()
                    .context("original acquisition worktree missing")?,
                worktrees,
                buffers,
                zed,
            )
            .await?;
        ensure!(
            matches!(
                opened,
                Response::BufferLease {
                    state: LeaseState::Open,
                    ..
                }
            ),
            "original acquisition buffer remains unconfirmed"
        );
        self.ready(&lease, buffers).await
    }

    async fn ready(&mut self, lease: &LeaseRef, buffers: &Buffers) -> Result<()> {
        let result = buffers
            .leases
            .lock()
            .await
            .acquisition_result(lease, buffers)
            .await?;
        self.buffer = Some(Box::new(serde_json::to_value(result)?));
        self.state = State::Ready;
        Ok(())
    }

    pub(super) async fn release(
        &mut self,
        worktrees: &Worktrees,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<()> {
        self.state = State::ReleasePrepared;
        if !self.buffer_released {
            if let Some(lease) = self.lease.clone() {
                let result = buffers
                    .leases
                    .lock()
                    .await
                    .release_acquisition(lease, buffers, zed, || self.state = State::ReleaseUnknown)
                    .await?;
                ensure!(
                    matches!(
                        result,
                        Response::BufferLease {
                            state: LeaseState::Released,
                            ..
                        }
                    ),
                    "original acquisition buffer cleanup remains unconfirmed"
                );
            }
            self.buffer_released = true;
            // A future continuation may explicitly start the next cleanup step.
            self.state = State::ReleasePrepared;
        }
        if let Some(worktree) = &self.worktree {
            worktree
                .release(worktrees, buffers, zed, || {
                    self.state = State::ReleaseUnknown;
                })
                .await?;
        }
        self.state = State::Released;
        self.buffer = None;
        Ok(())
    }

    pub(super) async fn observe(
        &mut self,
        worktrees: &Worktrees,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<()> {
        match self.state {
            State::Acquiring => {
                if let Some(worktree) = &self.worktree {
                    worktree.observe(worktrees, zed).await?;
                }
                if let Some(lease) = self.lease.clone() {
                    let observed = crate::buffer_leases::respond(
                        Request::QueryBufferLease {
                            lease: lease.clone(),
                        },
                        worktrees,
                        buffers,
                        zed,
                    )
                    .await?;
                    if matches!(
                        observed,
                        Response::BufferLease {
                            state: LeaseState::Open,
                            ..
                        }
                    ) {
                        self.ready(&lease, buffers).await?;
                    }
                }
            }
            State::ReleaseUnknown => {
                if !self.buffer_released {
                    let lease = self
                        .lease
                        .clone()
                        .context("original acquisition buffer reference missing")?;
                    let observed = crate::buffer_leases::respond(
                        Request::QueryBufferLease { lease },
                        worktrees,
                        buffers,
                        zed,
                    )
                    .await?;
                    if matches!(
                        observed,
                        Response::BufferLease {
                            state: LeaseState::Released,
                            ..
                        }
                    ) {
                        self.buffer_released = true;
                        self.state = State::ReleasePrepared;
                    }
                } else if let Some(worktree) = &self.worktree {
                    self.state = if worktree.observe(worktrees, zed).await? {
                        State::Released
                    } else {
                        State::ReleasePrepared
                    };
                } else {
                    self.state = State::Released;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

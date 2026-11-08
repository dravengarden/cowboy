//! One native worktree request. Observing it never repeats an effect.
use crate::{ZedRuntime, sync_native::wire};
use anyhow::{Context as _, Result, ensure};
use std::sync::{Arc, Weak};
use wire::cowboy_buffer_sync_envelope::Payload;
pub(super) use wire::cowboy_worktree_attempt_response::Kind;
use wire::cowboy_worktree_attempt_response::Outcome;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Observation {
    NoEffect,
    Completed(u64),
    Pending,
    Unknown,
}

#[derive(Debug)]
pub(super) struct Operation {
    kind: Kind,
    target: Option<u64>,
    message: u32,
    started: bool,
    transport: Option<Weak<crate::sync_native::Transport>>,
}

impl Operation {
    pub(super) fn new(kind: Kind, target: Option<u64>) -> Result<Self> {
        ensure!(
            kind != Kind::Unspecified
                && (kind == Kind::Add) == target.is_none()
                && target != Some(0),
            "invalid native worktree operation"
        );
        Ok(Self {
            kind,
            target,
            message: 0,
            started: false,
            transport: None,
        })
    }

    /// Save this operation before entering the future; cancellation retains it.
    pub(super) async fn execute(
        &mut self,
        zed: &ZedRuntime,
        payload: proto::envelope::Payload,
    ) -> Result<Observation> {
        ensure!(
            !self.started,
            "original worktree operation was already attempted"
        );
        self.check_request(&payload)?;
        self.started = true;
        self.transport = Some(Arc::downgrade(&zed.sync));
        zed.request_with(payload, |message| {
            self.message = message;
            #[cfg(test)]
            zed.sync.worktree_fault.dispatched(self.kind, message);
        })
        .await?;
        // In particular, AddWorktreeResponse precedes store registration.
        self.observe(zed).await
    }

    fn check_request(&self, payload: &proto::envelope::Payload) -> Result<()> {
        use proto::envelope::Payload as Native;
        let valid = match (self.kind, payload) {
            (Kind::Add, Native::AddWorktree(p)) => p.project_id == proto::REMOTE_SERVER_PROJECT_ID,
            (Kind::Remove, Native::RemoveWorktree(p)) => Some(p.worktree_id) == self.target,
            (Kind::Restrict, Native::RestrictWorktrees(p)) => {
                p.project_id == proto::REMOTE_SERVER_PROJECT_ID
                    && p.worktree_ids.as_slice() == [self.target.unwrap()]
            }
            (Kind::Trust, Native::TrustWorktrees(p)) => {
                p.project_id == proto::REMOTE_SERVER_PROJECT_ID
                    && matches!(p.trusted_paths.as_slice(), [path] if matches!(&path.content, Some(proto::path_trust::Content::WorktreeId(id)) if Some(*id) == self.target))
            }
            _ => false,
        };
        ensure!(valid, "native worktree request changed");
        Ok(())
    }

    pub(super) async fn observe(&self, zed: &ZedRuntime) -> Result<Observation> {
        if self.message == 0 {
            return Ok(Observation::NoEffect);
        }
        ensure!(
            self.transport
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|transport| Arc::ptr_eq(&transport, &zed.sync)),
            "original native worktree connection changed"
        );
        let barrier = zed
            .request(proto::envelope::Payload::Ping(proto::Ping {}))
            .await?;
        ensure!(
            matches!(barrier.payload, Some(proto::envelope::Payload::Ack(_))),
            "native worktree barrier unavailable"
        );
        let reply = query(zed, self.message).await?;
        self.decode(&reply)
    }

    fn decode(&self, reply: &wire::CowboyWorktreeAttemptResponse) -> Result<Observation> {
        ensure!(reply.protocol == 1, "native worktree protocol changed");
        match Outcome::from_i32(reply.outcome).context("invalid native worktree outcome")? {
            Outcome::Unknown | Outcome::Forgotten => {
                ensure!(
                    reply.kind == Kind::Unspecified as i32 && reply.worktree_id == 0,
                    "invalid absent worktree outcome"
                );
                // Ping and journal lookup do not prove an earlier async
                // handler has started. An absent entry is not a no-effect
                // receipt, even on the same connection.
                Ok(Observation::Unknown)
            }
            Outcome::Pending | Outcome::Uncertain => {
                ensure!(
                    reply.kind == self.kind as i32 && reply.worktree_id == 0,
                    "original worktree operation changed"
                );
                Ok(if reply.outcome == Outcome::Pending as i32 {
                    Observation::Pending
                } else {
                    Observation::Unknown
                })
            }
            Outcome::Completed => {
                ensure!(
                    reply.kind == self.kind as i32
                        && reply.worktree_id != 0
                        && self.target.is_none_or(|target| target == reply.worktree_id),
                    "original worktree result changed"
                );
                Ok(Observation::Completed(reply.worktree_id))
            }
            _ => anyhow::bail!("native worktree result unavailable"),
        }
    }
}

async fn query(zed: &ZedRuntime, message: u32) -> Result<wire::CowboyWorktreeAttemptResponse> {
    let reply = zed
        .sync
        .exchange(
            zed,
            Payload::WorktreeAttemptRequest(wire::CowboyWorktreeAttempt {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                protocol: 1,
                message_id: message,
            }),
        )
        .await?;
    let Payload::WorktreeAttemptResponse(reply) = reply else {
        anyhow::bail!("wrong native worktree response");
    };
    Ok(reply)
}

pub(super) async fn support(zed: &ZedRuntime) -> Result<()> {
    let reply = query(zed, 0).await?;
    ensure!(
        reply.protocol == 1
            && reply.kind == Kind::Unspecified as i32
            && reply.worktree_id == 0
            && reply.outcome == Outcome::Supported as i32,
        "original worktree recovery unavailable"
    );
    Ok(())
}

#[cfg(test)]
pub(crate) mod fault;
#[cfg(test)]
mod tests;

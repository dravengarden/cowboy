//! Original-process release references for automatic holdings. Preparation
//! precedes Close, uncertainty only queries the saved reference, and receipt
//! retirement keeps a bounded permit until the original peer confirms it.
use super::{CodeRuntimeHost, RunningCodeRuntime, WorktreeRoute, exchange};
use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const CAPACITY: usize = 1024;
const BATCH: usize = 4;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    instance: String,
    id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum State {
    Prepared,
    Unknown,
    Released,
    Retired,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(rename = "type")]
    kind: String,
    api_version: u8,
    release: Reference,
    state: State,
    buffer: Option<Value>,
}

impl Reply {
    fn parse(value: Value, expected: Option<&Reference>) -> Result<Self> {
        let reply: Self = serde_json::from_value(value).context("invalid legacy release reply")?;
        ensure!(
            reply.kind == "legacyRelease"
                && reply.api_version == 1
                && reply.release.instance.len() == 32
                && reply.release.id.len() == 16
                && reply
                    .release
                    .instance
                    .bytes()
                    .chain(reply.release.id.bytes())
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                && u64::from_str_radix(&reply.release.id, 16)? != 0
                && expected.is_none_or(|expected| expected == &reply.release)
                && (reply.state == State::Released) == reply.buffer.is_some(),
            "original legacy release reply changed"
        );
        Ok(reply)
    }
}

pub(super) struct Attempt {
    reference: Option<Reference>,
    submitted: bool,
    permit: OwnedSemaphorePermit,
}

struct Retirement {
    runtime: Arc<RunningCodeRuntime>,
    reference: Reference,
    _permit: OwnedSemaphorePermit,
}

struct Retiring<'a> {
    queue: &'a Mutex<VecDeque<Retirement>>,
    item: Option<Retirement>,
}

impl Drop for Retiring<'_> {
    fn drop(&mut self) {
        if let Some(item) = self.item.take() {
            self.queue.lock().push_back(item);
        }
    }
}

pub(super) struct Retirements {
    permits: Arc<Semaphore>,
    pending: Mutex<VecDeque<Retirement>>,
}

impl Default for Retirements {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(CAPACITY)),
            pending: Mutex::default(),
        }
    }
}

async fn request(socket: &Path, payload: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(2), exchange(socket, payload))
        .await
        .context("original legacy release observation timed out")?
}

impl Retirements {
    pub(super) async fn observe(&self) {
        let count = self.pending.lock().len().min(BATCH);
        for _ in 0..count {
            let Some(item) = self.pending.lock().pop_front() else {
                break;
            };
            let mut guard = Retiring {
                queue: &self.pending,
                item: Some(item),
            };
            let item = guard.item.as_ref().unwrap();
            // Only completed buffer releases enter this queue. An exited
            // adapter has lost this in-memory receipt; dropping its reference
            // still leaves descendant cleanup in the process retirement pool.
            if matches!(item.runtime.is_running(), Ok(false)) {
                guard.item.take();
                continue;
            }
            let result = request(
                &item.runtime.socket,
                &json!({"type":"legacyRelease", "release":item.reference, "action":"retire"}),
            )
            .await
            .and_then(|value| Reply::parse(value, Some(&item.reference)));
            if matches!(result, Ok(reply) if reply.state == State::Retired) {
                guard.item.take();
            }
        }
    }
}

impl CodeRuntimeHost {
    /// None preserves the old one-use Close for older Plugin generations.
    /// A retained attempt can never fall back or select another generation.
    pub(super) async fn release_agent(
        &self,
        route: &mut WorktreeRoute,
        runtime: &Arc<RunningCodeRuntime>,
        payload: &Value,
    ) -> Result<Option<Value>> {
        if let Some(response) = self.release_acquisition(route, runtime, payload).await? {
            return Ok(Some(response));
        }
        let key = WorktreeRoute::buffer_key(payload)?;
        let Some(held) = route.agent_buffers.get_mut(&key) else {
            return Ok(None);
        };
        if !held.recoverable_release()? {
            return Ok(None);
        }
        held.check_worktree(payload)?;
        if held.release.is_none() {
            ensure!(
                !held.releasing,
                "original automatic close remains unconfirmed"
            );
            let supported = request(&runtime.socket, &json!({"type":"legacyReleaseSupport"})).await;
            if !matches!(supported, Ok(ref value) if value == &json!({"type":"legacyReleaseSupport","api_version":1,"protocol":1}))
            {
                return Ok(None);
            }
            let permit = Arc::clone(&self.agent_retirements.permits).try_acquire_owned()?;
            held.release = Some(Attempt {
                reference: None,
                submitted: false,
                permit,
            });
            held.releasing = true;
        }
        let attempt = held.release.as_mut().expect("retained before preparation");
        if attempt.reference.is_none() {
            let prepared = request(&runtime.socket, &json!({"type":"prepareLegacyRelease", "worktree":payload["worktree"], "path":payload["path"], "leaseId":payload["leaseId"]})).await?;
            let prepared = Reply::parse(prepared, None)?;
            ensure!(
                prepared.state == State::Prepared,
                "legacy release was not prepared"
            );
            attempt.reference = Some(prepared.release);
        }
        let reference = attempt.reference.as_ref().unwrap();
        let action = if attempt.submitted {
            "query"
        } else {
            "execute"
        };
        attempt.submitted = true;
        let observed = request(
            &runtime.socket,
            &json!({"type":"legacyRelease", "release":reference, "action":action}),
        )
        .await?;
        let observed = Reply::parse(observed, Some(reference))?;
        if observed.state == State::Prepared {
            // This same preparation proves Close was never admitted. Continue
            // the original operation later, not a replacement operation ID.
            attempt.submitted = false;
        }
        ensure!(
            observed.state == State::Released,
            "original automatic close remains unconfirmed"
        );
        let buffer = observed.buffer.unwrap();
        // Validate before taking the bounded retirement permit from its owner.
        ensure!(
            buffer["type"] == "buffer"
                && buffer["api_version"] == 1
                && buffer["leases"].as_u64().is_some(),
            "invalid original buffer release result"
        );
        let attempt = held.release.take().unwrap();
        let idle = route.observe(payload, &buffer)?;
        self.agent_retirements.pending.lock().push_back(Retirement {
            runtime: Arc::clone(runtime),
            reference: attempt.reference.unwrap(),
            _permit: attempt.permit,
        });
        if idle {
            *route = WorktreeRoute::default();
        }
        Ok(Some(buffer))
    }
}

#[cfg(test)]
pub(super) mod tests;

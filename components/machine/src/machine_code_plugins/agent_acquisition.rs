//! Original-process automatic acquisition prepared before worktree effects.
use super::{CodeRuntimeHost, RunningCodeRuntime, WorktreeRoute, exchange};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::OwnedSemaphorePermit;

mod retirement;
#[cfg(test)]
pub(crate) mod tests;
pub(super) use retirement::Retirements;

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
    Acquiring,
    Ready,
    ReleasePrepared,
    ReleaseUnknown,
    Released,
    Retired,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(rename = "type")]
    kind: String,
    api_version: u8,
    acquisition: Reference,
    state: State,
    buffer: Option<Value>,
}

impl Reply {
    fn parse(value: Value, expected: Option<&Reference>) -> Result<Self> {
        let reply: Self =
            serde_json::from_value(value).context("invalid original acquisition reply")?;
        ensure!(
            reply.kind == "agentAcquisition"
                && reply.api_version == 1
                && reply.acquisition.instance.len() == 32
                && reply.acquisition.id.len() == 16
                && reply
                    .acquisition
                    .instance
                    .bytes()
                    .chain(reply.acquisition.id.bytes())
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                && u64::from_str_radix(&reply.acquisition.id, 16)? != 0
                && expected.is_none_or(|expected| expected == &reply.acquisition)
                && (reply.state == State::Ready) == reply.buffer.is_some(),
            "original acquisition reply changed"
        );
        Ok(reply)
    }
}

pub(super) struct Attempt {
    reference: Option<Reference>,
    submitted: bool,
    pub(super) releasing: bool,
    release_submitted: bool,
    permit: OwnedSemaphorePermit,
}

async fn request(socket: &Path, payload: &Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(2), exchange(socket, payload))
        .await
        .context("original acquisition observation timed out")?
}

pub(super) async fn supported(socket: &Path) -> Result<bool> {
    match request(socket, &json!({"type":"agentAcquisitionSupport"})).await {
        Ok(reply) => {
            ensure!(
                reply == json!({"type":"agentAcquisitionSupport","api_version":1,"protocol":1}),
                "invalid automatic acquisition support reply"
            );
            Ok(true)
        }
        // Only an explicit old-codec refusal admits the existing legacy path.
        Err(error)
            if error.to_string().starts_with(
                "code runtime rejected request: unknown variant `agentAcquisitionSupport`",
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

impl Attempt {
    async fn prepare(&mut self, socket: &Path, payload: &Value) -> Result<()> {
        if self.reference.is_some() {
            return Ok(());
        }
        let reply = request(socket, &json!({"type":"prepareAgentAcquisition","worktree":payload["worktree"],"path":payload["path"],"leaseId":payload["leaseId"]})).await?;
        let reply = Reply::parse(reply, None)?;
        ensure!(
            reply.state == State::Prepared,
            "original acquisition was not prepared"
        );
        self.reference = Some(reply.acquisition);
        Ok(())
    }
}

impl CodeRuntimeHost {
    pub(super) fn reserve_acquisition(&self) -> Result<Attempt> {
        Ok(Attempt {
            reference: None,
            submitted: false,
            releasing: false,
            release_submitted: false,
            permit: self.agent_acquisitions.reserve()?,
        })
    }

    pub(super) async fn acquire_prepared(
        &self,
        route: &mut WorktreeRoute,
        socket: &Path,
        payload: &Value,
    ) -> Result<Value> {
        let key = WorktreeRoute::buffer_key(payload)?;
        let held = route
            .agent_buffers
            .get_mut(&key)
            .context("automatic owner was not reserved")?;
        let attempt = held
            .acquisition
            .as_mut()
            .context("acquisition capacity was not reserved")?;
        ensure!(
            !attempt.releasing,
            "original acquisition cleanup is pending"
        );
        attempt.prepare(socket, payload).await?;
        let reference = attempt.reference.as_ref().unwrap();
        let action = if attempt.submitted {
            "query"
        } else {
            "execute"
        };
        attempt.submitted = true;
        let payload = json!({"type":"agentAcquisition","acquisition":reference,"action":action});
        let observed = if action == "execute" {
            exchange(socket, &payload).await?
        } else {
            request(socket, &payload).await?
        };
        let reply = Reply::parse(observed, Some(reference))?;
        if reply.state == State::Prepared {
            attempt.submitted = false;
        }
        ensure!(
            reply.state == State::Ready,
            "original automatic acquisition remains unconfirmed"
        );
        let buffer = reply.buffer.unwrap();
        ensure!(
            buffer["type"] == "buffer"
                && buffer["api_version"] == 1
                && buffer["leases"].as_u64().is_some_and(|n| n > 0)
                && buffer["buffer_id"].as_u64().is_some_and(|n| n > 0),
            "invalid original acquisition buffer result"
        );
        Ok(buffer)
    }

    pub(super) async fn release_acquisition(
        &self,
        route: &mut WorktreeRoute,
        runtime: &Arc<RunningCodeRuntime>,
        payload: &Value,
    ) -> Result<Option<Value>> {
        let key = WorktreeRoute::buffer_key(payload)?;
        let Some(held) = route.agent_buffers.get_mut(&key) else {
            return Ok(None);
        };
        held.check_worktree(payload)?;
        let Some(attempt) = held.acquisition.as_mut() else {
            return Ok(None);
        };
        held.releasing = true;
        attempt.releasing = true;
        attempt.prepare(&runtime.socket, payload).await?;
        let reference = attempt.reference.as_ref().unwrap();
        let action = if attempt.release_submitted {
            "query"
        } else {
            "release"
        };
        attempt.release_submitted = true;
        let observed = request(
            &runtime.socket,
            &json!({"type":"agentAcquisition","acquisition":reference,"action":action}),
        )
        .await?;
        let reply = Reply::parse(observed, Some(reference))?;
        if matches!(
            reply.state,
            State::Prepared | State::Acquiring | State::Ready | State::ReleasePrepared
        ) {
            // An exact original state proves no cleanup step was admitted yet,
            // or the previous one completed. Continue only this same reference.
            attempt.release_submitted = false;
        }
        ensure!(
            reply.state == State::Released,
            "original acquisition cleanup remains unconfirmed"
        );
        let attempt = held.acquisition.take().unwrap();
        self.agent_acquisitions
            .retire(runtime.clone(), attempt.reference.unwrap(), attempt.permit);
        let response = json!({"type":"buffer","api_version":1,"worktree":payload["worktree"],"path":payload["path"],"leases":0,"buffer_id":0,"version":[]});
        if route.observe(payload, &response)? {
            *route = WorktreeRoute::default();
        }
        Ok(Some(response))
    }
}

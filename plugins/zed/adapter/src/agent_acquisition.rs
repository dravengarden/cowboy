//! Automatic acquisition prepared before `EnsureWorktree`, bound to one process.
//! The reference is not authority; the private socket owner admits each action.
use crate::{Buffers, Request, Response, Worktrees, Zed};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[cfg(test)]
pub(crate) mod connected;
mod entry;
#[cfg(test)]
mod tests;
use entry::Entry;
const CAPACITY: usize = 1_024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    instance: String,
    id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Execute,
    Query,
    Release,
    Retire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Prepared,
    Acquiring,
    Ready,
    ReleasePrepared,
    ReleaseUnknown,
    Released,
    Retired,
}

#[derive(Default)]
pub(crate) struct Registry {
    instance: Option<String>,
    last: u64,
    entries: HashMap<u64, Entry>,
}

pub(crate) async fn respond(
    request: Request,
    worktrees: &Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    if matches!(request, Request::AgentAcquisitionSupport {}) {
        crate::worktree_recovery::support(zed.context("native worktree recovery unavailable")?)
            .await?;
        return Ok(Response::AgentAcquisitionSupport {
            api_version: 1,
            protocol: 1,
        });
    }
    let mut registry = buffers.acquisitions.lock().await;
    match request {
        Request::PrepareAgentAcquisition {
            worktree,
            path,
            lease_id,
        } => registry.prepare(worktree, path, lease_id).await,
        Request::AgentAcquisition {
            acquisition,
            action,
        } => {
            registry
                .act(acquisition, action, worktrees, buffers, zed)
                .await
        }
        _ => anyhow::bail!("not an automatic acquisition request"),
    }
}

impl Registry {
    async fn prepare(
        &mut self,
        worktree: PathBuf,
        path: PathBuf,
        owner: String,
    ) -> Result<Response> {
        crate::validate_lease_id(&owner)?;
        ensure!(
            worktree.is_absolute()
                && worktree.as_os_str().len() <= 4096
                && path.as_os_str().len() <= 4096,
            "invalid acquisition paths"
        );
        if let Some((&id, entry)) = self.entries.iter().find(|(_, entry)| entry.owner == owner) {
            ensure!(
                entry.requested == (worktree, path),
                "original acquisition target changed"
            );
            return Ok(entry.reply(self.reference(id)));
        }
        ensure!(
            self.entries.len() < CAPACITY,
            "automatic acquisition capacity exhausted"
        );
        let (root, file) = crate::buffer_key(worktree.clone(), path.clone()).await?;
        let identity = entry::Identity::read(&root).await?;
        if self.instance.is_none() {
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce)?;
            self.instance = Some(format!("{:032x}", u128::from_be_bytes(nonce)));
        }
        let id = self
            .last
            .checked_add(1)
            .context("automatic acquisition IDs exhausted")?;
        self.last = id;
        let entry = Entry::new((worktree, path), owner, root, file, identity);
        let reply = entry.reply(self.reference(id));
        self.entries.insert(id, entry);
        Ok(reply)
    }

    fn reference(&self, id: u64) -> Reference {
        Reference {
            instance: self.instance.clone().expect("allocated before reference"),
            id: format!("{id:016x}"),
        }
    }

    async fn act(
        &mut self,
        reference: Reference,
        action: Action,
        worktrees: &Worktrees,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        ensure!(
            self.instance.as_ref() == Some(&reference.instance),
            "acquisition belongs to another process"
        );
        let id = u64::from_str_radix(&reference.id, 16).context("invalid acquisition ID")?;
        ensure!(
            id != 0 && id <= self.last && reference.id == format!("{id:016x}"),
            "acquisition reference was not issued"
        );
        let Some(entry) = self.entries.get_mut(&id) else {
            ensure!(
                !matches!(action, Action::Execute | Action::Release),
                "acquisition was retired"
            );
            return Ok(reply(reference, State::Retired, None));
        };
        match action {
            Action::Execute if entry.state == State::Prepared => {
                entry.execute(worktrees, buffers, zed).await?;
            }
            Action::Query => entry.observe(worktrees, buffers, zed).await?,
            Action::Release if entry.state == State::ReleaseUnknown => {
                entry.observe(worktrees, buffers, zed).await?;
            }
            Action::Release if entry.state != State::Released => {
                entry.release(worktrees, buffers, zed).await?;
            }
            Action::Retire => {
                ensure!(
                    entry.state == State::Released,
                    "acquisition cleanup remains unconfirmed"
                );
                self.entries.remove(&id);
                return Ok(reply(reference, State::Retired, None));
            }
            _ => {}
        }
        Ok(entry.reply(reference))
    }
}

fn reply(acquisition: Reference, state: State, buffer: Option<Box<serde_json::Value>>) -> Response {
    Response::AgentAcquisition {
        api_version: 1,
        acquisition,
        state,
        buffer,
    }
}

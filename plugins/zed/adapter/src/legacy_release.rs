//! Prepared one-use release of an existing legacy owner. The Machine retains
//! this process-local reference before Close; Query never dispatches Close.
use crate::{BufferOwner, Buffers, Request, Response, Zed, legacy_buffers, native_close};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const CAPACITY: usize = 1024;

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
    Retire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Prepared,
    Unknown,
    Released,
    Retired,
}

struct Entry {
    captured: legacy_buffers::Capture,
    state: State,
    buffer: Option<Box<Value>>,
}

#[derive(Default)]
pub(crate) struct Registry {
    instance: Option<String>,
    last: u64,
    entries: HashMap<u64, Entry>,
}

pub(crate) async fn respond(
    request: Request,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    if matches!(request, Request::LegacyReleaseSupport {}) {
        return Ok(Response::LegacyReleaseSupport {
            api_version: 1,
            protocol: 1,
        });
    }
    let mut registry = buffers.legacy_releases.lock().await;
    {
        let legacy = buffers.legacy.lock().await;
        // A live preparation must survive a delayed or lost reply. Only an
        // effect-free record whose exact original pin is gone can be pruned.
        registry.entries.retain(|_, entry| {
            entry.state != State::Prepared || legacy.check_release(&entry.captured, None).is_ok()
        });
    }
    match request {
        Request::PrepareLegacyRelease {
            worktree,
            path,
            lease_id,
        } => {
            crate::validate_lease_id(&lease_id)?;
            ensure!(
                worktree.as_os_str().len() <= 4096 && path.as_os_str().len() <= 4096,
                "legacy release path exceeds limit"
            );
            let legacy = buffers.legacy.lock().await;
            let captured = legacy.capture((worktree, path, lease_id))?;
            let active = buffers.active.read().await;
            if let Some(buffer_id) = captured.buffer_id {
                buffers.native_open.check()?;
                let buffer = active
                    .get(&captured.key)
                    .context("original legacy buffer is absent")?;
                ensure!(
                    buffer.remote_id == buffer_id
                        && buffer
                            .lease_ids
                            .contains(&BufferOwner::Legacy(captured.requested.2.clone())),
                    "original legacy buffer owner changed"
                );
                crate::sync_owners::ensure_readable(buffer)?;
            } else {
                ensure_uncommitted(&captured, &active)?;
            }
            if let Some((&id, entry)) = registry.entries.iter().find(|(_, entry)| {
                entry.state == State::Prepared && entry.captured.same_owner(&captured)
            }) {
                return Ok(reply(
                    Reference {
                        instance: registry.instance.clone().unwrap(),
                        id: format!("{id:016x}"),
                    },
                    entry.state,
                    None,
                ));
            }
            ensure!(
                registry.entries.len() < CAPACITY,
                "legacy release capacity exhausted"
            );
            if registry.instance.is_none() {
                let mut nonce = [0; 16];
                getrandom::fill(&mut nonce)?;
                registry.instance = Some(format!("{:032x}", u128::from_be_bytes(nonce)));
            }
            let id = registry
                .last
                .checked_add(1)
                .context("legacy release IDs exhausted")?;
            registry.last = id;
            registry.entries.insert(
                id,
                Entry {
                    captured,
                    state: State::Prepared,
                    buffer: None,
                },
            );
            Ok(reply(
                Reference {
                    instance: registry.instance.clone().unwrap(),
                    id: format!("{id:016x}"),
                },
                State::Prepared,
                None,
            ))
        }
        Request::LegacyRelease { release, action } => {
            registry.act(release, action, buffers, zed).await
        }
        _ => anyhow::bail!("not a legacy release request"),
    }
}

impl Registry {
    async fn act(
        &mut self,
        release: Reference,
        action: Action,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        ensure!(
            self.instance.as_ref() == Some(&release.instance),
            "legacy release belongs to another process"
        );
        let id = u64::from_str_radix(&release.id, 16).context("invalid legacy release ID")?;
        ensure!(
            id > 0 && id <= self.last && release.id == format!("{id:016x}"),
            "legacy release was not issued"
        );
        let Some(entry) = self.entries.get_mut(&id) else {
            ensure!(
                !matches!(action, Action::Execute),
                "legacy release was retired"
            );
            return Ok(reply(release, State::Retired, None));
        };
        match action {
            Action::Retire => {
                ensure!(
                    entry.state != State::Unknown,
                    "legacy release remains unconfirmed"
                );
                self.entries.remove(&id);
                return Ok(reply(release, State::Retired, None));
            }
            Action::Execute if entry.state == State::Prepared => {
                entry.execute(id, buffers, zed).await?;
            }
            Action::Query if entry.state == State::Unknown => {
                entry.observe(id, buffers, zed).await?;
            }
            _ => {}
        }
        Ok(reply(
            release,
            entry.state,
            if entry.state == State::Released {
                entry.buffer.clone()
            } else {
                None
            },
        ))
    }
}

impl Entry {
    async fn execute(&mut self, id: u64, buffers: &Buffers, zed: Option<&Zed>) -> Result<()> {
        let mut legacy = buffers.legacy.lock().await;
        legacy.check_release(&self.captured, None)?;
        let mut active = buffers.active.write().await;
        if self.captured.buffer_id.is_none() {
            ensure_uncommitted(&self.captured, &active)?;
            let recovered = self
                .captured
                .recovery
                .release(&buffers.native_open, &active, zed, || {
                    legacy.mark_release(&self.captured, id);
                    self.state = State::Unknown;
                })
                .await?;
            ensure!(recovered, "original legacy Open remains unconfirmed");
            self.finish_uncommitted(&mut legacy)?;
            return Ok(());
        }
        let owner = BufferOwner::Legacy(self.captured.requested.2.clone());
        let buffer = active
            .get(&self.captured.key)
            .context("original legacy buffer disappeared")?;
        ensure!(
            Some(buffer.remote_id) == self.captured.buffer_id && buffer.lease_ids.contains(&owner),
            "original legacy buffer owner changed"
        );
        self.buffer = Some(Box::new(serde_json::to_value(Response::Buffer {
            api_version: 1,
            worktree: self.captured.key.0.clone(),
            path: self.captured.key.1.clone(),
            leases: buffer.lease_ids.len() - 1,
            buffer_id: buffer.remote_id,
            version: buffer.version.clone(),
        })?));
        native_close::release(
            HashSet::from([self.captured.key.clone()]),
            &owner,
            &mut active,
            zed,
            &buffers.native_open,
            || {
                legacy.mark_release(&self.captured, id);
                self.state = State::Unknown;
            },
        )
        .await?;
        legacy.released(&self.captured.key, &self.captured.requested.2);
        self.state = State::Released;
        Ok(())
    }

    async fn observe(&mut self, id: u64, buffers: &Buffers, zed: Option<&Zed>) -> Result<()> {
        let Some(zed) = zed else {
            return Ok(());
        };
        let mut legacy = buffers.legacy.lock().await;
        legacy.check_release(&self.captured, Some(id))?;
        let mut active = buffers.active.write().await;
        if self.captured.buffer_id.is_none() {
            ensure_uncommitted(&self.captured, &active)?;
            if self
                .captured
                .recovery
                .observe(&buffers.native_open, &active, Some(zed))
                .await
            {
                self.finish_uncommitted(&mut legacy)?;
            }
            return Ok(());
        }
        if native_close::recover(
            &HashSet::from([self.captured.key.clone()]),
            &BufferOwner::Legacy(self.captured.requested.2.clone()),
            &mut active,
            zed,
        )
        .await?
        {
            legacy.released(&self.captured.key, &self.captured.requested.2);
            self.state = State::Released;
        }
        Ok(())
    }

    fn finish_uncommitted(&mut self, legacy: &mut legacy_buffers::Registry) -> Result<()> {
        self.buffer = Some(Box::new(serde_json::to_value(Response::Buffer {
            api_version: 1,
            worktree: self.captured.key.0.clone(),
            path: self.captured.key.1.clone(),
            leases: 0,
            buffer_id: 0,
            version: Vec::new(),
        })?));
        legacy.released(&self.captured.key, &self.captured.requested.2);
        self.state = State::Released;
        Ok(())
    }
}

fn ensure_uncommitted(
    captured: &legacy_buffers::Capture,
    active: &HashMap<(std::path::PathBuf, std::path::PathBuf), crate::BufferLease>,
) -> Result<()> {
    ensure!(
        active.get(&captured.key).is_none_or(|buffer| !buffer
            .lease_ids
            .contains(&BufferOwner::Legacy(captured.requested.2.clone()))),
        "unconfirmed legacy owner cannot be released"
    );
    Ok(())
}

fn reply(release: Reference, state: State, buffer: Option<Box<Value>>) -> Response {
    Response::LegacyRelease {
        api_version: 1,
        release,
        state,
        buffer,
    }
}

#[cfg(test)]
mod tests;

//! Private adapter coordination, not a Service authorization API. Generic
//! Machine forwarding rejects effects; protocol 20 adds a separate core-owned
//! invocation. A serialized ID, content hash or read lease grants no effect.
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::sync_native::wire::{
    CowboyBufferSync, CowboyBufferSyncResponse, CowboyBufferSyncVersion,
    cowboy_buffer_sync::Action as NativeAction,
    cowboy_buffer_sync_response::{Phase, Refusal},
};
use crate::{
    BufferLease, BufferOwner, Buffers, Response, Zed, buffer_leases, content_reads::Content,
};

type Key = (PathBuf, PathBuf);
const MAX_OPERATIONS: usize = 256;
const PREPARE_TTL: Duration = Duration::from_secs(30);

pub(super) async fn support(zed: Option<&Zed>) -> Result<Response> {
    // A distinct, effect-free claim by this adapter, not only its native
    // server. Old adapters must not be mistaken for exclusive owners.
    crate::sync_native::support(zed).await?;
    Ok(Response::BufferSyncOwnerSupport {
        api_version: crate::ADAPTER_VERSION,
        protocol: 1,
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Purpose {
    RefreshFromDisk,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Apply,
    Query,
    Retire,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OperationRef {
    instance: String,
    id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum State {
    Prepared,
    Pending,
    Unknown,
    Applied {
        content: Content,
        version: Vec<crate::BufferVersionEntry>,
    },
    Refused {
        reason: Reason,
    },
    Retired,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Reason {
    Changed,
    Source,
    Shared,
}

impl State {
    fn terminal(&self) -> bool {
        matches!(self, Self::Applied { .. } | Self::Refused { .. })
    }
}

pub(super) struct Fence {
    id: u64,
    until: Option<Instant>,
}

impl Fence {
    fn live(&self) -> bool {
        self.until.is_none_or(|until| Instant::now() < until)
    }
}

pub(super) fn ensure_readable(buffer: &BufferLease) -> Result<()> {
    ensure!(!buffer.closing, "original native peer close is unresolved");
    ensure!(
        !buffer.sync.as_ref().is_some_and(Fence::live),
        "original native buffer is reserved for synchronization"
    );
    Ok(())
}

pub(super) fn ensure_admission(
    buffers: &crate::BufferState,
    active: &HashMap<Key, BufferLease>,
) -> Result<()> {
    buffers.native_open.check()?;
    // A path not in this map can alias an existing native ID (overlapping roots,
    // links, native canonicalization). Until that ID is known, do not issue an
    // OpenBuffer/navigation at all. This conservative gate is process-wide;
    // unrelated already-open reads and releases remain available.
    for buffer in active.values() {
        ensure_readable(buffer)?;
        ensure!(
            !buffer
                .lease_ids
                .iter()
                .any(|owner| matches!(owner, BufferOwner::NavigationPending(_))),
            "unresolved navigation prevents native admission"
        );
    }
    Ok(())
}

struct Ticket {
    instance: Vec<u8>,
    id: u64,
}

struct Slot {
    key: Key,
    owner: u64,
    remote_id: u64,
    content: Content,
    until: Instant,
    ticket: Option<Ticket>,
    state: State,
}

#[derive(Default)]
pub(super) struct Registry {
    instance: Option<String>,
    last_id: u64,
    slots: HashMap<u64, Slot>,
}

pub(super) async fn prepare(
    lease: buffer_leases::LeaseRef,
    purpose: Purpose,
    content: Content,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    match purpose {
        Purpose::RefreshFromDisk => {}
    }
    content.validate()?;
    let zed = zed.context("native synchronization unavailable")?;
    // Lock order: original leases -> sync registry -> active map. Legacy
    // paths take only active; no path takes leases while holding active.
    let mut owners = buffers.leases.lock().await;
    let (owner, key) = owners.sync_target(&lease)?;
    let mut registry = buffers.syncs.lock().await;
    registry.expire();
    ensure!(
        registry.slots.len() < MAX_OPERATIONS,
        "synchronization capacity reached"
    );
    let mut active = buffers.active.write().await;
    ensure_admission(buffers, &active)?;
    let buffer = active
        .get(&key)
        .context("original native buffer unavailable")?;
    ensure!(
        buffer.lease_ids.contains(&BufferOwner::Owned(owner))
            && active
                .values()
                .filter(|other| other.remote_id == buffer.remote_id)
                .map(|other| other.lease_ids.len())
                .sum::<usize>()
                == 1,
        "native buffer has another owner"
    );
    let remote_id = buffer.remote_id;
    let mut version: Vec<_> = zed
        .diagnostics
        .lock()
        .expect("diagnostic cache poisoned")
        .version(remote_id)?
        .into_iter()
        .map(|entry| CowboyBufferSyncVersion {
            replica_id: entry.replica_id,
            timestamp: entry.timestamp,
        })
        .collect();
    version.sort_by_key(|entry| entry.replica_id);
    let instance = zed.sync.probe(zed).await?.to_vec();
    let (id, operation) = registry.allocate()?;
    let until = Instant::now() + PREPARE_TTL;
    // An interrupted Prepare has no possible Apply. Only this effect-free
    // reservation may expire; an issued operation ID is never reused.
    active.get_mut(&key).expect("retained buffer").sync = Some(Fence {
        id,
        until: Some(until),
    });
    registry.slots.insert(
        id,
        Slot {
            key,
            owner,
            remote_id,
            content: content.clone(),
            until,
            ticket: None,
            state: State::Prepared,
        },
    );
    let reply = zed
        .sync
        .request(
            zed,
            CowboyBufferSync {
                protocol: 1,
                action: NativeAction::Prepare as i32,
                instance,
                buffer_id: remote_id,
                version,
                content_sha256: digest_bytes(&content.sha256),
                content_bytes: content.utf8_bytes,
                ..Default::default()
            },
        )
        .await?;
    let slot = registry.slots.get_mut(&id).expect("retained preparation");
    slot.ticket = Some(Ticket {
        instance: reply.instance,
        id: reply.operation_id,
    });
    ensure!(
        Instant::now() < until,
        "synchronization preparation expired"
    );
    Ok(response(operation, State::Prepared))
}

impl Registry {
    fn expire(&mut self) {
        self.slots.retain(|_, slot| {
            !matches!(slot.state, State::Prepared) || Instant::now() < slot.until
        });
    }

    fn allocate(&mut self) -> Result<(u64, OperationRef)> {
        if self.instance.is_none() {
            let mut bytes = [0; 16];
            getrandom::fill(&mut bytes)
                .map_err(|_| anyhow::anyhow!("synchronization identity unavailable"))?;
            self.instance = Some(format!("{:032x}", u128::from_be_bytes(bytes)));
        }
        self.last_id = self
            .last_id
            .checked_add(1)
            .context("synchronization IDs exhausted")?;
        Ok((
            self.last_id,
            OperationRef {
                instance: self.instance.clone().expect("initialized identity"),
                id: format!("{:016x}", self.last_id),
            },
        ))
    }

    fn resolve(&mut self, operation: &OperationRef) -> Result<u64> {
        self.expire();
        ensure!(
            self.instance.as_ref() == Some(&operation.instance),
            "foreign synchronization instance"
        );
        ensure!(
            operation.id.len() == 16
                && operation
                    .id
                    .bytes()
                    .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) }),
            "invalid synchronization ID"
        );
        let id = u64::from_str_radix(&operation.id, 16)?;
        ensure!(
            id > 0 && id <= self.last_id,
            "synchronization ID was never prepared"
        );
        Ok(id)
    }

    pub(super) async fn act(
        &mut self,
        operation: OperationRef,
        action: Action,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        let id = self.resolve(&operation)?;
        let Some(slot) = self.slots.get_mut(&id) else {
            return Ok(response(operation, State::Retired));
        };
        // Also finish local cleanup if an earlier observer was cancelled after
        // recording a terminal response but before obtaining the active lock.
        if slot.state.terminal() {
            clear_fence(buffers, slot, id).await;
        }
        match action {
            Action::Apply if !matches!(slot.state, State::Prepared) => {
                // Duplicate Apply is a local observation, never a native RPC.
                return Ok(response(operation, slot.state.clone()));
            }
            Action::Retire if matches!(slot.state, State::Pending | State::Unknown) => {
                return Ok(response(operation, slot.state.clone()));
            }
            Action::Query if slot.state.terminal() || matches!(slot.state, State::Prepared) => {
                return Ok(response(operation, slot.state.clone()));
            }
            Action::Retire if matches!(slot.state, State::Prepared) => {
                // No Apply was sent. Its inaccessible native preparation may
                // expire independently; local disposal cannot cause a write.
                clear_fence(buffers, slot, id).await;
                self.slots.remove(&id);
                return Ok(response(operation, State::Retired));
            }
            _ => {}
        }
        let zed = zed.context("original native synchronization runtime unavailable")?;
        let ticket = slot
            .ticket
            .as_ref()
            .context("native preparation was not observed")?;
        if matches!(action, Action::Apply) {
            let mut active = buffers.active.write().await;
            let buffer = active
                .get_mut(&slot.key)
                .context("original buffer unavailable")?;
            ensure!(
                buffer.remote_id == slot.remote_id
                    && buffer.lease_ids.len() == 1
                    && buffer.lease_ids.contains(&BufferOwner::Owned(slot.owner)),
                "original buffer owner changed"
            );
            let fence = buffer
                .sync
                .as_mut()
                .context("synchronization reservation lost")?;
            ensure!(
                fence.id == id && fence.live() && Instant::now() < slot.until,
                "synchronization reservation expired or changed"
            );
            // Commit uncertainty before the first transport await. Cancellation
            // drops an observer, not this retained fence or the Apply budget.
            fence.until = None;
            slot.state = State::Unknown;
        }
        let native_action = match action {
            Action::Apply => NativeAction::Apply,
            Action::Query => NativeAction::Query,
            Action::Retire => NativeAction::Retire,
        };
        let reply = zed
            .sync
            .request(
                zed,
                CowboyBufferSync {
                    protocol: 1,
                    action: native_action as i32,
                    instance: ticket.instance.clone(),
                    operation_id: ticket.id,
                    ..Default::default()
                },
            )
            .await?;
        if matches!(action, Action::Retire) {
            ensure!(
                reply.phase == Phase::Retired as i32,
                "native retirement not observed"
            );
            self.slots.remove(&id);
            return Ok(response(operation, State::Retired));
        }
        slot.observe(reply)?;
        slot.invalidate_applied_observations(zed);
        if slot.state.terminal() {
            clear_fence(buffers, slot, id).await;
        }
        Ok(response(operation, slot.state.clone()))
    }
}

impl Slot {
    fn invalidate_applied_observations(&self, zed: &Zed) {
        if let State::Applied { version, .. } = &self.state {
            // A private reply may arrive before all native text events. Reuse
            // the mirror's bounded reload floor and invalidate old coordinates
            // before exposing the terminal outcome or releasing exclusion.
            zed.diagnostics
                .lock()
                .expect("diagnostic cache poisoned")
                .observe(&proto::envelope::Payload::BufferReloaded(
                    proto::BufferReloaded {
                        project_id: proto::REMOTE_SERVER_PROJECT_ID,
                        buffer_id: self.remote_id,
                        version: version
                            .iter()
                            .map(|entry| proto::VectorClockEntry {
                                replica_id: entry.replica_id,
                                timestamp: entry.timestamp,
                            })
                            .collect(),
                        ..Default::default()
                    },
                ));
        }
    }

    fn observe(&mut self, reply: CowboyBufferSyncResponse) -> Result<()> {
        self.state = match Phase::from_i32(reply.phase).context("invalid native phase")? {
            Phase::Pending => State::Pending,
            Phase::Applied => {
                ensure!(
                    reply.content_sha256 == digest_bytes(&self.content.sha256)
                        && reply.content_bytes == self.content.utf8_bytes,
                    "native applied content differs from original intent"
                );
                State::Applied {
                    content: self.content.clone(),
                    version: reply
                        .version
                        .into_iter()
                        .map(|entry| crate::BufferVersionEntry {
                            replica_id: entry.replica_id,
                            timestamp: entry.timestamp,
                        })
                        .collect(),
                }
            }
            Phase::Refused => State::Refused {
                reason: match Refusal::from_i32(reply.refusal) {
                    Some(Refusal::Changed) => Reason::Changed,
                    Some(Refusal::Source) => Reason::Source,
                    Some(Refusal::Shared) => Reason::Shared,
                    // Core's accepted owner codec has no budget outcome yet.
                    // Do not invent a public enum or classify it as Source.
                    // The original Unknown fence remains; Query never replays.
                    Some(Refusal::Budget) => anyhow::bail!(
                        "native replacement budget refusal requires owner-level reconciliation"
                    ),
                    _ => anyhow::bail!("invalid native refusal"),
                },
            },
            // A missing native record after an attempted Apply does not prove
            // either no effect or restoration. Never release its owner fence.
            Phase::Prepared | Phase::Retired => State::Unknown,
            _ => anyhow::bail!("invalid native synchronization observation"),
        };
        Ok(())
    }
}

async fn clear_fence(buffers: &Buffers, slot: &Slot, id: u64) {
    let mut active = buffers.active.write().await;
    if let Some(buffer) = active.get_mut(&slot.key)
        && buffer.remote_id == slot.remote_id
        && buffer.sync.as_ref().is_some_and(|fence| fence.id == id)
    {
        buffer.sync = None;
    }
}

fn digest_bytes(hex: &str) -> Vec<u8> {
    // Called only on a validated closed Content, never arbitrary wire input.
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| {
                if byte.is_ascii_digit() {
                    byte - b'0'
                } else {
                    byte - b'a' + 10
                }
            };
            (nibble(pair[0]) << 4) | nibble(pair[1])
        })
        .collect()
}

fn response(operation: OperationRef, state: State) -> Response {
    Response::BufferSync {
        api_version: 1,
        operation,
        state,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod connected;

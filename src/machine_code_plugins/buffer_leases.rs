//! Process-owned routing for Zed's prepared buffer references. Handles select
//! a retained native process, never a new installation, path or Machine epoch.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::{CodeTarget, RunningCodeRuntime, WorktreeRoute, exchange};
use crate::code_buffer_read;

const MAX_LEASES: usize = 1_024;
const PREPARE_TTL: Duration = Duration::from_secs(30);
type Key = (String, LeaseRef);

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LeaseRef {
    instance: String,
    id: String,
}

impl LeaseRef {
    fn validate(&self) -> Result<()> {
        let hex = |value: &str, len| {
            value.len() == len
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        ensure!(
            hex(&self.instance, 32) && hex(&self.id, 16) && self.id != "0000000000000000",
            "invalid native buffer reference"
        );
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Command {
    BufferLeaseSupport {},
    BufferLeaseReadSupport {},
    BufferLeaseContentSupport {},
    PrepareBuffer {
        worktree: String,
        path: String,
    },
    OpenBufferLease {
        lease: LeaseRef,
    },
    ReleaseBufferLease {
        lease: LeaseRef,
    },
    QueryBufferLease {
        lease: LeaseRef,
    },
    ReadBufferLease {
        lease: LeaseRef,
        request: code_buffer_read::Request,
    },
}

impl Command {
    pub(super) fn parse(payload: &Value) -> Result<Option<Self>> {
        if !matches!(
            payload["type"].as_str(),
            Some(
                "bufferLeaseSupport"
                    | "bufferLeaseReadSupport"
                    | "bufferLeaseContentSupport"
                    | "prepareBuffer"
                    | "openBufferLease"
                    | "releaseBufferLease"
                    | "queryBufferLease"
                    | "readBufferLease"
            )
        ) {
            return Ok(None);
        }
        let value: Self =
            serde_json::from_value(payload.clone()).context("invalid owned buffer command")?;
        match &value {
            Self::BufferLeaseSupport {}
            | Self::BufferLeaseReadSupport {}
            | Self::BufferLeaseContentSupport {} => {}
            Self::PrepareBuffer { worktree, path } => ensure!(
                worktree.len() <= 4_096 && path.len() <= 4_096,
                "buffer path exceeds byte limit"
            ),
            Self::OpenBufferLease { lease }
            | Self::ReleaseBufferLease { lease }
            | Self::QueryBufferLease { lease } => lease.validate()?,
            Self::ReadBufferLease { lease, request } => {
                lease.validate()?;
                request.validate()?;
            }
        }
        Ok(Some(value))
    }

    pub(super) fn is_prepare(&self) -> bool {
        matches!(self, Self::PrepareBuffer { .. })
    }

    fn lease(&self) -> Result<&LeaseRef> {
        match self {
            Self::OpenBufferLease { lease }
            | Self::ReleaseBufferLease { lease }
            | Self::QueryBufferLease { lease }
            | Self::ReadBufferLease { lease, .. } => Ok(lease),
            Self::BufferLeaseSupport {}
            | Self::BufferLeaseReadSupport {}
            | Self::BufferLeaseContentSupport {}
            | Self::PrepareBuffer { .. } => {
                anyhow::bail!("buffer reference has not been prepared")
            }
        }
    }
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Prepared,
    Open,
    Released,
    Unknown,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(rename = "type")]
    kind: String,
    api_version: u8,
    lease: LeaseRef,
    state: State,
}

impl Reply {
    fn parse(value: &Value) -> Result<Self> {
        let reply: Self =
            serde_json::from_value(value.clone()).context("invalid native buffer lease reply")?;
        ensure!(
            reply.kind == "bufferLease" && reply.api_version == 1,
            "unsupported native buffer lease reply"
        );
        reply.lease.validate()?;
        Ok(reply)
    }
}

pub(super) struct Reservation {
    until: Instant,
    permit: OwnedSemaphorePermit,
}

struct Entry {
    route: Arc<Mutex<WorktreeRoute>>,
    runtime: Arc<RunningCodeRuntime>,
    // Removed before an open can cross an async boundary. Only effect-free
    // reservations may expire; missing/ambiguous replies keep their permit.
    until: Option<Instant>,
    _permit: OwnedSemaphorePermit,
}

#[derive(Default)]
struct Registry {
    entries: BTreeMap<Key, Entry>,
    // Bounded successful local-release evidence, without retaining processes.
    // Evicted evidence becomes unavailable, never authority to open/replay.
    released: VecDeque<Key>,
}

pub(super) struct Routes {
    registry: Mutex<Registry>,
    capacity: Arc<Semaphore>,
}

// Held under the worktree lock, including while awaiting the native response.
// A failed/cancelled preparation has no buffer effect and cannot strand an
// otherwise unleased runtime. Existing worktree and buffer leases still win.
struct InertPreparation<'a>(&'a mut WorktreeRoute);

impl Drop for InertPreparation<'_> {
    fn drop(&mut self) {
        if self.0.is_idle() {
            *self.0 = WorktreeRoute::default();
        }
    }
}

impl Default for Routes {
    fn default() -> Self {
        Self {
            registry: Mutex::default(),
            capacity: Arc::new(Semaphore::new(MAX_LEASES)),
        }
    }
}

impl Registry {
    fn retire(&mut self, key: Key) -> Option<Entry> {
        let entry = self.entries.remove(&key)?;
        if self.released.len() == MAX_LEASES {
            self.released.pop_front();
        }
        self.released.push_back(key);
        Some(entry)
    }
}

impl Routes {
    async fn reap_prepared(&self) {
        let expired = {
            let registry = self.registry.lock().await;
            registry
                .entries
                .iter()
                .filter(|(_, entry)| entry.until.is_some_and(|until| until <= Instant::now()))
                .map(|(key, entry)| (key.clone(), Arc::clone(&entry.route)))
                .collect::<Vec<_>>()
        };
        for (key, route) in expired {
            // Acquire both locks before retirement. Cancellation while waiting
            // must not discard the only record needed to release the route.
            let mut route = route.lock().await;
            let mut registry = self.registry.lock().await;
            if registry
                .entries
                .get(&key)
                .is_some_and(|entry| entry.until.is_some_and(|until| until <= Instant::now()))
            {
                registry.retire(key.clone());
                forget_route(&key.1, &mut route);
            }
        }
    }

    pub(super) async fn reserve(&self) -> Result<Reservation> {
        self.reap_prepared().await;
        Ok(Reservation {
            until: Instant::now() + PREPARE_TTL,
            permit: Arc::clone(&self.capacity)
                .try_acquire_owned()
                .context("Machine buffer lease capacity reached")?,
        })
    }

    pub(super) async fn prepare(
        &self,
        plugin_id: &str,
        reservation: Reservation,
        retained_route: Arc<Mutex<WorktreeRoute>>,
        route: &mut WorktreeRoute,
        payload: &Value,
    ) -> Result<Value> {
        let guard = InertPreparation(route);
        ensure!(
            reservation.until > Instant::now(),
            "buffer preparation expired"
        );
        let Some(CodeTarget::Installed(runtime)) = &guard.0.target else {
            anyhow::bail!("owned buffer leases require an installed Code Plugin generation");
        };
        let response = exchange(&runtime.socket, payload).await?;
        let reply = Reply::parse(&response)?;
        ensure!(
            reply.state == State::Prepared,
            "native buffer was not prepared"
        );
        ensure!(
            reservation.until > Instant::now(),
            "buffer preparation expired"
        );
        let key = (plugin_id.to_owned(), reply.lease.clone());
        let mut registry = self.registry.lock().await;
        ensure!(
            !registry.entries.contains_key(&key) && !registry.released.contains(&key),
            "native buffer reference was reused"
        );
        registry.entries.insert(
            key,
            Entry {
                route: retained_route,
                runtime: Arc::clone(runtime),
                until: Some(reservation.until),
                _permit: reservation.permit,
            },
        );
        guard.0.owned_buffers.insert(reply.lease);
        Ok(response)
    }

    pub(super) async fn request(
        &self,
        plugin_id: &str,
        command: Command,
        payload: &Value,
    ) -> Result<Value> {
        if matches!(command, Command::BufferLeaseSupport {}) {
            // Core-owned, effect-free negotiation. An older Machine rejects
            // this pathless request before it could reach a Plugin socket.
            return Ok(serde_json::json!({"type":"bufferLeaseSupport", "api_version":1}));
        }
        if matches!(command, Command::BufferLeaseReadSupport {}) {
            return Ok(serde_json::json!({"type":"bufferLeaseReadSupport", "api_version":1}));
        }
        if matches!(command, Command::BufferLeaseContentSupport {}) {
            return Ok(serde_json::json!({"type":"bufferLeaseContentSupport", "api_version":1}));
        }
        self.reap_prepared().await;
        let key = (plugin_id.to_owned(), command.lease()?.clone());
        let (runtime, route) = {
            let mut registry = self.registry.lock().await;
            if registry.released.contains(&key) {
                ensure!(
                    !matches!(
                        command,
                        Command::OpenBufferLease { .. } | Command::ReadBufferLease { .. }
                    ),
                    "native buffer lease was released"
                );
                return Ok(
                    serde_json::json!({"type":"bufferLease", "api_version":1, "lease":key.1, "state":"released"}),
                );
            }
            let entry = registry
                .entries
                .get_mut(&key)
                .context("native buffer lease is not retained by this Machine")?;
            if matches!(command, Command::OpenBufferLease { .. }) {
                entry.until = None;
            }
            ensure!(
                !matches!(command, Command::ReadBufferLease { .. }) || entry.until.is_none(),
                "native buffer has not been opened"
            );
            (Arc::clone(&entry.runtime), Arc::clone(&entry.route))
        };
        // Serialize with original worktree operations, but never consult its
        // current target: a dead/reopened route cannot replace this process.
        let mut route_guard = route.lock().await;
        ensure!(
            runtime.is_running()?,
            "original buffer runtime exited; outcome unavailable"
        );
        let response = exchange(&runtime.socket, payload).await?;
        if let Command::ReadBufferLease { request, .. } = command {
            code_buffer_read::Reply::parse(&response, &key.1, request)?;
            // A read never changes effect state, expiry, or routing ownership.
            return Ok(response);
        }
        let reply = Reply::parse(&response)?;
        ensure!(
            reply.lease == key.1,
            "native buffer reply changed its reference"
        );
        ensure!(
            !matches!(command, Command::OpenBufferLease { .. })
                || matches!(reply.state, State::Open | State::Unknown),
            "native buffer did not open"
        );
        ensure!(
            !matches!(command, Command::ReleaseBufferLease { .. })
                || matches!(reply.state, State::Released | State::Unknown),
            "native buffer did not release"
        );
        if reply.state == State::Released {
            self.registry.lock().await.retire(key.clone());
            forget_route(&key.1, &mut route_guard);
        }
        Ok(response)
    }
}

fn forget_route(lease: &LeaseRef, route: &mut WorktreeRoute) {
    if route.owned_buffers.remove(lease) && route.is_idle() {
        *route = WorktreeRoute::default();
    }
}

#[cfg(test)]
mod tests;

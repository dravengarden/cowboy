//! Finite original-owner synchronization, never generic adapter forwarding.
//! Unknown effects keep their process and exclusion; neither expiry nor a new
//! connection can authorize replay. Loss of Machine state is not restoration.

use super::{buffer_leases, exchange};
use crate::machine_plugins::{CodeBufferSyncInvocation, CodeBufferSyncOwner};
use crate::machine_protocol::code_buffer_sync::{Action, Content, OperationRef, Snapshot, State};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

const MAX_OPERATIONS: usize = 256;
const MAX_COMMANDS: usize = 64;
const PREPARE_TTL: Duration = Duration::from_secs(30);

struct Slot {
    owner: CodeBufferSyncOwner,
    target: Option<buffer_leases::SyncTarget>,
    fence: Option<buffer_leases::SyncFence>,
    native: OperationRef,
    content: Content,
    state: State,
    until: Option<Instant>,
    attempted: bool,
    retire_attempted: bool,
    permit: Option<OwnedSemaphorePermit>,
}

struct Registry {
    instance: String,
    last_id: u64,
    active: BTreeMap<OperationRef, Arc<Mutex<Slot>>>,
    retired: VecDeque<(OperationRef, Arc<Mutex<Slot>>)>,
    native: BTreeMap<OperationRef, OperationRef>,
}

pub(super) struct Operations {
    registry: parking_lot::Mutex<Registry>,
    capacity: Arc<Semaphore>,
    commands: Semaphore,
}

impl Default for Operations {
    fn default() -> Self {
        Self {
            registry: parking_lot::Mutex::new(Registry {
                instance: uuid::Uuid::new_v4().simple().to_string(),
                last_id: 0,
                active: BTreeMap::new(),
                retired: VecDeque::new(),
                native: BTreeMap::new(),
            }),
            capacity: Arc::new(Semaphore::new(MAX_OPERATIONS)),
            commands: Semaphore::new(MAX_COMMANDS),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum NativeAction {
    Apply,
    Query,
    Retire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    #[serde(rename = "type")]
    kind: ReplyKind,
    api_version: u8,
    operation: OperationRef,
    state: State,
}

#[derive(Deserialize)]
enum ReplyKind {
    #[serde(rename = "bufferSync")]
    BufferSync,
}

fn reply(value: &Value, content: &Content) -> Result<Reply> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= 16 * 1024, "synchronization reply too large");
    let reply: Reply = serde_json::from_slice(&bytes)?;
    let ReplyKind::BufferSync = reply.kind;
    ensure!(reply.api_version == 1, "unsupported synchronization reply");
    reply.operation.validate()?;
    reply.state.validate(content)?;
    Ok(reply)
}

impl Registry {
    fn retire(&mut self, id: &OperationRef, entry: &Arc<Mutex<Slot>>, slot: &mut Slot) {
        slot.state = State::Retired {};
        slot.target = None;
        slot.fence = None;
        slot.permit = None;
        slot.until = None;
        self.active.remove(id);
        if self.retired.len() == MAX_OPERATIONS
            && let Some((old, _)) = self.retired.pop_front()
        {
            self.native.retain(|_, owner| owner != &old);
        }
        self.retired.push_back((id.clone(), Arc::clone(entry)));
    }

    fn expire(&mut self) {
        // No await while the registry is locked; busy/attempted slots stay.
        let entries: Vec<_> = self
            .active
            .iter()
            .map(|(id, entry)| (id.clone(), Arc::clone(entry)))
            .collect();
        for (id, entry) in entries {
            if let Ok(mut slot) = entry.try_lock()
                && !slot.attempted
                && slot.until.is_some_and(|until| until <= Instant::now())
            {
                self.retire(&id, &entry, &mut slot);
            }
        }
    }

    fn allocate(&mut self) -> Result<OperationRef> {
        self.last_id = self
            .last_id
            .checked_add(1)
            .context("synchronization identities exhausted")?;
        Ok(OperationRef {
            instance: self.instance.clone(),
            id: format!("{:016x}", self.last_id),
        })
    }
}

impl Operations {
    pub(super) async fn execute(
        &self,
        invocation: CodeBufferSyncInvocation,
        leases: &buffer_leases::Routes,
    ) -> Result<Snapshot> {
        let _command = self
            .commands
            .try_acquire()
            .context("synchronization command capacity reached")?;
        tokio::time::timeout(
            invocation.remaining()?,
            self.execute_inner(&invocation, leases),
        )
        .await
        .context("synchronization command deadline ended")?
    }

    async fn execute_inner(
        &self,
        invocation: &CodeBufferSyncInvocation,
        leases: &buffer_leases::Routes,
    ) -> Result<Snapshot> {
        if let Action::Prepare {
            lease,
            purpose,
            content,
        } = &invocation.request().action
        {
            let (id, permit) = {
                let mut registry = self.registry.lock();
                registry.expire();
                let permit = Arc::clone(&self.capacity)
                    .try_acquire_owned()
                    .context("synchronization capacity reached")?;
                (registry.allocate()?, permit)
            };
            let until = Instant::now() + PREPARE_TTL;
            let target = leases.sync_target(lease).await?;
            let route = Arc::clone(&target.route);
            let _route = route.lock().await;
            invocation.remaining()?;
            ensure!(
                target.runtime.is_running()?,
                "original buffer runtime exited"
            );
            // Native protocol 1 also describes an older adapter without alias
            // exclusion. Probe this exact adapter's own closed contract.
            let supported = exchange(
                &target.runtime.socket,
                &json!({"type":"bufferSyncOwnerSupport"}),
            )
            .await?;
            ensure!(
                supported == json!({"type":"bufferSyncOwnerSupport","api_version":1,"protocol":1}),
                "owned synchronization unavailable"
            );
            invocation.remaining()?;
            let fence = leases.sync_fence(lease, &target).await?;
            invocation.remaining()?;
            let value = exchange(&target.runtime.socket, &json!({"type":"prepareBufferSync","lease":lease,"purpose":purpose,"content":content})).await?;
            let observed = reply(&value, content)?;
            ensure!(
                observed.state == (State::Prepared {}),
                "synchronization was not prepared"
            );
            let owner = invocation.owner()?;
            ensure!(
                until > Instant::now(),
                "synchronization preparation expired"
            );
            let mut registry = self.registry.lock();
            ensure!(
                !registry.native.contains_key(&observed.operation),
                "native synchronization reference reused"
            );
            registry
                .native
                .insert(observed.operation.clone(), id.clone());
            registry.active.insert(
                id.clone(),
                Arc::new(Mutex::new(Slot {
                    owner,
                    target: Some(target),
                    fence: Some(fence),
                    native: observed.operation,
                    content: content.clone(),
                    state: State::Prepared {},
                    until: Some(until),
                    attempted: false,
                    retire_attempted: false,
                    permit: Some(permit),
                })),
            );
            return Ok(Snapshot {
                api_version: 1,
                operation: id,
                state: State::Prepared {},
            });
        }

        let (id, action) = match &invocation.request().action {
            Action::Apply { operation } => (operation, NativeAction::Apply),
            Action::Query { operation } => (operation, NativeAction::Query),
            Action::Retire { operation } => (operation, NativeAction::Retire),
            Action::Prepare { .. } => unreachable!(),
        };
        let entry = {
            let mut registry = self.registry.lock();
            registry.expire();
            registry
                .active
                .get(id)
                .or_else(|| {
                    registry
                        .retired
                        .iter()
                        .find(|(key, _)| key == id)
                        .map(|(_, entry)| entry)
                })
                .cloned()
                .context("synchronization is not retained by this Machine")?
        };
        let mut slot = entry.lock().await;
        invocation.check_owner(&slot.owner)?;
        let snapshot = |state| Snapshot {
            api_version: 1,
            operation: id.clone(),
            state,
        };
        if slot.state == (State::Retired {})
            || action == NativeAction::Apply
                && (slot.attempted || slot.retire_attempted || slot.state != (State::Prepared {}))
            || action == NativeAction::Query && slot.state.terminal() && !slot.retire_attempted
            || action == NativeAction::Retire
                && (slot.retire_attempted || slot.attempted && !slot.state.terminal())
        {
            return Ok(snapshot(slot.state.clone()));
        }
        if !slot.attempted && slot.until.is_some_and(|until| until <= Instant::now()) {
            self.registry.lock().retire(id, &entry, &mut slot);
            return Ok(snapshot(State::Retired {}));
        }
        let target = slot
            .target
            .as_ref()
            .context("synchronization target lost")?;
        let runtime = Arc::clone(&target.runtime);
        let route = Arc::clone(&target.route);
        let _route = route.lock().await;
        invocation.check_owner(&slot.owner)?;
        ensure!(
            runtime.is_running()?,
            "original synchronization runtime exited"
        );
        if action == NativeAction::Apply {
            ensure!(
                slot.until.is_some_and(|until| until > Instant::now()),
                "synchronization preparation expired"
            );
            // Commit uncertainty before the first I/O await. An observer loss
            // or deadline never makes Apply or its exclusion reusable.
            slot.attempted = true;
            slot.until = None;
            slot.state = State::Unknown {};
        } else if action == NativeAction::Retire {
            slot.retire_attempted = true;
        }
        let value = exchange(
            &runtime.socket,
            &json!({"type":"bufferSync","operation":slot.native,"action":action}),
        )
        .await?;
        let observed = reply(&value, &slot.content)?;
        ensure!(
            observed.operation == slot.native,
            "synchronization reply owner changed"
        );
        ensure!(
            if slot.attempted {
                observed.state != (State::Prepared {})
                    && (observed.state != (State::Retired {})
                        || slot.state.terminal() && slot.retire_attempted)
            } else {
                matches!(observed.state, State::Prepared {} | State::Retired {})
            },
            "synchronization outcome unavailable"
        );
        ensure!(
            !slot.state.terminal()
                || observed.state == slot.state
                || observed.state == (State::Retired {}),
            "synchronization terminal evidence changed"
        );
        ensure!(
            action != NativeAction::Retire || observed.state == (State::Retired {}),
            "synchronization retirement not observed"
        );
        if observed.state == (State::Retired {}) {
            self.registry.lock().retire(id, &entry, &mut slot);
        } else {
            slot.state = observed.state;
            if slot.state.terminal() {
                slot.fence = None;
            }
        }
        Ok(snapshot(slot.state.clone()))
    }
}

#[cfg(test)]
mod tests;

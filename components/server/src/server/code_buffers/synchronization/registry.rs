//! Finite Service continuations. HTTP identifiers are lookup keys, not grants.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use axum::http::StatusCode;
use parking_lot::Mutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::{Action, Content, NativeState, Snapshot, State};
use crate::machine_protocol::code_buffer_sync::OperationRef;
use crate::server::code_buffers::registry::{Binding, PREPARE_TTL, SyncFence};

const MAX_OPERATIONS: usize = 256;
const MAX_JOBS: usize = 64;

pub(super) struct Preparation {
    until: Instant,
    permit: OwnedSemaphorePermit,
}

pub(super) struct Prepared {
    pub resource: String,
    pub binding: Binding,
    pub fence: SyncFence,
    pub content: Content,
    pub operation: OperationRef,
}

struct Entry {
    resource: String,
    binding: Binding,
    fence: Option<SyncFence>,
    content: Content,
    operation: OperationRef,
    state: NativeState,
    until: Option<Instant>,
    busy: bool,
    attempted: bool,
    retire_attempted: bool,
    _permit: OwnedSemaphorePermit,
}

impl Entry {
    fn snapshot(&self, id: &str) -> Snapshot {
        Snapshot {
            api_version: 1,
            operation_id: id.to_owned(),
            resource_id: self.resource.clone(),
            purpose: super::Purpose::RefreshFromDisk,
            content: self.content.clone(),
            state: State::from(self.state.clone()),
            pending: self.busy,
        }
    }
}

struct Tombstone {
    user: String,
    snapshot: Snapshot,
    // Small identities only, to reject native-ID reuse. No connection, Session,
    // credential, runtime owner or capacity permit survives local retirement.
    machine: String,
    operation: OperationRef,
}

struct Slots {
    instance: String,
    last_id: u64,
    active: BTreeMap<String, Entry>,
    retired: VecDeque<Tombstone>,
}

impl Slots {
    fn retire(&mut self, id: &str, state: State) -> Snapshot {
        let entry = self.active.remove(id).expect("owned synchronization entry");
        let mut snapshot = entry.snapshot(id);
        snapshot.state = state;
        snapshot.pending = false;
        if self.retired.len() == MAX_OPERATIONS {
            self.retired.pop_front();
        }
        self.retired.push_back(Tombstone {
            user: entry.binding.user,
            snapshot: snapshot.clone(),
            machine: entry.binding.scope.machine_id().to_owned(),
            operation: entry.operation,
        });
        snapshot
    }

    fn expire(&mut self) {
        let ids: Vec<_> = self
            .active
            .iter()
            .filter(|(_, entry)| {
                !entry.busy
                    && !entry.attempted
                    && entry.until.is_some_and(|until| until <= Instant::now())
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.retire(&id, State::Expired {});
        }
    }
}

pub(in crate::server::code_buffers) struct Operations {
    slots: Mutex<Slots>,
    capacity: Arc<Semaphore>,
    jobs: Arc<Semaphore>,
    closed: AtomicBool,
}

impl Default for Operations {
    fn default() -> Self {
        Self {
            slots: Mutex::new(Slots {
                instance: uuid::Uuid::new_v4().simple().to_string(),
                last_id: 0,
                active: BTreeMap::new(),
                retired: VecDeque::new(),
            }),
            capacity: Arc::new(Semaphore::new(MAX_OPERATIONS)),
            jobs: Arc::new(Semaphore::new(MAX_JOBS)),
            closed: AtomicBool::new(false),
        }
    }
}

pub(super) enum Admission {
    Saved(Snapshot),
    Run(Box<Job>),
}

impl Operations {
    fn check_open(&self) -> Result<(), StatusCode> {
        if self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        Ok(())
    }

    pub(in crate::server::code_buffers) fn expire(&self) {
        self.slots.lock().expire();
    }

    pub(in crate::server::code_buffers) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub(super) fn reserve(&self) -> Result<Preparation, StatusCode> {
        self.check_open()?;
        self.expire();
        Ok(Preparation {
            until: Instant::now() + PREPARE_TTL,
            permit: Arc::clone(&self.capacity)
                .try_acquire_owned()
                .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?,
        })
    }

    pub(super) fn insert(
        &self,
        reservation: Preparation,
        prepared: Prepared,
    ) -> Result<Snapshot, StatusCode> {
        self.check_open()?;
        if reservation.until <= Instant::now() {
            return Err(StatusCode::CONFLICT);
        }
        let mut slots = self.slots.lock();
        let machine = prepared.binding.scope.machine_id();
        if slots.active.values().any(|entry| {
            entry.binding.scope.machine_id() == machine && entry.operation == prepared.operation
        }) || slots
            .retired
            .iter()
            .any(|entry| entry.machine == machine && entry.operation == prepared.operation)
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
        slots.last_id = slots
            .last_id
            .checked_add(1)
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let id = format!("sync-{}-{:016x}", slots.instance, slots.last_id);
        let entry = Entry {
            resource: prepared.resource,
            binding: prepared.binding,
            fence: Some(prepared.fence),
            content: prepared.content,
            operation: prepared.operation,
            state: NativeState::Prepared {},
            until: Some(reservation.until),
            busy: false,
            attempted: false,
            retire_attempted: false,
            _permit: reservation.permit,
        };
        let snapshot = entry.snapshot(&id);
        slots.active.insert(id, entry);
        Ok(snapshot)
    }

    pub(super) fn admit(
        self: &Arc<Self>,
        user: &str,
        id: &str,
        action: Action,
    ) -> Result<Admission, StatusCode> {
        self.check_open()?;
        let mut slots = self.slots.lock();
        slots.expire();
        if let Some(entry) = slots
            .retired
            .iter()
            .find(|entry| entry.snapshot.operation_id == id && entry.user == user)
        {
            return if action == Action::Apply {
                Err(StatusCode::CONFLICT)
            } else {
                Ok(Admission::Saved(entry.snapshot.clone()))
            };
        }
        let entry = slots
            .active
            .get_mut(id)
            .filter(|entry| entry.binding.user == user)
            .ok_or(StatusCode::NOT_FOUND)?;
        if entry.busy
            || action == Action::Apply && entry.attempted
            || action == Action::Retire && entry.retire_attempted
            || action == Action::Query && entry.state.terminal() && !entry.retire_attempted
        {
            return Ok(Admission::Saved(entry.snapshot(id)));
        }
        if action == Action::Apply
            && (entry.retire_attempted || entry.state != (NativeState::Prepared {}))
            || action == Action::Retire && entry.attempted && !entry.state.terminal()
        {
            return Err(StatusCode::CONFLICT);
        }
        let permit = Arc::clone(&self.jobs)
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        entry.busy = true;
        Ok(Admission::Run(Box::new(Job {
            registry: Arc::clone(self),
            id: id.to_owned(),
            binding: entry.binding.clone(),
            content: entry.content.clone(),
            operation: entry.operation.clone(),
            action,
            _permit: permit,
        })))
    }
}

pub(super) struct Job {
    registry: Arc<Operations>,
    id: String,
    pub binding: Binding,
    pub content: Content,
    pub operation: OperationRef,
    pub action: Action,
    _permit: OwnedSemaphorePermit,
}

impl Job {
    /// Authority checks precede this synchronous transition. Mark uncertainty
    /// before transport, including an enqueue failure or cancelled observer.
    pub(super) fn begin(&self) -> Result<(), StatusCode> {
        self.registry.check_open()?;
        let mut slots = self.registry.slots.lock();
        let entry = slots
            .active
            .get_mut(&self.id)
            .ok_or(StatusCode::NOT_FOUND)?;
        match self.action {
            Action::Apply => {
                if !entry.until.is_some_and(|until| until > Instant::now()) {
                    return Err(StatusCode::CONFLICT);
                }
                entry.attempted = true;
                entry.until = None;
                entry.state = NativeState::Unknown {};
            }
            Action::Retire => entry.retire_attempted = true,
            Action::Query => {}
        }
        Ok(())
    }

    pub(super) fn finish(&self, observed: NativeState) -> Result<Snapshot, StatusCode> {
        observed
            .validate(&self.content)
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        let mut slots = self.registry.slots.lock();
        let entry = slots
            .active
            .get_mut(&self.id)
            .ok_or(StatusCode::NOT_FOUND)?;
        let invalid = if entry.attempted {
            observed == (NativeState::Prepared {})
                || observed == (NativeState::Retired {})
                    && !(entry.state.terminal() && entry.retire_attempted)
        } else {
            !matches!(observed, NativeState::Prepared {} | NativeState::Retired {})
        };
        if invalid
            || entry.state.terminal()
                && observed != entry.state
                && observed != (NativeState::Retired {})
            || self.action == Action::Retire && observed != (NativeState::Retired {})
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
        if observed == (NativeState::Retired {}) {
            return Ok(slots.retire(&self.id, State::Retired {}));
        }
        entry.state = observed;
        if entry.state.terminal() {
            entry.fence = None;
        }
        // The job is still owned through the final asynchronous authority
        // check. Only its Drop may reopen admission: clearing busy here would
        // let a successor enter, then this older Drop clear the successor's
        // exclusion. The completed observation itself is no longer pending.
        let mut snapshot = entry.snapshot(&self.id);
        snapshot.pending = false;
        Ok(snapshot)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Some(entry) = self.registry.slots.lock().active.get_mut(&self.id) {
            entry.busy = false;
        }
    }
}

#[cfg(test)]
mod tests;

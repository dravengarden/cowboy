//! Bounded process-local ownership, independent of the HTTP observer. Only
//! inert preparations expire; active/unknown effects never undergo LRU eviction.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use axum::http::StatusCode;
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::remote::{Action, LeaseState, NativeRef};
use crate::core::SessionCodeScope;
use crate::machine_control::ConnectionToken;

const MAX_LEASES: usize = 1_024;
const MAX_JOBS: usize = 64;
pub(super) const PREPARE_TTL: Duration = Duration::from_secs(30);
pub(super) const JOB_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Snapshot {
    api_version: u8,
    pub resource_id: String,
    pub state: LeaseState,
    pub pending: bool,
}

impl Snapshot {
    fn new(id: &str, state: LeaseState, pending: bool) -> Self {
        Self {
            api_version: 1,
            resource_id: id.to_owned(),
            state,
            pending,
        }
    }
}

pub(super) struct Reservation {
    pub until: Instant,
    permit: OwnedSemaphorePermit,
}

#[derive(Clone)]
pub(super) struct Binding {
    pub user: String,
    pub scope: SessionCodeScope,
    pub connection: ConnectionToken,
    pub native: NativeRef,
}

struct Entry {
    binding: Binding,
    until: Option<Instant>,
    state: LeaseState,
    busy: bool,
    open_attempted: bool,
    release_attempted: bool,
    synchronizing: Arc<AtomicBool>,
    _permit: OwnedSemaphorePermit,
}

struct Slots {
    instance: String,
    last_id: u64,
    active: BTreeMap<String, Entry>,
    // Only terminal local receipts. No Session, connection, native reference,
    // credential or process is retained by this bounded observation cache.
    released: VecDeque<(String, String)>,
}

impl Slots {
    fn retire(&mut self, id: &str) {
        if let Some(entry) = self.active.remove(id) {
            if self.released.len() == MAX_LEASES {
                self.released.pop_front();
            }
            self.released.push_back((id.to_owned(), entry.binding.user));
        }
    }

    fn expire(&mut self) {
        let expired: Vec<_> = self
            .active
            .iter()
            .filter(|(_, entry)| {
                !entry.busy && entry.until.is_some_and(|until| until <= Instant::now())
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.retire(&id);
        }
    }
}

pub(in crate::server) struct Owners {
    pub(super) synchronizations: Arc<super::synchronization::registry::Operations>,
    slots: Mutex<Slots>,
    capacity: Arc<Semaphore>,
    job_capacity: Arc<Semaphore>,
    tasks: Mutex<JoinSet<()>>,
    closed: AtomicBool,
}

impl Default for Owners {
    fn default() -> Self {
        Self {
            synchronizations: Arc::default(),
            slots: Mutex::new(Slots {
                instance: uuid::Uuid::new_v4().simple().to_string(),
                last_id: 0,
                active: BTreeMap::new(),
                released: VecDeque::new(),
            }),
            capacity: Arc::new(Semaphore::new(MAX_LEASES)),
            job_capacity: Arc::new(Semaphore::new(MAX_JOBS)),
            tasks: Mutex::new(JoinSet::new()),
            closed: AtomicBool::new(false),
        }
    }
}

pub(super) enum Admission {
    Saved(Snapshot),
    Run(Box<Job>),
}

impl Owners {
    pub(super) fn reserve(&self) -> Result<Reservation, StatusCode> {
        if self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        self.slots.lock().expire();
        Ok(Reservation {
            until: Instant::now() + PREPARE_TTL,
            permit: self
                .capacity
                .clone()
                .try_acquire_owned()
                .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?,
        })
    }

    pub(super) fn insert(
        &self,
        reservation: Reservation,
        binding: Binding,
    ) -> Result<Snapshot, StatusCode> {
        if reservation.until <= Instant::now() || self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::CONFLICT);
        }
        let mut slots = self.slots.lock();
        slots.last_id = slots
            .last_id
            .checked_add(1)
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let id = format!("{}-{:016x}", slots.instance, slots.last_id);
        // A misbehaving peer must not bind one native owner to two core users.
        if slots.active.values().any(|entry| {
            entry.binding.native == binding.native
                && entry.binding.scope.machine_id() == binding.scope.machine_id()
        }) {
            return Err(StatusCode::BAD_GATEWAY);
        }
        slots.active.insert(
            id.clone(),
            Entry {
                binding,
                until: Some(reservation.until),
                state: LeaseState::Prepared,
                busy: false,
                open_attempted: false,
                release_attempted: false,
                synchronizing: Arc::new(AtomicBool::new(false)),
                _permit: reservation.permit,
            },
        );
        Ok(Snapshot::new(&id, LeaseState::Prepared, false))
    }

    pub(super) fn admit(
        self: &Arc<Self>,
        user: &str,
        id: &str,
        action: Action,
    ) -> Result<Admission, StatusCode> {
        if self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        self.synchronizations.expire();
        let mut slots = self.slots.lock();
        slots.expire();
        if slots
            .released
            .iter()
            .any(|(key, owner)| key == id && owner == user)
        {
            return if action == Action::Open {
                Err(StatusCode::CONFLICT)
            } else {
                Ok(Admission::Saved(Snapshot::new(
                    id,
                    LeaseState::Released,
                    false,
                )))
            };
        }
        let entry = slots
            .active
            .get_mut(id)
            .filter(|entry| entry.binding.user == user)
            .ok_or(StatusCode::NOT_FOUND)?;
        if action != Action::Query && entry.synchronizing.load(Ordering::Acquire) {
            return Err(StatusCode::CONFLICT);
        }
        if entry.busy
            || (action == Action::Open && entry.open_attempted)
            || (action == Action::Release && entry.release_attempted)
        {
            return Ok(Admission::Saved(Snapshot::new(id, entry.state, entry.busy)));
        }
        if action == Action::Open
            && (entry.release_attempted || entry.state != LeaseState::Prepared)
            || action == Action::Release && entry.state == LeaseState::Unknown
        {
            // An ambiguous open/release is observed, never inverted or replayed.
            return Err(StatusCode::CONFLICT);
        }
        let permit = self
            .job_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        entry.busy = true;
        Ok(Admission::Run(Box::new(Job {
            owners: Arc::clone(self),
            id: id.to_owned(),
            binding: entry.binding.clone(),
            action,
            finished: false,
            _permit: permit,
        })))
    }

    /// The task owner, not the observer, retains this admitted continuation.
    /// The owner bounds the entire task, including authorization/queue time.
    pub(super) fn spawn<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = Result<T, StatusCode>> + Send + 'static,
    ) -> Result<oneshot::Receiver<Result<T, StatusCode>>, StatusCode> {
        let mut tasks = self.tasks.lock();
        if self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        while tasks.try_join_next().is_some() {}
        let (sender, receiver) = oneshot::channel();
        tasks.spawn(async move {
            let result = tokio::time::timeout(JOB_TIMEOUT, future)
                .await
                .unwrap_or(Err(StatusCode::GATEWAY_TIMEOUT));
            let _ = sender.send(result);
        });
        Ok(receiver)
    }

    pub(super) fn admit_read(
        self: &Arc<Self>,
        user: &str,
        id: &str,
    ) -> Result<ReadJob, StatusCode> {
        if self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        self.synchronizations.expire();
        let mut slots = self.slots.lock();
        slots.expire();
        let entry = slots
            .active
            .get_mut(id)
            .filter(|entry| entry.binding.user == user)
            .ok_or(StatusCode::NOT_FOUND)?;
        if entry.busy
            || entry.state != LeaseState::Open
            || entry.release_attempted
            || entry.synchronizing.load(Ordering::Acquire)
        {
            return Err(StatusCode::CONFLICT);
        }
        let permit = self
            .job_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        entry.busy = true;
        Ok(ReadJob {
            owners: Arc::clone(self),
            id: id.to_owned(),
            binding: entry.binding.clone(),
            _permit: permit,
        })
    }

    pub(super) fn synchronization_owner(
        &self,
        user: &str,
        id: &str,
    ) -> Result<(Binding, SyncFence), StatusCode> {
        if self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        self.synchronizations.expire();
        let mut slots = self.slots.lock();
        slots.expire();
        let entry = slots
            .active
            .get(id)
            .filter(|entry| entry.binding.user == user)
            .ok_or(StatusCode::NOT_FOUND)?;
        // A query reporting Open is not our admitted open. Never let an inert
        // buffer reservation expire while an effect borrows its ownership.
        if entry.busy
            || entry.state != LeaseState::Open
            || !entry.open_attempted
            || entry.until.is_some()
            || entry.release_attempted
            || entry.synchronizing.load(Ordering::Acquire)
        {
            return Err(StatusCode::CONFLICT);
        }
        entry.synchronizing.store(true, Ordering::Release);
        Ok((
            entry.binding.clone(),
            SyncFence(Arc::clone(&entry.synchronizing)),
        ))
    }

    pub(in crate::server) async fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        self.synchronizations.close();
        let mut tasks = std::mem::take(&mut *self.tasks.lock());
        tasks.shutdown().await;
    }
}

/// Retained by one finite synchronization, not by its HTTP observer. Drop only
/// after an inert preparation ends or exact terminal evidence clears the fence.
pub(super) struct SyncFence(Arc<AtomicBool>);

impl Drop for SyncFence {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// A borrow, never an effect transition. Releasing the borrow after success,
// error, cancellation or deadline leaves the last native observation intact.
pub(super) struct ReadJob {
    owners: Arc<Owners>,
    id: String,
    pub binding: Binding,
    _permit: OwnedSemaphorePermit,
}

impl Drop for ReadJob {
    fn drop(&mut self) {
        if let Some(entry) = self.owners.slots.lock().active.get_mut(&self.id) {
            entry.busy = false;
        }
    }
}

pub(super) struct Job {
    owners: Arc<Owners>,
    id: String,
    pub binding: Binding,
    pub action: Action,
    finished: bool,
    _permit: OwnedSemaphorePermit,
}

impl Job {
    /// Called only after fresh authority and original-scope checks. Mark the
    /// potential effect before the first transport await, including cancellation.
    pub(super) fn begin(&self) -> Result<(), StatusCode> {
        let mut slots = self.owners.slots.lock();
        let entry = slots
            .active
            .get_mut(&self.id)
            .ok_or(StatusCode::NOT_FOUND)?;
        if self.owners.closed.load(Ordering::Acquire)
            || entry.until.is_some_and(|until| until <= Instant::now())
        {
            return Err(StatusCode::CONFLICT);
        }
        match self.action {
            Action::Open => {
                entry.open_attempted = true;
                entry.until = None;
            }
            Action::Release => {
                entry.release_attempted = true;
                entry.until = None;
            }
            Action::Query => {}
        }
        // A mutation's missing reply cannot preserve a claim that the prior
        // state still holds. Read failures retain only the last observation.
        if self.action != Action::Query {
            entry.state = LeaseState::Unknown;
        }
        Ok(())
    }

    pub(super) fn finish(mut self, state: LeaseState) -> Result<Snapshot, StatusCode> {
        let mut slots = self.owners.slots.lock();
        let entry = slots
            .active
            .get_mut(&self.id)
            .ok_or(StatusCode::NOT_FOUND)?;
        if entry.state == LeaseState::Open && state == LeaseState::Prepared {
            return Err(StatusCode::BAD_GATEWAY);
        }
        entry.state = state;
        entry.busy = false;
        if state == LeaseState::Released {
            slots.retire(&self.id);
        }
        self.finished = true;
        Ok(Snapshot::new(&self.id, state, false))
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if !self.finished
            && let Some(entry) = self.owners.slots.lock().active.get_mut(&self.id)
        {
            entry.busy = false;
        }
    }
}

#[cfg(test)]
mod tests;

//! Bounded owners, not a graph interpreter. Unknown effects are never expired
//! or replayed. Destination preparations retain their original lookup/capacity
//! before dispatch and enter the existing ordinary buffer owner synchronously.
use super::{
    Action, Content, DestinationSnapshot, DestinationState, NativeAction, Prepare, Snapshot, State,
};
use crate::machine_protocol::code_buffer_navigation::{
    BufferRef, NavigationRef, Snapshot as NativeSnapshot,
};
use crate::server::code_buffers::registry::{Binding, Owners, PREPARE_TTL, Reservation};
use axum::http::StatusCode;
use parking_lot::Mutex;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

const MAX_GROUPS: usize = 32;
const MAX_JOBS: usize = 64;

pub(super) struct Preparation {
    until: Instant,
    permit: OwnedSemaphorePermit,
}

struct Destination {
    content: Content,
    attempted: bool,
    reservation: Option<Reservation>,
    native: Option<BufferRef>,
    snapshot: DestinationSnapshot,
}

struct Entry {
    binding: Binding,
    navigation: NavigationRef,
    snapshot: Snapshot,
    destinations: BTreeMap<u32, Destination>,
    until: Option<Instant>,
    attempted: bool,
    release_attempted: bool,
    busy: bool,
    _permit: OwnedSemaphorePermit,
}

impl Entry {
    fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.snapshot.clone();
        snapshot.pending = self.busy;
        snapshot.destinations = self
            .destinations
            .values()
            .map(|entry| entry.snapshot.clone())
            .collect();
        snapshot
    }

    fn expire_destinations(&mut self) {
        for entry in self.destinations.values_mut() {
            if entry
                .reservation
                .as_ref()
                .is_some_and(|reservation| reservation.until <= Instant::now())
            {
                entry.reservation = None;
                entry.snapshot.state = DestinationState::Expired;
            }
        }
    }
}

struct Tombstone {
    user: String,
    machine: String,
    navigation: NavigationRef,
    snapshot: Snapshot,
}

struct Slots {
    instance: String,
    last_id: u64,
    active: BTreeMap<String, Entry>,
    retired: VecDeque<Tombstone>,
}

impl Slots {
    fn retire(&mut self, id: &str, state: State) -> Snapshot {
        let mut entry = self.active.remove(id).expect("owned navigation");
        for destination in entry.destinations.values_mut() {
            if destination.reservation.take().is_some() {
                destination.snapshot.state = DestinationState::Expired;
            }
        }
        let mut snapshot = entry.snapshot();
        snapshot.state = state;
        snapshot.pending = false;
        if self.retired.len() == MAX_GROUPS {
            self.retired.pop_front();
        }
        self.retired.push_back(Tombstone {
            user: entry.binding.user,
            machine: entry.binding.scope.machine_id().to_owned(),
            navigation: entry.navigation,
            snapshot: snapshot.clone(),
        });
        snapshot
    }

    fn expire(&mut self) {
        let mut expired = Vec::new();
        for (id, entry) in &mut self.active {
            entry.expire_destinations();
            if !entry.busy
                && !entry.attempted
                && !entry.release_attempted
                && entry.until.is_some_and(|until| until <= Instant::now())
            {
                expired.push(id.clone());
            }
        }
        for id in expired {
            self.retire(&id, State::Expired);
        }
    }
}

pub(in crate::server::code_buffers) struct Groups {
    slots: Mutex<Slots>,
    capacity: Arc<Semaphore>,
    jobs: Arc<Semaphore>,
    closed: AtomicBool,
}

impl Default for Groups {
    fn default() -> Self {
        Self {
            slots: Mutex::new(Slots {
                instance: uuid::Uuid::new_v4().simple().to_string(),
                last_id: 0,
                active: BTreeMap::new(),
                retired: VecDeque::new(),
            }),
            capacity: Arc::new(Semaphore::new(MAX_GROUPS)),
            jobs: Arc::new(Semaphore::new(MAX_JOBS)),
            closed: AtomicBool::new(false),
        }
    }
}

pub(super) enum Admission {
    Saved(Box<Snapshot>, Option<Box<Binding>>),
    Run(Box<Job>),
}

impl Groups {
    fn check_open(&self) -> Result<(), StatusCode> {
        if self.closed.load(Ordering::Acquire) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        Ok(())
    }

    pub(in crate::server::code_buffers) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub(in crate::server::code_buffers) fn expire(&self) {
        self.slots.lock().expire();
    }

    pub(super) fn reserve(&self) -> Result<Preparation, StatusCode> {
        self.check_open()?;
        self.slots.lock().expire();
        Ok(Preparation {
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
        reservation: Preparation,
        resource: String,
        binding: Binding,
        prepare: Prepare,
        navigation: NavigationRef,
    ) -> Result<Snapshot, StatusCode> {
        self.check_open()?;
        if reservation.until <= Instant::now() {
            return Err(StatusCode::CONFLICT);
        }
        let mut slots = self.slots.lock();
        let machine = binding.scope.machine_id();
        if slots.active.values().any(|entry| {
            entry.binding.scope.machine_id() == machine && entry.navigation == navigation
        }) || slots
            .retired
            .iter()
            .any(|entry| entry.machine == machine && entry.navigation == navigation)
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
        slots.last_id = slots
            .last_id
            .checked_add(1)
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
        let id = format!("nav-{}-{:016x}", slots.instance, slots.last_id);
        let snapshot = Snapshot {
            api_version: 1,
            navigation_id: id.clone(),
            source_resource_id: resource,
            content: prepare.content,
            position: prepare.position,
            query: prepare.query,
            state: State::Prepared,
            locations: Vec::new(),
            destinations: Vec::new(),
            pending: false,
        };
        slots.active.insert(
            id,
            Entry {
                binding,
                navigation,
                snapshot: snapshot.clone(),
                destinations: BTreeMap::new(),
                until: Some(reservation.until),
                attempted: false,
                release_attempted: false,
                busy: false,
                _permit: reservation.permit,
            },
        );
        Ok(snapshot)
    }

    pub(super) fn admit(
        self: &Arc<Self>,
        owners: &Owners,
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
            .find(|entry| entry.snapshot.navigation_id == id && entry.user == user)
        {
            return if action.acquisition() {
                Err(StatusCode::CONFLICT)
            } else {
                Ok(Admission::Saved(Box::new(entry.snapshot.clone()), None))
            };
        }
        let entry = slots
            .active
            .get_mut(id)
            .filter(|entry| entry.binding.user == user)
            .ok_or(StatusCode::NOT_FOUND)?;
        if let Action::Destination {
            destination,
            content,
        } = &action
        {
            if entry.snapshot.state != State::Retained
                || entry.release_attempted
                || entry
                    .snapshot
                    .locations
                    .get(*destination as usize)
                    .is_none_or(|location| location.content != *content)
            {
                return Err(StatusCode::CONFLICT);
            }
            if let Some(previous) = entry.destinations.get(destination) {
                if previous.content != *content {
                    return Err(StatusCode::CONFLICT);
                }
                return Ok(Admission::Saved(
                    Box::new(entry.snapshot()),
                    Some(Box::new(entry.binding.clone())),
                ));
            }
        }
        if entry.busy
            || action == Action::Execute && entry.attempted
            || action == Action::Release && entry.release_attempted
        {
            return Ok(Admission::Saved(
                Box::new(entry.snapshot()),
                Some(Box::new(entry.binding.clone())),
            ));
        }
        if action == Action::Execute
            && (entry.release_attempted || entry.snapshot.state != State::Prepared)
            || action == Action::Release && entry.snapshot.state == State::Unknown
        {
            return Err(StatusCode::CONFLICT);
        }
        let permit = self
            .jobs
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        if let Action::Destination {
            destination,
            content,
        } = &action
        {
            // Lock order is navigation -> ordinary; ordinary owners never call
            // back into this registry while holding their slots lock.
            let reservation = owners.reserve_navigation()?;
            entry.destinations.insert(
                *destination,
                Destination {
                    content: content.clone(),
                    attempted: false,
                    reservation: Some(reservation),
                    native: None,
                    snapshot: DestinationSnapshot {
                        destination: *destination,
                        state: DestinationState::Pending,
                        resource_id: None,
                    },
                },
            );
        }
        entry.busy = true;
        Ok(Admission::Run(Box::new(Job {
            registry: Arc::clone(self),
            id: id.to_owned(),
            resource: entry.snapshot.source_resource_id.clone(),
            binding: entry.binding.clone(),
            navigation: entry.navigation.clone(),
            action,
            _permit: permit,
        })))
    }
}

pub(super) struct Job {
    registry: Arc<Groups>,
    id: String,
    pub resource: String,
    pub binding: Binding,
    navigation: NavigationRef,
    pub action: Action,
    _permit: OwnedSemaphorePermit,
}

impl Job {
    pub(super) fn native_action(&self) -> NativeAction {
        let navigation = self.navigation.clone();
        match &self.action {
            Action::Execute => NativeAction::Execute { navigation },
            Action::Query => NativeAction::Query { navigation },
            Action::Release => NativeAction::Release { navigation },
            Action::Destination {
                destination,
                content,
            } => NativeAction::PrepareDestination {
                navigation,
                destination: *destination,
                content: content.clone(),
            },
        }
    }

    pub(super) fn begin(&self) -> Result<(), StatusCode> {
        self.registry.check_open()?;
        let mut slots = self.registry.slots.lock();
        let entry = slots
            .active
            .get_mut(&self.id)
            .ok_or(StatusCode::NOT_FOUND)?;
        if entry.until.is_some_and(|until| until <= Instant::now()) {
            return Err(StatusCode::CONFLICT);
        }
        match &self.action {
            Action::Execute => {
                entry.attempted = true;
                entry.until = None;
                entry.snapshot.state = State::Unknown;
            }
            Action::Release => {
                entry.release_attempted = true;
                entry.until = None;
                entry.snapshot.state = State::ReleaseUnknown;
            }
            Action::Destination { destination, .. } => {
                let target = entry
                    .destinations
                    .get_mut(destination)
                    .ok_or(StatusCode::CONFLICT)?;
                if !target
                    .reservation
                    .as_ref()
                    .is_some_and(|reservation| reservation.until > Instant::now())
                {
                    return Err(StatusCode::CONFLICT);
                }
                target.attempted = true;
                target.snapshot.state = DestinationState::Unknown;
            }
            Action::Query => {}
        }
        Ok(())
    }

    pub(super) fn finish(
        &self,
        owners: &Owners,
        observed: NativeSnapshot,
    ) -> Result<Snapshot, StatusCode> {
        observed.validate().map_err(|_| StatusCode::BAD_GATEWAY)?;
        if observed.navigation != self.navigation {
            return Err(StatusCode::BAD_GATEWAY);
        }
        let mut slots = self.registry.slots.lock();
        let entry = slots
            .active
            .get_mut(&self.id)
            .ok_or(StatusCode::NOT_FOUND)?;
        let next = State::from(observed.phase);
        let valid = match entry.snapshot.state {
            State::Prepared => next == State::Prepared,
            State::Unknown => matches!(next, State::Unknown | State::Retained),
            State::Retained => next == State::Retained,
            State::ReleaseUnknown => matches!(next, State::ReleaseUnknown | State::Released),
            State::Released | State::Expired => false,
        };
        if !valid
            || !entry.attempted && !observed.locations.is_empty()
            || matches!(
                entry.snapshot.state,
                State::Retained | State::ReleaseUnknown
            ) && observed.locations != entry.snapshot.locations
        {
            return Err(StatusCode::BAD_GATEWAY);
        }
        // Validate the entire receipt before committing any ordinary owner.
        for destination in &observed.destinations {
            let target = entry
                .destinations
                .get(&destination.destination)
                .ok_or(StatusCode::BAD_GATEWAY)?;
            if !target.attempted
                || target
                    .native
                    .as_ref()
                    .is_some_and(|lease| lease != &destination.lease)
            {
                return Err(StatusCode::BAD_GATEWAY);
            }
        }
        if entry.destinations.iter().any(|(index, target)| {
            target.native.is_some()
                && !observed
                    .destinations
                    .iter()
                    .any(|destination| destination.destination == *index)
        }) {
            return Err(StatusCode::BAD_GATEWAY);
        }
        entry.expire_destinations();
        for destination in observed.destinations {
            let target = entry
                .destinations
                .get_mut(&destination.destination)
                .expect("validated destination");
            target.native = Some(destination.lease.clone());
            if let Some(reservation) = target.reservation.take() {
                let binding = Binding {
                    native: destination.lease,
                    ..entry.binding.clone()
                };
                // An inert preparation can fail/expire but cannot be renewed.
                match owners.insert(reservation, binding) {
                    Ok(snapshot) => {
                        target.snapshot.resource_id = Some(snapshot.resource_id);
                        target.snapshot.state = DestinationState::Prepared;
                    }
                    Err(_) => {
                        target.snapshot.state = DestinationState::Expired;
                    }
                }
            }
        }
        entry.snapshot.state = next;
        entry.snapshot.locations = observed.locations;
        if next == State::Released {
            return Ok(slots.retire(&self.id, State::Released));
        }
        let mut snapshot = entry.snapshot();
        // Keep exclusion until Job Drop, including the final auth await.
        snapshot.pending = false;
        Ok(snapshot)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if let Some(entry) = self.registry.slots.lock().active.get_mut(&self.id) {
            entry.busy = false;
            if let Action::Destination { destination, .. } = self.action
                && entry
                    .destinations
                    .get(&destination)
                    .is_some_and(|target| !target.attempted)
            {
                entry.destinations.remove(&destination);
            }
        }
    }
}

#[cfg(test)]
mod tests;

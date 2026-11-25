//! In-memory Controller view of Machine browsers: per-Machine capability,
//! the latest state of each account's browser on each Machine, pending
//! stream pairings and live viewer attachments. Everything here is re-learned from Machine
//! hellos and state events and is scoped to the connection that reported
//! it: a reconnected (possibly rebooted) Machine starts from nothing. The
//! durable placement lives in the store.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::WebSocket;
use tokio::sync::{oneshot, watch};

use crate::machine_control::ConnectionToken;
use crate::machine_protocol::machine_browser::{
    Capability, Event, InstanceRef, PAIRING_TIMEOUT_MS, STREAM_TOKEN_TTL_MS, State, StateEvent,
};
use crate::provider_account::ProviderAccountId;

/// The latest observation of one account's browser, on the connection that
/// reported it.
#[derive(Clone, Debug)]
pub struct Observed {
    pub machine_id: String,
    pub connection: ConnectionToken,
    pub event: StateEvent,
    pub observed_at: Instant,
}

impl PartialEq for Observed {
    fn eq(&self, other: &Self) -> bool {
        self.machine_id == other.machine_id
            && self.connection.same(&other.connection)
            && self.event == other.event
            && self.observed_at == other.observed_at
    }
}

impl Eq for Observed {}

impl Observed {
    pub fn running(&self) -> bool {
        matches!(
            self.event.state,
            State::Starting | State::Detached | State::Attached
        )
    }

    /// A process may exist: running or still stopping.
    pub fn live(&self) -> bool {
        self.running() || self.event.state == State::Stopping
    }

    pub fn instance(&self) -> InstanceRef {
        self.event.instance.clone()
    }
}

/// A viewer waiting for the Machine's half of its display stream.
pub struct Pairing {
    pub created: Instant,
    pub connection: ConnectionToken,
    pub deliver: oneshot::Sender<WebSocket>,
}

struct Viewer {
    /// The owner whose card attached.
    owner: String,
    /// The identity whose card attached: the account itself, or a member
    /// of the share whose browser this is.
    member: String,
    /// The browser identity (`own` or `share:<id>:<rev>`) confirmed at attach
    /// time, enforced for cached clients that send `attachment_id` without
    /// `expected_browser`.
    identity: String,
    attachment_id: String,
    incarnation: u64,
    connection: ConnectionToken,
    /// Registered before the Machine confirmed the attachment: only an
    /// incarnation change or the instance stopping ends it meanwhile.
    pending: bool,
    cancel: watch::Sender<bool>,
}

/// A browser is identified per Machine and account: the same account may
/// run a browser on several Machines, each with its own instance.
pub type Key = (String, String);

fn key(machine_id: &str, account_id: &str) -> Key {
    (machine_id.to_owned(), account_id.to_owned())
}

type Locks = parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

/// What a Machine's current connection has not yet answered for: a
/// reconnected Machine's surviving browsers are unknown until each durable
/// placement there is answered authoritatively on that connection.
struct Inventory {
    connection: ConnectionToken,
    /// Placed accounts still unanswered; `None` until the placements load.
    pending: Option<HashSet<ProviderAccountId>>,
    /// Accounts answered before the placements loaded.
    answered: HashSet<ProviderAccountId>,
}

#[derive(Default)]
pub struct Coordinator {
    /// The current connection of each Machine, as of its hello.
    connections: parking_lot::RwLock<HashMap<String, ConnectionToken>>,
    /// Per Machine: the browser capability its current connection reported,
    /// with that connection, so a reader can bind it to the exact connection
    /// a command will travel on.
    capabilities: parking_lot::RwLock<HashMap<String, (ConnectionToken, Capability)>>,
    /// Per Machine and account: the latest observation of that Machine's
    /// instance. One Machine's report never replaces another's.
    states: parking_lot::RwLock<HashMap<Key, Observed>>,
    pairings: parking_lot::Mutex<HashMap<String, Pairing>>,
    /// Per Machine and account: the viewer of that Machine's instance.
    viewers: parking_lot::Mutex<HashMap<Key, Viewer>>,
    /// Per account: attachment admission and viewer registration.
    attach_locks: Locks,
    /// Per account: open dispatch against revocation dispatch.
    lifecycle_locks: Locks,
    /// Per owner: share membership changes against share resets, opens, and
    /// identity-bound instance commands (`expected_browser`).
    share_locks: Locks,
    /// Per Machine: placements its current connection has not answered for.
    inventories: parking_lot::Mutex<HashMap<String, Inventory>>,
    /// Identity-bound navigations dispatched to a Machine and not yet settled
    /// (including those whose Controller wait timed out, whose transport
    /// dropped, or that were reloaded after a Controller restart). Keyed by
    /// `request_id`.
    pending_navigations: parking_lot::Mutex<HashMap<String, PendingNavigation>>,
    /// Request ids settled in memory by synchronous observation callbacks and
    /// waiting to be deleted from `browser_pending_navigations` in the store.
    settled_navigation_ids: parking_lot::Mutex<HashSet<String>>,
    /// Signalled whenever any pending navigation settles so durable fence
    /// deletions and deferred revocation share-membership removals run promptly.
    navigation_settled: Arc<tokio::sync::Notify>,
}

#[derive(Clone, Debug)]
pub struct PendingNavigation {
    pub request_id: String,
    pub owner: String,
    pub browser_key: ProviderAccountId,
    pub machine_id: String,
    /// `Some` when dispatched in this Controller process; `None` when seeded
    /// from durable storage after a Controller restart.
    pub connection: Option<ConnectionToken>,
    pub instance: InstanceRef,
    pub observed_at_ms: i64,
}

/// What the cache keeps of a state event. A reason on a running instance's
/// state is the outcome of the command that state answers (a detach of an
/// attachment that had already ended is `superseded`, a refused resize
/// `failed`, an attach during a recheck `busy`), never a condition of the
/// instance: kept with it, every later read would report that command's
/// outcome as the browser's own state. The command's caller still receives
/// it with the answer itself.
fn observation(state: &StateEvent) -> StateEvent {
    let mut event = state.clone();
    if matches!(
        event.state,
        State::Starting | State::Detached | State::Attached
    ) {
        event.reason = None;
    }
    event
}

fn lock_for(locks: &Locks, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    Arc::clone(locks.lock().entry(account_id.to_owned()).or_default())
}

impl Coordinator {
    /// A Machine connection was installed: everything learned on any
    /// earlier connection of that Machine is gone, and its viewers end.
    pub fn connected(&self, connection: &ConnectionToken, capability: Option<Capability>) {
        let machine_id = connection.machine_id().to_owned();
        self.connections
            .write()
            .insert(machine_id.clone(), connection.clone());
        self.observe_capability(connection, capability);
        self.inventories.lock().insert(
            machine_id.clone(),
            Inventory {
                connection: connection.clone(),
                pending: None,
                answered: HashSet::new(),
            },
        );
        self.states
            .write()
            .retain(|(machine, _), _| *machine != machine_id);
        self.cancel_viewers_where(|(machine, _), viewer| {
            *machine == machine_id && !viewer.connection.same(connection)
        });
    }

    /// A Machine connection ended: drop what it reported and end its viewers.
    pub fn disconnected(&self, connection: &ConnectionToken) {
        let machine_id = connection.machine_id();
        let current = {
            let mut connections = self.connections.write();
            let current = connections
                .get(machine_id)
                .is_some_and(|known| known.same(connection));
            if current {
                connections.remove(machine_id);
            }
            current
        };
        if current {
            self.observe_capability(connection, None);
        }
        let mut inventories = self.inventories.lock();
        if inventories
            .get(machine_id)
            .is_some_and(|inventory| inventory.connection.same(connection))
        {
            inventories.remove(machine_id);
        }
        drop(inventories);
        self.states
            .write()
            .retain(|_, observed| !observed.connection.same(connection));
        self.cancel_viewers_where(|_, viewer| viewer.connection.same(connection));
    }

    fn is_current(&self, connection: &ConnectionToken) -> bool {
        self.connections
            .read()
            .get(connection.machine_id())
            .is_some_and(|known| known.same(connection))
    }

    pub fn observe_capability(&self, connection: &ConnectionToken, capability: Option<Capability>) {
        let machine_id = connection.machine_id();
        let mut capabilities = self.capabilities.write();
        match capability {
            Some(capability) => {
                capabilities.insert(machine_id.to_owned(), (connection.clone(), capability));
            }
            None => {
                capabilities.remove(machine_id);
            }
        }
    }

    pub fn capability(&self, machine_id: &str) -> Option<Capability> {
        self.capabilities
            .read()
            .get(machine_id)
            .map(|(_, capability)| capability.clone())
    }

    /// The capability `connection` itself reported; `None` once its Machine
    /// reported none or another connection replaced it, so nothing learned
    /// on one connection ever vouches for another.
    pub fn capability_on(&self, connection: &ConnectionToken) -> Option<Capability> {
        self.capabilities
            .read()
            .get(connection.machine_id())
            .filter(|(known, _)| known.same(connection))
            .map(|(_, capability)| capability.clone())
    }

    /// The Machines whose current connection reported an available browser
    /// capability, sorted. A disconnect removes its Machine.
    pub fn available_machines(&self) -> Vec<String> {
        let mut machines: Vec<String> = self
            .capabilities
            .read()
            .iter()
            .filter(|(_, (_, capability))| capability.available)
            .map(|(machine, _)| machine.clone())
            .collect();
        machines.sort();
        machines
    }

    /// Consume one Machine browser event from the current connection. It
    /// describes that Machine's instance only. A state that no longer
    /// matches a live viewer's attachment or incarnation ends that viewer.
    /// An answer naming no instance (a refused reset, a stopped browser, a
    /// `stopping`/`busy` for a unit of unknown owner) never hides a live
    /// observation here; only `record_authoritative` accepts it over one. A terminal `hibernated` for the cached incarnation (a
    /// confirmed exit after `stopping`) always replaces it.
    pub fn observe(&self, connection: &ConnectionToken, event: &Event) {
        if !self.is_current(connection) {
            return;
        }
        let machine_id = connection.machine_id();
        match event {
            Event::State(state) => {
                let key = key(machine_id, state.instance.account_id.as_str());
                let running = matches!(
                    state.state,
                    State::Starting | State::Detached | State::Attached
                );
                {
                    let mut states = self.states.write();
                    let replaces = states.get(&key).is_none_or(|previous| {
                        // A hibernated answer for an older incarnation cannot
                        // hide a newer live instance.
                        !previous.connection.same(connection)
                            || running
                            || (!state.instance.instance_id.is_empty()
                                && previous.event.instance.incarnation
                                    <= state.instance.incarnation)
                            || !previous.live()
                    });
                    if replaces {
                        states.insert(
                            key.clone(),
                            Observed {
                                machine_id: machine_id.to_owned(),
                                connection: connection.clone(),
                                event: observation(state),
                                observed_at: Instant::now(),
                            },
                        );
                    }
                }
                self.end_mismatched_viewer(&key, state);
                self.settle_navigations_from_state(connection, state, false);
            }
            Event::Capability(capability) => {
                self.observe_capability(connection, Some(capability.clone()));
            }
            Event::CapabilityLost => {
                self.observe_capability(connection, None);
                self.cancel_viewers_where(|(machine, _), _| machine == machine_id);
            }
        }
    }

    fn end_mismatched_viewer(&self, key: &Key, state: &StateEvent) {
        let mut viewers = self.viewers.lock();
        let Some(viewer) = viewers.get(key) else {
            return;
        };
        if state.instance.instance_id.is_empty() || state.instance.incarnation < viewer.incarnation
        {
            return;
        }
        let stopped = !matches!(state.state, State::Detached | State::Attached);
        let replaced = state.instance.incarnation != viewer.incarnation
            || stopped
            || (!viewer.pending
                && (state.state != State::Attached
                    || state.attachment_id.as_deref() != Some(viewer.attachment_id.as_str())));
        if replaced {
            let _ = viewer.cancel.send(true);
            viewers.remove(key);
        }
    }

    /// The Machine's direct answer about an account's browser on that
    /// Machine (a query, a confirmed reset or revocation): it replaces the
    /// cached state of that Machine's instance, even a live one, unless a
    /// newer observation already arrived on the same connection. An answer
    /// naming no instance means none runs there: its viewer ends.
    pub fn record_authoritative(&self, connection: &ConnectionToken, event: &StateEvent) {
        if !self.is_current(connection) {
            return;
        }
        let key = key(connection.machine_id(), event.instance.account_id.as_str());
        // Either way the connection has answered for the account.
        self.answered(connection, &event.instance.account_id);
        {
            let mut states = self.states.write();
            let newer = states.get(&key).is_some_and(|previous| {
                previous.connection.same(connection)
                    && previous.event.observed_at_ms > event.observed_at_ms
            });
            if newer {
                return;
            }
            states.insert(
                key.clone(),
                Observed {
                    machine_id: connection.machine_id().to_owned(),
                    connection: connection.clone(),
                    event: event.clone(),
                    observed_at: Instant::now(),
                },
            );
        }
        if event.instance.instance_id.is_empty() {
            self.cancel_viewer(&key.0, &key.1);
        } else {
            self.end_mismatched_viewer(&key, event);
        }
        self.settle_navigations_from_state(connection, event, true);
    }

    /// The durable placements on the connection's Machine are known: each
    /// stays unresolved until that connection answers for it.
    pub fn placements_loaded(
        &self,
        connection: &ConnectionToken,
        accounts: impl IntoIterator<Item = ProviderAccountId>,
    ) {
        let machine_id = connection.machine_id();
        let mut inventories = self.inventories.lock();
        let Some(inventory) = inventories
            .get_mut(machine_id)
            .filter(|inventory| inventory.connection.same(connection))
        else {
            return;
        };
        let pending: HashSet<_> = accounts
            .into_iter()
            .filter(|account| !inventory.answered.contains(account))
            .collect();
        if pending.is_empty() {
            inventories.remove(machine_id);
        } else {
            inventory.pending = Some(pending);
            inventory.answered.clear();
        }
    }

    fn answered(&self, connection: &ConnectionToken, account: &ProviderAccountId) {
        let machine_id = connection.machine_id();
        let mut inventories = self.inventories.lock();
        let Some(inventory) = inventories
            .get_mut(machine_id)
            .filter(|inventory| inventory.connection.same(connection))
        else {
            return;
        };
        match inventory.pending.as_mut() {
            None => {
                inventory.answered.insert(account.clone());
            }
            Some(pending) => {
                pending.remove(account);
                if pending.is_empty() {
                    inventories.remove(machine_id);
                }
            }
        }
    }

    /// The placements on this Machine whose browser its current connection
    /// has not answered for (empty while the placements are still loading),
    /// or `None` once every one was answered. A browser that survived a
    /// reconnect is invisible to `running_on`/`stopping_on` meanwhile.
    pub fn unresolved_on(&self, machine_id: &str) -> Option<Vec<ProviderAccountId>> {
        let inventories = self.inventories.lock();
        let inventory = inventories.get(machine_id)?;
        let mut accounts: Vec<_> = inventory.pending.iter().flatten().cloned().collect();
        accounts.sort();
        Some(accounts)
    }

    /// The latest observation of the account's browser on one Machine.
    pub fn state_on(&self, machine_id: &str, account_id: &str) -> Option<Observed> {
        self.states
            .read()
            .get(&key(machine_id, account_id))
            .cloned()
    }

    /// The latest observation of the account's browser on every Machine
    /// that reported one, ordered by Machine.
    pub fn states_of(&self, account_id: &str) -> Vec<Observed> {
        let mut states: Vec<Observed> = self
            .states
            .read()
            .iter()
            .filter(|((_, account), _)| account == account_id)
            .map(|(_, observed)| observed.clone())
            .collect();
        states.sort_by(|left, right| left.machine_id.cmp(&right.machine_id));
        states
    }

    /// Running instances on one Machine, for memory reclaim.
    pub fn running_on(&self, machine_id: &str) -> Vec<Observed> {
        let mut running: Vec<Observed> = self
            .states
            .read()
            .iter()
            .filter(|((machine, _), observed)| machine == machine_id && observed.running())
            .map(|(_, observed)| observed.clone())
            .collect();
        running.sort_by(|left, right| {
            left.event
                .instance
                .account_id
                .as_str()
                .cmp(right.event.instance.account_id.as_str())
        });
        running
    }

    /// Stopping instances on one Machine, including those naming no
    /// instance (a unit of unknown owner whose exit is unconfirmed, or the
    /// startup inventory still pending): each may still hold memory.
    pub fn stopping_on(&self, machine_id: &str) -> Vec<Observed> {
        let mut stopping: Vec<Observed> = self
            .states
            .read()
            .iter()
            .filter(|((machine, _), observed)| {
                machine == machine_id
                    && observed.event.state == State::Stopping
                    && self.is_current(&observed.connection)
            })
            .map(|(_, observed)| observed.clone())
            .collect();
        stopping.sort_by(|left, right| {
            left.event
                .instance
                .account_id
                .as_str()
                .cmp(right.event.instance.account_id.as_str())
        });
        stopping
    }

    /// Register a one-time stream token for a viewer that is about to ask
    /// the Machine to attach.
    pub fn register_pairing(&self, token: String, pairing: Pairing) {
        let mut pairings = self.pairings.lock();
        let ttl = Duration::from_millis(u64::try_from(STREAM_TOKEN_TTL_MS).unwrap_or(30_000));
        pairings.retain(|_, pairing| pairing.created.elapsed() < ttl);
        pairings.insert(token, pairing);
    }

    /// Consume a stream token; a token older than its TTL is gone.
    pub fn take_pairing(&self, token: &str) -> Option<Pairing> {
        let mut pairings = self.pairings.lock();
        let pairing = pairings.remove(token)?;
        let ttl = Duration::from_millis(u64::try_from(STREAM_TOKEN_TTL_MS).unwrap_or(30_000));
        (pairing.created.elapsed() < ttl).then_some(pairing)
    }

    pub fn forget_pairing(&self, token: &str) {
        self.pairings.lock().remove(token);
    }

    /// Serializes attachment admission and viewer registration of one
    /// account, so two viewers can never both see it free.
    pub fn attach_lock(&self, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock_for(&self.attach_locks, account_id)
    }

    /// Serializes an account's open dispatch with its revocation dispatch,
    /// so a revocation is never sent ahead of an open it must follow.
    pub fn lifecycle_lock(&self, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock_for(&self.lifecycle_locks, account_id)
    }

    /// Serializes an owner's share membership changes with share resets,
    /// opens, and identity-bound instance commands (`expected_browser`).
    pub fn share_lock(&self, owner: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock_for(&self.share_locks, owner)
    }

    /// Handle notified whenever a tracked navigation fence clears.
    pub fn navigation_settled_notify(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.navigation_settled)
    }

    /// Record an identity-bound navigation dispatched on `connection` so share
    /// membership changes remain fenced across Controller wait timeouts and
    /// transport drops until the Machine settles it or confirms instance exit.
    pub fn begin_navigation(
        &self,
        request_id: String,
        owner: &str,
        browser_key: ProviderAccountId,
        connection: &ConnectionToken,
        instance: InstanceRef,
        observed_at_ms: i64,
    ) {
        let machine_id = connection.machine_id().to_owned();
        self.settled_navigation_ids.lock().remove(&request_id);
        self.pending_navigations.lock().insert(
            request_id.clone(),
            PendingNavigation {
                request_id,
                owner: owner.to_owned(),
                browser_key,
                machine_id,
                connection: Some(connection.clone()),
                instance,
                observed_at_ms,
            },
        );
    }

    /// Seed a durable pre-restart pending navigation into memory so live
    /// Machine observations can settle it promptly once its Machine connects.
    pub fn seed_durable_navigation(&self, entry: PendingNavigation) {
        if self
            .settled_navigation_ids
            .lock()
            .contains(&entry.request_id)
        {
            return;
        }
        self.pending_navigations
            .lock()
            .entry(entry.request_id.clone())
            .or_insert(entry);
    }

    /// Clear a tracked navigation fence once the command definitely failed or
    /// finished on the Machine.
    pub fn settle_navigation(&self, request_id: &str) {
        if self.pending_navigations.lock().remove(request_id).is_some() {
            self.settled_navigation_ids
                .lock()
                .insert(request_id.to_owned());
            self.navigation_settled.notify_one();
        }
    }

    /// Clear a tracked navigation fence when the exact Machine connection
    /// answers that `request_id` (for example a delayed `CommandResult` refusal).
    /// Returns `true` when a matching browser navigation was settled.
    pub fn settle_navigation_on(&self, connection: &ConnectionToken, request_id: &str) -> bool {
        let mut pending = self.pending_navigations.lock();
        if pending.get(request_id).is_some_and(|entry| {
            entry
                .connection
                .as_ref()
                .is_some_and(|conn| conn.same(connection))
        }) {
            pending.remove(request_id);
            drop(pending);
            self.settled_navigation_ids
                .lock()
                .insert(request_id.to_owned());
            self.navigation_settled.notify_one();
            return true;
        }
        false
    }

    /// Take request ids settled in memory that must be deleted from durable
    /// storage (`browser_pending_navigations`).
    pub fn take_settled_navigation_ids(&self) -> Vec<String> {
        self.settled_navigation_ids.lock().drain().collect()
    }

    /// Re-queue settled request ids if their durable store deletion failed.
    pub fn requeue_settled_navigation_ids(&self, ids: Vec<String>) {
        self.settled_navigation_ids.lock().extend(ids);
    }

    /// Whether `request_id` is still tracked as an unsettled navigation.
    pub fn is_navigation_pending(&self, request_id: &str) -> bool {
        self.pending_navigations.lock().contains_key(request_id)
    }

    /// Whether `request_id` was dispatched in this Controller process on a
    /// connection that is still its Machine's current connection (so the live
    /// connection can still deliver its reply without reconnect reconciliation).
    pub fn is_navigation_live_on_current(&self, request_id: &str) -> bool {
        let connections = self.connections.read();
        self.pending_navigations
            .lock()
            .get(request_id)
            .is_some_and(|entry| {
                entry.connection.as_ref().is_some_and(|conn| {
                    connections
                        .get(&entry.machine_id)
                        .is_some_and(|current| current.same(conn))
                })
            })
    }

    /// Whether `browser_key` has any unsettled identity-bound navigation in
    /// flight in memory (callers also check durable storage).
    pub fn has_pending_navigation(&self, owner: &str, browser_key: &ProviderAccountId) -> bool {
        self.pending_navigations
            .lock()
            .values()
            .any(|pending| pending.owner == owner && pending.browser_key == *browser_key)
    }

    /// Unsettled in-memory navigations whose dispatching connection has been
    /// replaced by a newer live connection on the same Machine (or that were
    /// seeded after restart with `connection: None`).
    pub fn orphaned_navigations(&self, machine_id: Option<&str>) -> Vec<PendingNavigation> {
        let connections = self.connections.read();
        self.pending_navigations
            .lock()
            .values()
            .filter(|entry| {
                machine_id.is_none_or(|id| entry.machine_id == id)
                    && connections.get(&entry.machine_id).is_some_and(|current| {
                        entry
                            .connection
                            .as_ref()
                            .is_none_or(|conn| !current.same(conn))
                    })
            })
            .cloned()
            .collect()
    }

    fn settle_navigations_from_state(
        &self,
        connection: &ConnectionToken,
        state: &StateEvent,
        authoritative: bool,
    ) {
        let machine_id = connection.machine_id();
        let mut settled_ids = Vec::new();
        let mut pending = self.pending_navigations.lock();
        // A matching `Stopping` reply (e.g. launcher navigation timeout with
        // unconfirmed unit exit) must NOT clear the fence until exit confirms.
        if let Some(request_id) = state.request_id.as_deref()
            && state.state != State::Stopping
            && pending.get(request_id).is_some_and(|entry| {
                entry
                    .connection
                    .as_ref()
                    .is_some_and(|conn| conn.same(connection))
            })
        {
            pending.remove(request_id);
            settled_ids.push(request_id.to_owned());
        }
        pending.retain(|id, entry| {
            if entry.machine_id != machine_id || entry.browser_key != state.instance.account_id {
                return true;
            }
            let same_connection = entry
                .connection
                .as_ref()
                .is_some_and(|conn| conn.same(connection));
            let keep = match state.state {
                State::Hibernated => {
                    if !state.instance.instance_id.is_empty() {
                        // Confirmed terminal exit of the exact tracked instance (or a newer one).
                        !(state.instance.instance_id == entry.instance.instance_id
                            && state.instance.incarnation >= entry.instance.incarnation)
                    } else if authoritative
                        && (!same_connection || state.observed_at_ms >= entry.observed_at_ms)
                    {
                        // Authoritative Machine answer (`Query`, confirmed `Reset`/`Revoke`)
                        // proving no running or unconfirmed unit exists for this key.
                        false
                    } else {
                        // Non-authoritative empty `Hibernated` (such as a refused reset for a
                        // stale profile revision) never proves the running browser exited.
                        true
                    }
                }
                State::Stopping => {
                    // `Stopping` for the tracked incarnation does not confirm unit exit.
                    // Only a strictly newer incarnation proves the earlier one already exited.
                    !(!state.instance.instance_id.is_empty()
                        && state.instance.instance_id == entry.instance.instance_id
                        && state.instance.incarnation > entry.instance.incarnation)
                }
                State::Starting | State::Detached | State::Attached => {
                    // A different running instance or strictly newer incarnation proves the
                    // tracked instance exited and was replaced.
                    !(!state.instance.instance_id.is_empty()
                        && (state.instance.instance_id != entry.instance.instance_id
                            || state.instance.incarnation > entry.instance.incarnation))
                }
            };
            if !keep {
                settled_ids.push(id.clone());
            }
            keep
        });
        drop(pending);
        if !settled_ids.is_empty() {
            self.settled_navigation_ids.lock().extend(settled_ids);
            self.navigation_settled.notify_one();
        }
    }

    /// Whether the Controller holds a viewer for the account's instance on
    /// this Machine, attached or still attaching.
    pub fn has_viewer(&self, machine_id: &str, account_id: &str) -> bool {
        self.viewers
            .lock()
            .contains_key(&key(machine_id, account_id))
    }

    /// Whether this attachment is still the registered viewer of the
    /// account's instance on this Machine.
    pub fn is_viewer(&self, machine_id: &str, account_id: &str, attachment_id: &str) -> bool {
        self.viewers
            .lock()
            .get(&key(machine_id, account_id))
            .is_some_and(|viewer| viewer.attachment_id == attachment_id)
    }

    /// The server-recorded browser identity captured when `attachment_id`
    /// attached for `(owner, member)` on `(machine_id, account_id)`, if that
    /// viewer is still registered.
    pub fn viewer_identity(
        &self,
        machine_id: &str,
        account_id: &str,
        owner: &str,
        member: &str,
        attachment_id: &str,
    ) -> Option<String> {
        self.viewers
            .lock()
            .get(&key(machine_id, account_id))
            .filter(|viewer| {
                viewer.owner == owner
                    && viewer.member == member
                    && viewer.attachment_id == attachment_id
            })
            .map(|viewer| viewer.identity.clone())
    }

    /// Register the viewer of the account's instance on the connection's
    /// Machine before its attachment is requested, ending the previous one.
    /// Account disablement and revocation cancel it from this moment on.
    pub fn register_viewer(
        &self,
        (account_id, owner, member): (&str, &str, &str),
        attachment_id: &str,
        incarnation: u64,
        connection: &ConnectionToken,
    ) -> watch::Receiver<bool> {
        self.register_viewer_with_identity(
            (account_id, owner, member),
            attachment_id,
            incarnation,
            connection,
            "own".to_owned(),
        )
    }

    /// Like [`Self::register_viewer`], also binding the confirmed browser
    /// identity (`own` or `share:<id>:<rev>`) observed when the viewer attached.
    pub fn register_viewer_with_identity(
        &self,
        (account_id, owner, member): (&str, &str, &str),
        attachment_id: &str,
        incarnation: u64,
        connection: &ConnectionToken,
        identity: String,
    ) -> watch::Receiver<bool> {
        let (cancel, cancelled) = watch::channel(false);
        let previous = self.viewers.lock().insert(
            key(connection.machine_id(), account_id),
            Viewer {
                owner: owner.to_owned(),
                member: member.to_owned(),
                identity,
                attachment_id: attachment_id.to_owned(),
                incarnation,
                connection: connection.clone(),
                pending: true,
                cancel,
            },
        );
        if let Some(previous) = previous {
            let _ = previous.cancel.send(true);
        }
        cancelled
    }

    /// The Machine confirmed the attachment; from now on any other
    /// attachment or state ends this viewer.
    pub fn confirm_viewer(&self, machine_id: &str, account_id: &str, attachment_id: &str) {
        if let Some(viewer) = self.viewers.lock().get_mut(&key(machine_id, account_id))
            && viewer.attachment_id == attachment_id
        {
            viewer.pending = false;
        }
    }

    /// Forget a viewer when its own stream ended; a replacement stays.
    pub fn unregister_viewer(&self, machine_id: &str, account_id: &str, attachment_id: &str) {
        let key = key(machine_id, account_id);
        let mut viewers = self.viewers.lock();
        if viewers
            .get(&key)
            .is_some_and(|viewer| viewer.attachment_id == attachment_id)
        {
            viewers.remove(&key);
        }
    }

    /// End the viewer of the account's instance on one Machine.
    pub fn cancel_viewer(&self, machine_id: &str, account_id: &str) {
        if let Some(viewer) = self.viewers.lock().remove(&key(machine_id, account_id)) {
            let _ = viewer.cancel.send(true);
        }
    }

    /// End every viewer of the account, on any Machine (disablement,
    /// revocation).
    pub fn cancel_account_viewers(&self, account_id: &str) {
        self.cancel_viewers_where(|(_, account), _| account == account_id);
    }

    /// End every viewer `owner`'s card `member` attached, to its own
    /// browser or to a share's (the member was disabled, revoked or left the
    /// share). Other members' viewers of the same share, and every other
    /// owner's viewers, stay.
    pub fn cancel_member_viewers(&self, owner: &str, member: &str) {
        self.cancel_viewers_where(|_, viewer| viewer.owner == owner && viewer.member == member);
    }

    fn cancel_viewers_where(&self, ends: impl Fn(&Key, &Viewer) -> bool) {
        self.viewers.lock().retain(|key, viewer| {
            if ends(key, viewer) {
                let _ = viewer.cancel.send(true);
                false
            } else {
                true
            }
        });
    }

    /// Pairing deadline for a viewer waiting on the Machine stream.
    pub fn pairing_timeout() -> Duration {
        Duration::from_millis(u64::try_from(PAIRING_TIMEOUT_MS).unwrap_or(15_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_control::MachineControl;
    use crate::provider_account::ProviderAccountId;

    const ACCOUNT: &str = "11111111-2222-4333-8444-555555555555";

    fn state_event(incarnation: u64, state: State, attachment: Option<&str>) -> StateEvent {
        StateEvent {
            request_id: None,
            instance: InstanceRef {
                account_id: ProviderAccountId::try_from(ACCOUNT.to_owned()).unwrap(),
                instance_id: if incarnation == 0 {
                    String::new()
                } else {
                    "b-1".into()
                },
                incarnation,
            },
            state,
            reason: None,
            geometry: None,
            scale: None,
            deadline_ms: None,
            attachment_id: attachment.map(ToOwned::to_owned),
            snapshot_generation: None,
            snapshot_at_ms: None,
            profile_revision: Some(1),
            unsaved_state_lost: false,
            observed_at_ms: i64::try_from(incarnation).unwrap(),
        }
    }

    fn event(incarnation: u64, state: State, attachment: Option<&str>) -> Event {
        Event::State(state_event(incarnation, state, attachment))
    }

    fn connect(
        control: &MachineControl,
        coordinator: &Coordinator,
        epoch: &str,
    ) -> ConnectionToken {
        connect_machine(control, coordinator, "ovh", epoch)
    }

    fn connect_machine(
        control: &MachineControl,
        coordinator: &Coordinator,
        machine: &str,
        epoch: &str,
    ) -> ConnectionToken {
        let connection = control.install(
            machine.into(),
            epoch.into(),
            false,
            crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
            tokio::sync::mpsc::unbounded_channel().0,
        );
        coordinator.connected(&connection, None);
        connection
    }

    #[test]
    fn viewers_end_when_their_attachment_incarnation_or_state_changes() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect(&control, &coordinator, "one");
        let mut cancelled =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-1", 7, &connection);
        coordinator.confirm_viewer("ovh", ACCOUNT, "att-1");
        coordinator.observe(&connection, &event(7, State::Attached, Some("att-1")));
        assert!(!*cancelled.borrow_and_update());
        // A stale observation for an older incarnation is ignored.
        coordinator.observe(&connection, &event(6, State::Hibernated, None));
        assert!(!*cancelled.borrow_and_update());
        assert!(coordinator.state_on("ovh", ACCOUNT).unwrap().running());
        // A takeover by another attachment ends this viewer.
        coordinator.observe(&connection, &event(7, State::Attached, Some("att-2")));
        assert!(*cancelled.borrow_and_update());
        let mut cancelled =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-3", 7, &connection);
        coordinator.observe(&connection, &event(7, State::Hibernated, None));
        assert!(*cancelled.borrow_and_update());
        assert!(!coordinator.state_on("ovh", ACCOUNT).unwrap().running());
        assert!(coordinator.running_on("ovh").is_empty());
    }

    #[test]
    fn a_command_s_outcome_is_never_cached_as_a_running_browser_s_state() {
        use crate::machine_protocol::machine_browser::Reason;
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect(&control, &coordinator, "one");
        let answer = |state, reason| {
            let mut event = state_event(7, state, None);
            event.request_id = Some("browser-detach-1".into());
            event.reason = Some(reason);
            Event::State(event)
        };
        // The viewer's stream ended first, so the Controller's detach of that
        // attachment is a no-op the Machine answers `superseded`; a refused
        // resize or attach likewise answers with the running state.
        for (state, reason) in [
            (State::Detached, Reason::Superseded),
            (State::Attached, Reason::Failed),
            (State::Starting, Reason::Busy),
        ] {
            coordinator.observe(&connection, &answer(state, reason));
            let cached = coordinator.state_on("ovh", ACCOUNT).unwrap().event;
            assert_eq!(cached.state, state);
            assert_eq!(cached.reason, None, "{state:?} kept {reason:?}");
            assert_eq!(cached.request_id.as_deref(), Some("browser-detach-1"));
        }
        // Why an instance is stopping or stopped is its own state.
        coordinator.observe(&connection, &answer(State::Stopping, Reason::Grace));
        assert_eq!(
            coordinator.state_on("ovh", ACCOUNT).unwrap().event.reason,
            Some(Reason::Grace)
        );
        coordinator.observe(&connection, &answer(State::Hibernated, Reason::Deadline));
        assert_eq!(
            coordinator.state_on("ovh", ACCOUNT).unwrap().event.reason,
            Some(Reason::Deadline)
        );
    }

    #[test]
    fn a_pending_viewer_survives_until_its_attachment_is_confirmed() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect(&control, &coordinator, "one");
        coordinator.observe(&connection, &event(7, State::Detached, None));
        let mut cancelled =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-1", 7, &connection);
        assert!(coordinator.has_viewer("ovh", ACCOUNT));
        // The previous attachment's detach arrives while this one attaches.
        coordinator.observe(&connection, &event(7, State::Detached, None));
        assert!(!*cancelled.borrow_and_update());
        coordinator.confirm_viewer("ovh", ACCOUNT, "att-1");
        coordinator.observe(&connection, &event(7, State::Detached, None));
        assert!(*cancelled.borrow_and_update());
        assert!(!coordinator.has_viewer("ovh", ACCOUNT));
    }

    #[test]
    fn a_second_registration_replaces_and_ends_the_first_viewer() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect(&control, &coordinator, "one");
        let mut first =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-1", 7, &connection);
        let _second =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-2", 7, &connection);
        assert!(*first.borrow_and_update());
        assert!(coordinator.is_viewer("ovh", ACCOUNT, "att-2"));
        coordinator.unregister_viewer("ovh", ACCOUNT, "att-1");
        assert!(coordinator.is_viewer("ovh", ACCOUNT, "att-2"));
    }

    #[test]
    fn observations_are_scoped_to_the_reporting_connection() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let first = connect(&control, &coordinator, "one");
        coordinator.observe(&first, &event(7, State::Detached, None));
        let mut cancelled =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-1", 7, &first);
        // The Machine reconnects (it may have rebooted): nothing survives.
        let second = connect(&control, &coordinator, "two");
        assert!(coordinator.state_on("ovh", ACCOUNT).is_none());
        assert!(*cancelled.borrow_and_update());
        // A late event from the replaced connection is not believed.
        coordinator.observe(&first, &event(8, State::Detached, None));
        assert!(coordinator.state_on("ovh", ACCOUNT).is_none());
        coordinator.observe(&second, &event(1, State::Detached, None));
        assert_eq!(
            coordinator
                .state_on("ovh", ACCOUNT)
                .unwrap()
                .event
                .instance
                .incarnation,
            1
        );
        // The old connection's teardown leaves the new one alone.
        coordinator.disconnected(&first);
        assert!(coordinator.state_on("ovh", ACCOUNT).is_some());
        coordinator.disconnected(&second);
        assert!(coordinator.state_on("ovh", ACCOUNT).is_none());
    }

    #[test]
    fn only_an_authoritative_answer_clears_a_live_instance() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect(&control, &coordinator, "one");
        coordinator.observe(&connection, &event(7, State::Detached, None));
        let mut cancelled =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-1", 7, &connection);
        // A refused reset names no instance and must not hide the browser.
        coordinator.observe(&connection, &event(0, State::Hibernated, None));
        assert!(coordinator.state_on("ovh", ACCOUNT).unwrap().running());
        assert!(!*cancelled.borrow_and_update());
        // A query answering that nothing runs is believed, and newer.
        let mut absent = state_event(0, State::Hibernated, None);
        absent.observed_at_ms = 100;
        coordinator.record_authoritative(&connection, &absent);
        assert!(!coordinator.state_on("ovh", ACCOUNT).unwrap().live());
        assert!(*cancelled.borrow_and_update());
        // An older answer does not override a newer observation.
        let mut running = state_event(9, State::Detached, None);
        running.observed_at_ms = 200;
        coordinator.observe(&connection, &Event::State(running));
        coordinator.record_authoritative(&connection, &absent);
        assert!(coordinator.state_on("ovh", ACCOUNT).unwrap().running());
    }

    #[test]
    fn each_machine_keeps_its_own_observation_and_viewer() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let first = connect_machine(&control, &coordinator, "alpha", "one");
        let second = connect_machine(&control, &coordinator, "beta", "one");
        coordinator.observe(&first, &event(7, State::Stopping, None));
        let mut cancelled =
            coordinator.register_viewer((ACCOUNT, "owner", ACCOUNT), "att-1", 7, &first);
        // Another Machine's newer, running instance of the same account
        // neither replaces nor ends what the first Machine reported.
        coordinator.observe(&second, &event(9, State::Detached, None));
        let mut absent = state_event(0, State::Hibernated, None);
        absent.observed_at_ms = 100;
        coordinator.record_authoritative(&second, &absent);
        let alpha = coordinator.state_on("alpha", ACCOUNT).unwrap();
        assert_eq!(alpha.event.state, State::Stopping);
        assert_eq!(alpha.event.instance.incarnation, 7);
        assert!(alpha.connection.same(&first));
        assert!(!*cancelled.borrow_and_update());
        assert!(coordinator.has_viewer("alpha", ACCOUNT));
        assert!(!coordinator.has_viewer("beta", ACCOUNT));
        assert_eq!(coordinator.states_of(ACCOUNT).len(), 2);
        assert!(coordinator.running_on("alpha").is_empty());
        // Disablement ends the account's viewers on every Machine.
        coordinator.cancel_account_viewers(ACCOUNT);
        assert!(*cancelled.borrow_and_update());
        // One Machine reconnecting forgets only its own observation.
        connect_machine(&control, &coordinator, "beta", "two");
        assert!(coordinator.state_on("beta", ACCOUNT).is_none());
        assert!(coordinator.state_on("alpha", ACCOUNT).is_some());
    }

    #[test]
    fn a_terminal_hibernation_replaces_a_stopping_instance() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect(&control, &coordinator, "one");
        coordinator.observe(&connection, &event(7, State::Stopping, None));
        // An older incarnation's exit cannot hide the stopping instance.
        coordinator.observe(&connection, &event(6, State::Hibernated, None));
        assert_eq!(
            coordinator.state_on("ovh", ACCOUNT).unwrap().event.state,
            State::Stopping
        );
        // The confirmed exit of this incarnation, after a termination retry.
        coordinator.observe(&connection, &event(7, State::Hibernated, None));
        let observed = coordinator.state_on("ovh", ACCOUNT).unwrap();
        assert_eq!(observed.event.state, State::Hibernated);
        assert_eq!(observed.event.instance.incarnation, 7);
        assert!(!observed.live());
    }

    #[test]
    fn pairings_are_single_use() {
        let coordinator = Coordinator::default();
        let (deliver, _receive) = oneshot::channel();
        let token = "token-1234567890abcdef".to_owned();
        coordinator.register_pairing(
            token.clone(),
            Pairing {
                created: Instant::now(),
                connection: crate::machine_control::MachineControl::default().install(
                    "ovh".into(),
                    "challenge".into(),
                    false,
                    crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
                    tokio::sync::mpsc::unbounded_channel().0,
                ),
                deliver,
            },
        );
        assert!(coordinator.take_pairing(&token).is_some());
        assert!(coordinator.take_pairing(&token).is_none());
    }

    #[test]
    fn a_reconnected_machine_stays_unresolved_until_each_placement_is_answered() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let other: ProviderAccountId = "22222222-2222-4333-8444-555555555555"
            .to_owned()
            .try_into()
            .unwrap();
        let account: ProviderAccountId = ACCOUNT.to_owned().try_into().unwrap();
        let first = connect(&control, &coordinator, "one");
        // Unknown until the placements load.
        assert_eq!(coordinator.unresolved_on("ovh"), Some(Vec::new()));
        // An answer that arrives before the placements load still counts.
        coordinator.record_authoritative(&first, &state_event(0, State::Hibernated, None));
        coordinator.placements_loaded(&first, [account.clone(), other.clone()]);
        assert_eq!(coordinator.unresolved_on("ovh"), Some(vec![other.clone()]));
        // A later connection starts over; the earlier one resolves nothing.
        let second = connect(&control, &coordinator, "two");
        coordinator.placements_loaded(&first, []);
        assert_eq!(coordinator.unresolved_on("ovh"), Some(Vec::new()));
        coordinator.placements_loaded(&second, [account.clone(), other.clone()]);
        let mut answer = state_event(0, State::Hibernated, None);
        coordinator.record_authoritative(&first, &answer);
        coordinator.observe(&second, &event(3, State::Detached, None));
        // Only authoritative answers on the current connection resolve.
        assert_eq!(coordinator.unresolved_on("ovh").map(|v| v.len()), Some(2));
        coordinator.record_authoritative(&second, &answer);
        answer.instance.account_id = other;
        coordinator.record_authoritative(&second, &answer);
        assert_eq!(coordinator.unresolved_on("ovh"), None);
        // Nothing placed there: resolved at once; a disconnect forgets it.
        let third = connect(&control, &coordinator, "three");
        coordinator.placements_loaded(&third, []);
        assert_eq!(coordinator.unresolved_on("ovh"), None);
        let fourth = connect(&control, &coordinator, "four");
        assert!(coordinator.unresolved_on("ovh").is_some());
        coordinator.disconnected(&fourth);
        assert_eq!(coordinator.unresolved_on("ovh"), None);
    }

    #[test]
    fn a_member_leaving_ends_only_its_own_viewer_of_a_share() {
        const SHARE: &str = "99999999-8888-8777-a666-555555555555";
        const LYNX: &str = "aaaaaaaa-bbbb-8ccc-8ddd-eeeeeeeeeeee";
        const PIKA: &str = "0f0f0f0f-1e1e-8d2d-ac3c-4b4b4b4b4b4b";
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect_machine(&control, &coordinator, "ovh", "one");
        // Lynx's card attached to the share's single instance.
        let mut lynx = coordinator.register_viewer((SHARE, "owner", LYNX), "att-1", 7, &connection);
        // Another member (Pika) leaving or being disabled leaves it alone,
        // and so does another owner's identity of the same id.
        coordinator.cancel_member_viewers("owner", PIKA);
        coordinator.cancel_member_viewers("other", LYNX);
        assert!(!*lynx.borrow_and_update());
        assert!(coordinator.has_viewer("ovh", SHARE));
        // Pika takes the display over: Lynx's viewer is replaced.
        let mut pika = coordinator.register_viewer((SHARE, "owner", PIKA), "att-2", 7, &connection);
        assert!(*lynx.borrow_and_update());
        // Pika leaving ends exactly Pika's viewer of the share.
        coordinator.cancel_member_viewers("owner", PIKA);
        assert!(*pika.borrow_and_update());
        assert!(!coordinator.has_viewer("ovh", SHARE));
        // A member's viewer of its own key ends as before.
        let mut own = coordinator.register_viewer((LYNX, "owner", LYNX), "att-3", 7, &connection);
        coordinator.cancel_member_viewers("owner", LYNX);
        assert!(*own.borrow_and_update());
    }
}

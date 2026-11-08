//! In-memory Controller view of Machine browsers: per-Machine capability,
//! the latest state of each account's browser on each Machine, pending
//! stream pairings and live viewer attachments. Everything here is re-learned from Machine
//! hellos and state events and is scoped to the connection that reported
//! it: a reconnected (possibly rebooted) Machine starts from nothing. The
//! durable placement lives in the store.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::WebSocket;
use tokio::sync::{oneshot, watch};

use crate::machine_control::ConnectionToken;
use crate::machine_protocol::machine_browser::{
    Capability, Event, InstanceRef, PAIRING_TIMEOUT_MS, STREAM_TOKEN_TTL_MS, State, StateEvent,
};

/// The latest observation of one account's browser, on the connection that
/// reported it.
#[derive(Clone, Debug)]
pub(in crate::server) struct Observed {
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
    pub(in crate::server) fn running(&self) -> bool {
        matches!(
            self.event.state,
            State::Starting | State::Detached | State::Attached
        )
    }

    /// A process may exist: running or still stopping.
    pub(in crate::server) fn live(&self) -> bool {
        self.running() || self.event.state == State::Stopping
    }

    pub(in crate::server) fn instance(&self) -> InstanceRef {
        self.event.instance.clone()
    }
}

/// A viewer waiting for the Machine's half of its display stream.
pub(in crate::server) struct Pairing {
    pub created: Instant,
    pub connection: ConnectionToken,
    pub deliver: oneshot::Sender<WebSocket>,
}

struct Viewer {
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
pub(in crate::server) type Key = (String, String);

fn key(machine_id: &str, account_id: &str) -> Key {
    (machine_id.to_owned(), account_id.to_owned())
}

type Locks = parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

#[derive(Default)]
pub(in crate::server) struct Coordinator {
    /// The current connection of each Machine, as of its hello.
    connections: parking_lot::RwLock<HashMap<String, ConnectionToken>>,
    capabilities: parking_lot::RwLock<HashMap<String, Capability>>,
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
}

fn lock_for(locks: &Locks, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    Arc::clone(locks.lock().entry(account_id.to_owned()).or_default())
}

impl Coordinator {
    /// A Machine connection was installed: everything learned on any
    /// earlier connection of that Machine is gone, and its viewers end.
    pub(in crate::server) fn connected(
        &self,
        connection: &ConnectionToken,
        capability: Option<Capability>,
    ) {
        let machine_id = connection.machine_id().to_owned();
        self.connections
            .write()
            .insert(machine_id.clone(), connection.clone());
        self.observe_capability(&machine_id, capability);
        self.states
            .write()
            .retain(|(machine, _), _| *machine != machine_id);
        self.cancel_viewers_where(|(machine, _), viewer| {
            *machine == machine_id && !viewer.connection.same(connection)
        });
    }

    /// A Machine connection ended: drop what it reported and end its viewers.
    pub(in crate::server) fn disconnected(&self, connection: &ConnectionToken) {
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
            self.observe_capability(machine_id, None);
        }
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

    pub(in crate::server) fn observe_capability(
        &self,
        machine_id: &str,
        capability: Option<Capability>,
    ) {
        let mut capabilities = self.capabilities.write();
        match capability {
            Some(capability) => {
                capabilities.insert(machine_id.to_owned(), capability);
            }
            None => {
                capabilities.remove(machine_id);
            }
        }
    }

    pub(in crate::server) fn capability(&self, machine_id: &str) -> Option<Capability> {
        self.capabilities.read().get(machine_id).cloned()
    }

    /// Consume one Machine browser event from the current connection. It
    /// describes that Machine's instance only. A state that no longer
    /// matches a live viewer's attachment or incarnation ends that viewer.
    /// An answer naming no instance (a refused reset, a stopped browser)
    /// never hides a live instance here; only `record_authoritative` accepts
    /// it over one. A terminal `hibernated` for the cached incarnation (a
    /// confirmed exit after `stopping`) always replaces it.
    pub(in crate::server) fn observe(&self, connection: &ConnectionToken, event: &Event) {
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
                            || previous.event.instance.incarnation <= state.instance.incarnation
                            || !previous.live()
                    });
                    if replaces {
                        states.insert(
                            key.clone(),
                            Observed {
                                machine_id: machine_id.to_owned(),
                                connection: connection.clone(),
                                event: state.clone(),
                                observed_at: Instant::now(),
                            },
                        );
                    }
                }
                self.end_mismatched_viewer(&key, state);
            }
            Event::Capability(capability) => {
                self.observe_capability(machine_id, Some(capability.clone()));
            }
            Event::CapabilityLost => {
                self.observe_capability(machine_id, None);
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
    pub(in crate::server) fn record_authoritative(
        &self,
        connection: &ConnectionToken,
        event: &StateEvent,
    ) {
        if !self.is_current(connection) {
            return;
        }
        let key = key(connection.machine_id(), event.instance.account_id.as_str());
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
    }

    /// The latest observation of the account's browser on one Machine.
    pub(in crate::server) fn state_on(
        &self,
        machine_id: &str,
        account_id: &str,
    ) -> Option<Observed> {
        self.states
            .read()
            .get(&key(machine_id, account_id))
            .cloned()
    }

    /// The latest observation of the account's browser on every Machine
    /// that reported one, ordered by Machine.
    pub(in crate::server) fn states_of(&self, account_id: &str) -> Vec<Observed> {
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
    pub(in crate::server) fn running_on(&self, machine_id: &str) -> Vec<Observed> {
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

    /// Register a one-time stream token for a viewer that is about to ask
    /// the Machine to attach.
    pub(in crate::server) fn register_pairing(&self, token: String, pairing: Pairing) {
        let mut pairings = self.pairings.lock();
        let ttl = Duration::from_millis(u64::try_from(STREAM_TOKEN_TTL_MS).unwrap_or(30_000));
        pairings.retain(|_, pairing| pairing.created.elapsed() < ttl);
        pairings.insert(token, pairing);
    }

    /// Consume a stream token; a token older than its TTL is gone.
    pub(in crate::server) fn take_pairing(&self, token: &str) -> Option<Pairing> {
        let mut pairings = self.pairings.lock();
        let pairing = pairings.remove(token)?;
        let ttl = Duration::from_millis(u64::try_from(STREAM_TOKEN_TTL_MS).unwrap_or(30_000));
        (pairing.created.elapsed() < ttl).then_some(pairing)
    }

    pub(in crate::server) fn forget_pairing(&self, token: &str) {
        self.pairings.lock().remove(token);
    }

    /// Serializes attachment admission and viewer registration of one
    /// account, so two viewers can never both see it free.
    pub(in crate::server) fn attach_lock(&self, account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock_for(&self.attach_locks, account_id)
    }

    /// Serializes an account's open dispatch with its revocation dispatch,
    /// so a revocation is never sent ahead of an open it must follow.
    pub(in crate::server) fn lifecycle_lock(
        &self,
        account_id: &str,
    ) -> Arc<tokio::sync::Mutex<()>> {
        lock_for(&self.lifecycle_locks, account_id)
    }

    /// Whether the Controller holds a viewer for the account's instance on
    /// this Machine, attached or still attaching.
    pub(in crate::server) fn has_viewer(&self, machine_id: &str, account_id: &str) -> bool {
        self.viewers
            .lock()
            .contains_key(&key(machine_id, account_id))
    }

    /// Whether this attachment is still the registered viewer of the
    /// account's instance on this Machine.
    pub(in crate::server) fn is_viewer(
        &self,
        machine_id: &str,
        account_id: &str,
        attachment_id: &str,
    ) -> bool {
        self.viewers
            .lock()
            .get(&key(machine_id, account_id))
            .is_some_and(|viewer| viewer.attachment_id == attachment_id)
    }

    /// Register the viewer of the account's instance on the connection's
    /// Machine before its attachment is requested, ending the previous one.
    /// Account disablement and revocation cancel it from this moment on.
    pub(in crate::server) fn register_viewer(
        &self,
        account_id: &str,
        attachment_id: &str,
        incarnation: u64,
        connection: &ConnectionToken,
    ) -> watch::Receiver<bool> {
        let (cancel, cancelled) = watch::channel(false);
        let previous = self.viewers.lock().insert(
            key(connection.machine_id(), account_id),
            Viewer {
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
    pub(in crate::server) fn confirm_viewer(
        &self,
        machine_id: &str,
        account_id: &str,
        attachment_id: &str,
    ) {
        if let Some(viewer) = self.viewers.lock().get_mut(&key(machine_id, account_id))
            && viewer.attachment_id == attachment_id
        {
            viewer.pending = false;
        }
    }

    /// Forget a viewer when its own stream ended; a replacement stays.
    pub(in crate::server) fn unregister_viewer(
        &self,
        machine_id: &str,
        account_id: &str,
        attachment_id: &str,
    ) {
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
    pub(in crate::server) fn cancel_viewer(&self, machine_id: &str, account_id: &str) {
        if let Some(viewer) = self.viewers.lock().remove(&key(machine_id, account_id)) {
            let _ = viewer.cancel.send(true);
        }
    }

    /// End every viewer of the account, on any Machine (disablement,
    /// revocation).
    pub(in crate::server) fn cancel_account_viewers(&self, account_id: &str) {
        self.cancel_viewers_where(|(_, account), _| account == account_id);
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
    pub(in crate::server) fn pairing_timeout() -> Duration {
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
        let mut cancelled = coordinator.register_viewer(ACCOUNT, "att-1", 7, &connection);
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
        let mut cancelled = coordinator.register_viewer(ACCOUNT, "att-3", 7, &connection);
        coordinator.observe(&connection, &event(7, State::Hibernated, None));
        assert!(*cancelled.borrow_and_update());
        assert!(!coordinator.state_on("ovh", ACCOUNT).unwrap().running());
        assert!(coordinator.running_on("ovh").is_empty());
    }

    #[test]
    fn a_pending_viewer_survives_until_its_attachment_is_confirmed() {
        let control = MachineControl::default();
        let coordinator = Coordinator::default();
        let connection = connect(&control, &coordinator, "one");
        coordinator.observe(&connection, &event(7, State::Detached, None));
        let mut cancelled = coordinator.register_viewer(ACCOUNT, "att-1", 7, &connection);
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
        let mut first = coordinator.register_viewer(ACCOUNT, "att-1", 7, &connection);
        let _second = coordinator.register_viewer(ACCOUNT, "att-2", 7, &connection);
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
        let mut cancelled = coordinator.register_viewer(ACCOUNT, "att-1", 7, &first);
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
        let mut cancelled = coordinator.register_viewer(ACCOUNT, "att-1", 7, &connection);
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
        let mut cancelled = coordinator.register_viewer(ACCOUNT, "att-1", 7, &first);
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
}

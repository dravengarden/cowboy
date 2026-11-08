//! In-memory Controller view of Machine browsers: per-Machine capability,
//! the latest state of each account's browser, pending stream pairings and
//! live viewer attachments. Everything here is re-learned from Machine
//! hellos and state events; the durable placement lives in the store.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::ws::WebSocket;
use tokio::sync::{oneshot, watch};

use crate::machine_control::ConnectionToken;
use crate::machine_protocol::machine_browser::{
    Capability, Event, InstanceRef, PAIRING_TIMEOUT_MS, STREAM_TOKEN_TTL_MS, State, StateEvent,
};

/// The latest observation of one account's browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::server) struct Observed {
    pub machine_id: String,
    pub event: StateEvent,
    pub observed_at: Instant,
}

impl Observed {
    pub(in crate::server) fn running(&self) -> bool {
        matches!(
            self.event.state,
            State::Starting | State::Detached | State::Attached
        )
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
    cancel: watch::Sender<bool>,
}

#[derive(Default)]
pub(in crate::server) struct Coordinator {
    capabilities: parking_lot::RwLock<HashMap<String, Capability>>,
    states: parking_lot::RwLock<HashMap<String, Observed>>,
    pairings: parking_lot::Mutex<HashMap<String, Pairing>>,
    viewers: parking_lot::Mutex<HashMap<String, Viewer>>,
}

impl Coordinator {
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

    /// Consume one Machine browser event. A state that no longer matches a
    /// live viewer's attachment or incarnation ends that viewer.
    pub(in crate::server) fn observe(&self, machine_id: &str, event: &Event) {
        match event {
            Event::State(state) => {
                let account = state.instance.account_id.as_str().to_owned();
                let running = matches!(
                    state.state,
                    State::Starting | State::Detached | State::Attached
                );
                {
                    let mut states = self.states.write();
                    let replaces = states.get(&account).is_none_or(|previous| {
                        // A hibernated answer for an older incarnation cannot
                        // hide a newer running instance.
                        running
                            || previous.event.instance.incarnation <= state.instance.incarnation
                            || !previous.running()
                    });
                    if replaces {
                        states.insert(
                            account.clone(),
                            Observed {
                                machine_id: machine_id.to_owned(),
                                event: state.clone(),
                                observed_at: Instant::now(),
                            },
                        );
                    }
                }
                let mut viewers = self.viewers.lock();
                if let Some(viewer) = viewers.get(&account) {
                    let replaced = state.instance.incarnation != viewer.incarnation
                        || !matches!(state.state, State::Attached)
                        || state.attachment_id.as_deref() != Some(viewer.attachment_id.as_str());
                    if replaced && state.instance.incarnation >= viewer.incarnation {
                        let _ = viewer.cancel.send(true);
                        viewers.remove(&account);
                    }
                }
            }
            Event::Capability(capability) => {
                self.observe_capability(machine_id, Some(capability.clone()));
            }
            Event::CapabilityLost => {
                self.observe_capability(machine_id, None);
                let accounts: Vec<String> = self
                    .states
                    .read()
                    .iter()
                    .filter(|(_, observed)| observed.machine_id == machine_id)
                    .map(|(account, _)| account.clone())
                    .collect();
                for account in accounts {
                    self.cancel_viewer(&account);
                }
            }
        }
    }

    pub(in crate::server) fn state(&self, account_id: &str) -> Option<Observed> {
        self.states.read().get(account_id).cloned()
    }

    /// Running instances on one Machine, for memory reclaim.
    pub(in crate::server) fn running_on(&self, machine_id: &str) -> Vec<Observed> {
        let mut running: Vec<Observed> = self
            .states
            .read()
            .values()
            .filter(|observed| observed.machine_id == machine_id && observed.running())
            .cloned()
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

    /// Register the live viewer of an account, ending the previous one.
    pub(in crate::server) fn register_viewer(
        &self,
        account_id: &str,
        attachment_id: &str,
        incarnation: u64,
    ) -> watch::Receiver<bool> {
        let (cancel, cancelled) = watch::channel(false);
        let previous = self.viewers.lock().insert(
            account_id.to_owned(),
            Viewer {
                attachment_id: attachment_id.to_owned(),
                incarnation,
                cancel,
            },
        );
        if let Some(previous) = previous {
            let _ = previous.cancel.send(true);
        }
        cancelled
    }

    /// Forget a viewer when its own stream ended; a replacement stays.
    pub(in crate::server) fn unregister_viewer(&self, account_id: &str, attachment_id: &str) {
        let mut viewers = self.viewers.lock();
        if viewers
            .get(account_id)
            .is_some_and(|viewer| viewer.attachment_id == attachment_id)
        {
            viewers.remove(account_id);
        }
    }

    pub(in crate::server) fn cancel_viewer(&self, account_id: &str) {
        if let Some(viewer) = self.viewers.lock().remove(account_id) {
            let _ = viewer.cancel.send(true);
        }
    }

    /// Pairing deadline for a viewer waiting on the Machine stream.
    pub(in crate::server) fn pairing_timeout() -> Duration {
        Duration::from_millis(u64::try_from(PAIRING_TIMEOUT_MS).unwrap_or(15_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::machine_browser::Reason;
    use crate::provider_account::ProviderAccountId;

    fn event(incarnation: u64, state: State, attachment: Option<&str>) -> Event {
        Event::State(StateEvent {
            request_id: None,
            instance: InstanceRef {
                account_id: ProviderAccountId::try_from(
                    "11111111-2222-4333-8444-555555555555".to_owned(),
                )
                .unwrap(),
                instance_id: "b-1".into(),
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
            observed_at_ms: 1,
        })
    }

    #[test]
    fn viewers_end_when_their_attachment_incarnation_or_state_changes() {
        let coordinator = Coordinator::default();
        let account = "11111111-2222-4333-8444-555555555555";
        let mut cancelled = coordinator.register_viewer(account, "att-1", 7);
        coordinator.observe("ovh", &event(7, State::Attached, Some("att-1")));
        assert!(!*cancelled.borrow_and_update());
        // A stale observation for an older incarnation is ignored.
        coordinator.observe("ovh", &event(6, State::Hibernated, None));
        assert!(!*cancelled.borrow_and_update());
        assert!(coordinator.state(account).unwrap().running());
        // A takeover by another attachment ends this viewer.
        coordinator.observe("ovh", &event(7, State::Attached, Some("att-2")));
        assert!(*cancelled.borrow_and_update());
        let mut cancelled = coordinator.register_viewer(account, "att-3", 7);
        coordinator.observe("ovh", &event(7, State::Hibernated, None));
        assert!(*cancelled.borrow_and_update());
        assert!(!coordinator.state(account).unwrap().running());
        assert!(coordinator.running_on("ovh").is_empty());
        let _ = Reason::Grace;
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
                    43,
                    tokio::sync::mpsc::unbounded_channel().0,
                ),
                deliver,
            },
        );
        assert!(coordinator.take_pairing(&token).is_some());
        assert!(coordinator.take_pairing(&token).is_none());
    }
}

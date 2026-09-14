//! Merge only simultaneous reconciliation of one immutable Service generation
//! on the same authenticated connection. This is not an auth cache, a grant,
//! a retry loop, or a durable receipt. Each AppState owns its own coordinator.
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;

use crate::machine_control::{ConnectionToken, MachineControl};
use crate::machine_protocol::{MachineCommand, ProviderAuthAction, SealedProviderAuth};

const MAX_FLIGHTS: usize = 128;
const MAX_OBSERVERS: usize = 128;
const CANCELLED: &str = "Provider auth sync owner stopped; outcome unknown";
const DISCONNECTED: &str = "Provider auth sync connection changed; outcome unknown";

type SyncResult = Result<(), String>;

// Constructed only from the Service vault's immutable, freshly sealed record.
// Randomized encryption bytes are not identity and never enter this table.
// Equal generations with conflicting public metadata are rejected, not joined.
#[derive(PartialEq, Eq)]
struct Identity {
    provider: String,
    generation: u64,
    schema: u16,
    contract: String,
    projection: String,
    action: ProviderAuthAction,
    service_key: String,
}

impl From<&SealedProviderAuth> for Identity {
    fn from(envelope: &SealedProviderAuth) -> Self {
        Self {
            provider: envelope.provider_id.clone(),
            generation: envelope.auth_generation,
            schema: envelope.envelope_schema,
            contract: envelope.auth_contract_fingerprint.clone(),
            projection: envelope.projection_schema.clone(),
            action: envelope.action.clone(),
            service_key: envelope.service_public_key.clone(),
        }
    }
}

struct Flight {
    identity: Identity,
    connection: ConnectionToken,
    result: watch::Sender<Option<SyncResult>>,
}

#[derive(Default)]
pub(super) struct Coordinator {
    flights: Mutex<Vec<Arc<Flight>>>,
}

enum Admission<'a> {
    Owner(Owner<'a>),
    Observer(watch::Receiver<Option<SyncResult>>),
}

struct Owner<'a> {
    coordinator: &'a Coordinator,
    flight: Arc<Flight>,
}

impl Owner<'_> {
    fn complete(self, result: SyncResult) -> SyncResult {
        self.flight.result.send_replace(Some(result.clone()));
        result
    }
}

impl Drop for Owner<'_> {
    fn drop(&mut self) {
        self.flight.result.send_if_modified(|result| {
            if result.is_some() {
                return false;
            }
            *result = Some(Err(CANCELLED.into()));
            true
        });
        // A late owner cannot erase a newer repair or a replacement connection.
        self.coordinator
            .flights
            .lock()
            .retain(|flight| !Arc::ptr_eq(flight, &self.flight));
    }
}

impl Coordinator {
    fn begin(
        &self,
        connection: &ConnectionToken,
        identity: Identity,
    ) -> Result<Admission<'_>, String> {
        let mut flights = self.flights.lock();
        // Never reuse a completed result, including the brief interval between
        // publication and owner Drop on another thread. Later repair is real work.
        flights.retain(|flight| flight.result.borrow().is_none());
        if let Some(flight) = flights.iter().find(|flight| {
            flight.connection.same(connection)
                && flight.identity.provider == identity.provider
                && flight.identity.generation == identity.generation
        }) {
            if flight.identity != identity {
                return Err("conflicting Provider auth synchronization identity".into());
            }
            if flight.result.receiver_count() >= MAX_OBSERVERS {
                return Err("Provider auth sync observer budget exceeded".into());
            }
            return Ok(Admission::Observer(flight.result.subscribe()));
        }
        if flights.len() >= MAX_FLIGHTS {
            return Err("Provider auth sync flight budget exceeded".into());
        }
        let (result, _) = watch::channel(None);
        let flight = Arc::new(Flight {
            identity,
            connection: connection.clone(),
            result,
        });
        flights.push(Arc::clone(&flight));
        Ok(Admission::Owner(Owner {
            coordinator: self,
            flight,
        }))
    }

    /// Only Core's Service sealing path may call this. The Machine continues to
    /// verify signature, immutable replica semantics and materialization itself.
    pub(super) async fn apply(
        &self,
        control: &MachineControl,
        connection: &ConnectionToken,
        envelope: SealedProviderAuth,
    ) -> Result<u64, String> {
        if !control.is_current(connection) {
            return Err(DISCONNECTED.into());
        }
        let generation = envelope.auth_generation;
        let result = match self.begin(connection, Identity::from(&envelope))? {
            Admission::Owner(owner) => {
                let request_id = super::machine_request_id("provider-auth");
                let result = control
                    .command_on_connection(
                        connection,
                        request_id.clone(),
                        MachineCommand::ApplyProviderAuth {
                            request_id,
                            envelope: Box::new(envelope),
                        },
                    )
                    .await
                    .map_err(|error| error.detail);
                owner.complete(if control.is_current(connection) {
                    result
                } else {
                    Err(DISCONNECTED.into())
                })
            }
            Admission::Observer(mut observer) => {
                // No observer retains an extra credential envelope while waiting.
                drop(envelope);
                loop {
                    let result = observer.borrow().clone();
                    if let Some(result) = result {
                        break result;
                    }
                    // This observes the owner's original 90-second command
                    // deadline, not a fresh observer deadline or another request.
                    if observer.changed().await.is_err() {
                        break Err(CANCELLED.into());
                    }
                }
            }
        };
        if !control.is_current(connection) {
            return Err(DISCONNECTED.into());
        }
        result?;
        Ok(generation)
    }
}

#[cfg(test)]
mod tests;

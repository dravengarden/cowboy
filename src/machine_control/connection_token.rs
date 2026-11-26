use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Created only by installing an authenticated channel. Not deserializable;
/// even accidentally reusing an epoch string creates a distinct incarnation.
#[derive(Clone, Debug)]
pub struct ConnectionToken(pub(super) Arc<ConnectionIdentity>);

static NEXT_CONNECTION_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(super) struct ConnectionIdentity {
    pub(super) machine_id: String,
    pub(super) epoch: String,
    conn_epoch: u64,
}

impl ConnectionToken {
    pub(super) fn new(machine_id: String, epoch: String) -> Self {
        Self(Arc::new(ConnectionIdentity {
            machine_id,
            epoch,
            conn_epoch: NEXT_CONNECTION_EPOCH.fetch_add(1, Ordering::Relaxed),
        }))
    }

    pub fn machine_id(&self) -> &str {
        &self.0.machine_id
    }

    /// The authenticated handshake's epoch, also recorded as the Machine's
    /// current `connection_epoch` before this token is installed.
    pub fn epoch(&self) -> &str {
        &self.0.epoch
    }

    /// Controller-local monotonic connection epoch (`>= 1`; `0` is reserved for
    /// local reads) used to invalidate diagnostic baselines across Machine
    /// reconnects (R2-M10).
    #[must_use]
    pub fn conn_epoch(&self) -> u64 {
        self.0.conn_epoch
    }

    pub fn same(&self, other: &Self) -> bool {
        self.0.epoch == other.0.epoch && Arc::ptr_eq(&self.0, &other.0)
    }

    /// A connection identity of its own, for Controller tests.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn for_test(machine_id: &str, epoch: &str) -> Self {
        Self::new(machine_id.to_owned(), epoch.to_owned())
    }
}

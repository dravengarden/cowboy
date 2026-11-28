//! Connection protocol and host-layer hibernation-epoch capability helpers
//! for [`super::MachineControl`].

use super::{ConnectionToken, MachineControl};

impl MachineControl {
    /// Short-lived connection evidence. Never saved in the operation journal or
    /// reacquired after restart to replay an old destructive command.
    /// The current connection and its negotiated protocol, if connected.
    pub fn connection_protocol(&self, machine_id: &str) -> Option<(ConnectionToken, u16)> {
        self.live
            .read()
            .connections
            .get(machine_id)
            .map(|connection| (connection.token.clone(), connection.protocol))
    }

    /// Whether the Machine's current connection negotiated at least `minimum`.
    pub fn machine_supports(&self, machine_id: &str, minimum: u16) -> bool {
        self.live
            .read()
            .connections
            .get(machine_id)
            .is_some_and(|c| c.protocol >= minimum)
    }

    /// Record that the authenticated connection `token` advertised host-layer
    /// epoch-bound `HibernateSession` enforcement on upgrade.
    pub fn mark_hibernation_epoch_supported(&self, token: &ConnectionToken) {
        let mut live = self.live.write();
        if let Some(connection) = live.connections.get_mut(&token.0.machine_id)
            && connection.token.same(token)
        {
            connection.hibernation_epoch = true;
        }
    }

    /// Whether the Machine's current connection advertised epoch-bound
    /// `HibernateSession` enforcement.
    #[must_use]
    pub fn supports_hibernation_epoch(&self, machine_id: &str) -> bool {
        self.live
            .read()
            .connections
            .get(machine_id)
            .is_some_and(|connection| connection.hibernation_epoch)
    }

    /// Whether the exact connection `token` is still current and advertised
    /// epoch-bound `HibernateSession` enforcement.
    #[must_use]
    pub fn connection_supports_hibernation_epoch(&self, token: &ConnectionToken) -> bool {
        self.live
            .read()
            .connections
            .get(&token.0.machine_id)
            .is_some_and(|connection| connection.token.same(token) && connection.hibernation_epoch)
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;

    #[test]
    fn hibernation_epoch_capability_is_connection_scoped_and_retires_on_reconnect() {
        let control = MachineControl::default();
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let first = control.install("hawk".into(), "epoch-1".into(), false, 54, tx1);
        assert!(!control.supports_hibernation_epoch("hawk"));
        assert!(!control.connection_supports_hibernation_epoch(&first));

        control.mark_hibernation_epoch_supported(&first);
        assert!(control.supports_hibernation_epoch("hawk"));
        assert!(control.connection_supports_hibernation_epoch(&first));

        // Reconnect without the header retires `first` immediately and resets capability.
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let second = control.install("hawk".into(), "epoch-2".into(), false, 54, tx2);
        assert!(!control.connection_supports_hibernation_epoch(&first));
        assert!(!control.supports_hibernation_epoch("hawk"));
        assert!(!control.connection_supports_hibernation_epoch(&second));

        // Marking the retired token is a no-op and never enables `second`.
        control.mark_hibernation_epoch_supported(&first);
        assert!(!control.supports_hibernation_epoch("hawk"));
        assert!(!control.connection_supports_hibernation_epoch(&second));
    }
}

//! Core-owned observation lease. This is neither a policy grant nor a durable
//! state lease; Machine independently checks installation and policy at effect.
use super::{ConnectionToken, LiveState, MachineControl};
use crate::machine_protocol::telemetry_binding::BindingInstallation;
use std::sync::Arc;

pub(crate) struct TelemetryInstallationLease {
    connection: ConnectionToken,
    revision: Arc<()>,
    installation: BindingInstallation,
}

impl TelemetryInstallationLease {
    pub(super) fn matches(
        &self,
        live: &LiveState,
        connection: &ConnectionToken,
        installation: &BindingInstallation,
    ) -> bool {
        self.connection.same(connection)
            && self.installation == *installation
            && live.is_current(connection)
            && live
                .plugin_inventory
                .get(&connection.0.machine_id)
                .and_then(|inventory| inventory.slot_revisions.get(&installation.plugin_id))
                .is_some_and(|revision| Arc::ptr_eq(revision, &self.revision))
            && live.telemetry_installation_matches(&connection.0.machine_id, installation)
    }
}

impl MachineControl {
    pub(crate) fn lease_telemetry_installation(
        &self,
        connection: &ConnectionToken,
        installation: &BindingInstallation,
    ) -> Option<TelemetryInstallationLease> {
        let live = self.live.read();
        if !live.is_current(connection)
            || !live.telemetry_installation_matches(&connection.0.machine_id, installation)
        {
            return None;
        }
        Some(TelemetryInstallationLease {
            connection: connection.clone(),
            revision: Arc::clone(
                &live.plugin_inventory[&connection.0.machine_id].slot_revisions
                    [&installation.plugin_id],
            ),
            installation: installation.clone(),
        })
    }
}

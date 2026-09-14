//! Socket admission captures the original lease before detached scheduling.

use crate::machine_plugins::{MachinePluginStore, PluginExecutionScope};
use crate::machine_protocol::plugin_install::{InstallLookup, InstallObservation, InstallStep};
use crate::machine_protocol::{DesiredPlugin, MachineEvent};
use std::sync::Arc;

// Reader first: protocol negotiation and the existing uninstall admission flag
// alone must not enable a new durable installation namespace. Activation needs
// accepted active, next-transaction recovery and cold Machine readers.
pub(super) const WRITER_ENABLED: bool = false;

pub(super) fn install(
    request_id: String,
    step: InstallStep,
    plugin: DesiredPlugin,
    store: Arc<MachinePluginStore>,
    execution: &PluginExecutionScope,
    admitted: bool,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
) {
    let lease = match execution.installation(&step) {
        Ok(lease) => lease,
        Err(reason) => {
            let _ = events.send(MachineEvent::PluginInstallationStep {
                request_id,
                observation: Box::new(InstallObservation {
                    admission_enabled: false,
                    result: InstallLookup::Unavailable { reason },
                }),
            });
            return;
        }
    };
    tokio::spawn(async move {
        let observation = store.install_step(&step, &plugin, &lease, admitted).await;
        if let Ok(plugins) = store.inventory() {
            let _ = events.send(MachineEvent::PluginInventory {
                plugins,
                observed_at_ms: super::unix_ms(),
            });
        }
        let _ = events.send(MachineEvent::PluginInstallationStep {
            request_id,
            observation: Box::new(observation),
        });
    });
}

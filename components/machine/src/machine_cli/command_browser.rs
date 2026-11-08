//! `MachineCommand` family routed by `handle_machine_command`: Machine browser
//! instances. Until the instance manager lands, every command answers with
//! the instance hibernated and `not_available`, so a Controller that already
//! speaks protocol 43 observes a closed, truthful state instead of a timeout.

use super::*;
use crate::machine_protocol::machine_browser::{
    Command, Event, InstanceRef, Reason, State, StateEvent,
};

pub(super) fn handle(command: MachineCommand, context: MachineCommandContext) {
    let MachineCommandContext { events, .. } = context;
    let MachineCommand::Browser {
        request_id,
        browser: command,
    } = command
    else {
        return;
    };
    if let Err(error) = command.validate() {
        tracing::warn!(%error, request = %request_id, "rejected malformed browser command");
        return;
    }
    let instance = match &command {
        Command::Open {
            account_id,
            instance_id,
            ..
        } => InstanceRef {
            account_id: account_id.clone(),
            instance_id: instance_id.clone(),
            incarnation: 0,
        },
        Command::Attach { instance, .. }
        | Command::Lease { instance, .. }
        | Command::Detach { instance, .. }
        | Command::Resize { instance, .. }
        | Command::Navigate { instance, .. }
        | Command::Snapshot { instance }
        | Command::Hibernate { instance } => instance.clone(),
        Command::Reset { account_id, .. } => InstanceRef {
            account_id: account_id.clone(),
            instance_id: String::new(),
            incarnation: 0,
        },
    };
    let _ = events.send(MachineEvent::Browser {
        browser: Event::State(StateEvent {
            request_id: Some(request_id),
            instance,
            state: State::Hibernated,
            reason: Some(Reason::NotAvailable),
            geometry: None,
            scale: None,
            deadline_ms: None,
            attachment_id: None,
            snapshot_generation: None,
            snapshot_at_ms: None,
            unsaved_state_lost: false,
            observed_at_ms: now_ms(),
        }),
    });
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

//! `MachineCommand` family routed by `handle_machine_command`: Machine browser
//! instances. Every command is handed to the process-wide instance manager,
//! which answers through the connection's event channel; a slow lifecycle
//! step never holds the Controller read loop. The manager is handed commands
//! in arrival order and keeps each account's lifecycle commands in it.

use super::*;

pub(super) fn handle(command: MachineCommand, context: MachineCommandContext) {
    let MachineCommandContext { browsers, .. } = context;
    let MachineCommand::Browser {
        request_id,
        browser: command,
    } = command
    else {
        return;
    };
    browsers.dispatch(request_id, command);
}

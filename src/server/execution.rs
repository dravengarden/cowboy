//! Service authorization and opaque forwarding for an enrolled runtime worker.
//! Provider/native JSON-RPC remains private to the endpoints.

use super::AppState;
use crate::execution_protocol::{RuntimeReply, RuntimeRequest, Scope};
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::execution::{Action, Refusal, Request, Response};

pub(super) mod sessions;

pub(super) async fn forward(
    state: &AppState,
    connection: &ConnectionToken,
    runtime_machine: &str,
    request: RuntimeRequest,
) -> RuntimeReply {
    let response = forward_checked(state, connection, runtime_machine, &request).await;
    RuntimeReply {
        session_id: request.session_id,
        worker_epoch: request.worker_epoch,
        request_id: request.request_id,
        scope: Scope::from_binding(&request.binding),
        response,
    }
}

async fn forward_checked(
    state: &AppState,
    connection: &ConnectionToken,
    runtime_machine: &str,
    request: &RuntimeRequest,
) -> Response {
    let refused = |reason| Response::Refused { reason };
    if !state.machine_control.is_current(connection)
        || state
            .execution_closures
            .lock()
            .contains(&request.session_id)
        || request.binding.validate().is_err()
        || request.binding.runtime.machine_id != runtime_machine
        || !crate::execution_protocol::valid_operation_id(&request.request_id)
        || !state
            .hub
            .session_info(&request.session_id)
            .is_some_and(|info| {
                info.meta.machine_id == runtime_machine
                    && info.meta.require_runtime_launch().is_ok()
                    && info
                        .meta
                        .execution_binding
                        .as_ref()
                        .and_then(|binding| {
                            binding.for_runtime(runtime_machine, &info.meta.cwd).ok()
                        })
                        .is_some_and(|binding| binding == request.binding)
            })
    {
        return refused(Refusal::IdentityMismatch);
    }
    // A startup snapshot and its first private request use different internal
    // queues. Missing ownership is retryable, never permission to run locally.
    if !state
        .runtime_router
        .runtime(runtime_machine)
        .is_some_and(|runtime| runtime.execution_owner_matches(request))
    {
        return refused(Refusal::Unavailable);
    }
    let machine_id = &request.binding.environment.machine_id;
    state
        .machine_control
        .execution_request(
            machine_id,
            Request {
                service_id: state.service_id.clone(),
                machine_id: machine_id.clone(),
                action: Action::Call {
                    session_id: request.session_id.clone(),
                    binding: Box::new(request.binding.clone()),
                    command: request.command.clone(),
                },
            },
        )
        .await
        .unwrap_or_else(|_| refused(Refusal::Unavailable))
}

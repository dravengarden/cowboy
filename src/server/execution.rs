//! Service authorization and opaque forwarding for an enrolled runtime worker.
//! Provider/native JSON-RPC remains private to the endpoints.

use super::AppState;
use crate::execution_protocol::{Command, RuntimeReply, RuntimeRequest, Scope};
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::execution::{Action, Refusal, Request, Response};

pub(super) mod recovery;
pub(super) mod sessions;

/// Serialize lifecycle maintenance separately from admission of native calls.
/// Recovery fences through its durable non-runnable binding; observing an
/// already committed receipt must leave the healthy replacement usable.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Maintenance {
    Close,
    Recover,
}

pub(super) async fn forward(
    state: &AppState,
    connection: &ConnectionToken,
    runtime_machine: &str,
    request: RuntimeRequest,
) -> RuntimeReply {
    let started = std::time::Instant::now();
    let response = forward_checked(state, connection, runtime_machine, &request).await;
    observe_slow_forward(&request, started.elapsed());
    RuntimeReply {
        session_id: request.session_id,
        worker_epoch: request.worker_epoch,
        request_id: request.request_id,
        scope: Scope::from_binding(&request.binding),
        response,
    }
}

/// The worker logs each invocation's whole duration under the same operation
/// identity. A slow one logged here too spent that time at or behind the
/// Controller (target execution, long-poll waits); one absent here was lost
/// in transport or queues between the worker and the Controller.
const SLOW_FORWARD: std::time::Duration = std::time::Duration::from_secs(2);

fn observe_slow_forward(request: &RuntimeRequest, elapsed: std::time::Duration) {
    if elapsed < SLOW_FORWARD {
        return;
    }
    let (kind, operation) = match &request.command {
        Command::Invoke { invocation, .. } => ("invoke", invocation.operation_id.as_str()),
        Command::Observe { operation_id, .. } => ("observe", operation_id.as_str()),
        _ => return,
    };
    tracing::info!(
        event_name = "cowboy.execution.forward_observed",
        session = %request.session_id,
        environment_id = %request.binding.environment.id,
        %operation,
        kind,
        duration_ms = elapsed.as_secs_f64() * 1000.0,
        "slow execution forward observed"
    );
}

async fn forward_checked(
    state: &AppState,
    connection: &ConnectionToken,
    runtime_machine: &str,
    request: &RuntimeRequest,
) -> Response {
    let refused = |reason| Response::Refused { reason };
    if !state.machine_control.is_current(connection)
        || state.execution_closures.lock().get(&request.session_id) == Some(&Maintenance::Close)
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

//! Resolve one logical Session and its original core-owned execution route.

use crate::core::{CodeReadScope, Hub};
use crate::machine_control::{MachineControl, SessionReadScope};

pub(in crate::server) fn resolve(
    hub: &Hub,
    control: &MachineControl,
    service: &str,
    id: &str,
) -> Option<CodeReadScope> {
    let scope = control.session_read_scope(service, hub.session_code_scope(id)?)?;
    current(hub, control, &scope).then_some(CodeReadScope::Session(scope))
}

pub(in crate::server) fn current(
    hub: &Hub,
    control: &MachineControl,
    scope: &SessionReadScope,
) -> bool {
    hub.code_scope_is_current(scope.session()) && control.session_read_scope_is_current(scope)
}

/// Manifest readiness and the following filesystem read share the original
/// route. Readiness cannot switch to a newly installed connection in between.
pub(in crate::server) async fn worktree_ready(
    hub: &Hub,
    control: &MachineControl,
    local_socket: Option<&std::path::Path>,
    scope: &SessionReadScope,
) -> anyhow::Result<bool> {
    let request = serde_json::json!({
        "type": "ensureWorktree", "path": scope.session().cwd(), "trusted": true,
    });
    match adapter_request(hub, control, local_socket, scope, request).await? {
        crate::server::ZedAdapterResponse::Worktree { state, .. } => Ok(state == "ready"),
        _ => anyhow::bail!("unexpected Zed adapter response"),
    }
}

/// The buffered readers keep the connection captured before asynchronous
/// authorization. Never resolve it again from the logical Machine name.
pub(super) async fn adapter_request(
    hub: &Hub,
    control: &MachineControl,
    local_socket: Option<&std::path::Path>,
    scope: &SessionReadScope,
    request: serde_json::Value,
) -> anyhow::Result<crate::server::ZedAdapterResponse> {
    use crate::server::validate_zed_adapter_response;
    anyhow::ensure!(current(hub, control, scope), "code context changed");
    let response = if let Some(connection) = scope.connection() {
        let value = control
            .adapter_request_on_connection(connection, "zed", request)
            .await
            .map_err(anyhow::Error::msg)?;
        validate_zed_adapter_response(serde_json::from_value(value)?)?
    } else {
        crate::server::zed_request_in_scope(hub, control, local_socket, scope.session(), request)
            .await?
    };
    anyhow::ensure!(current(hub, control, scope), "code context changed");
    Ok(response)
}

#[cfg(test)]
mod tests;

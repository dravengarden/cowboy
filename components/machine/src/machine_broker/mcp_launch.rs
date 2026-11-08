//! Mandatory MCP metadata cannot be dropped during launch or adoption.
use super::{Broker, StartSession, account_worker};
use anyhow::{Context as _, Result, ensure};

impl Broker {
    pub(super) fn mcp_launch_adoption_changed(&self, session: &StartSession) -> bool {
        session
            .execution_binding
            .as_ref()
            .is_some_and(|binding| binding.decode().is_err() && binding.managed_child().is_none())
            || self
                .sessions
                .lock()
                .get(&session.session_id)
                .is_some_and(|previous| {
                    previous.execution_binding != session.execution_binding
                        || previous.mcp_plan != session.mcp_plan
                })
            || self
                .workers
                .lock()
                .get(&session.session_id)
                .and_then(|worker| worker.snapshot.launch.as_ref())
                .is_some_and(|launch| {
                    launch.execution_binding != session.execution_binding
                        || launch.mcp_plan != session.mcp_plan
                })
    }
}

pub(super) fn same_placement(old: &StartSession, session: &StartSession) -> bool {
    old.cwd == session.cwd && old.mcp_plan == session.mcp_plan
}

pub(super) fn validate(
    session: &StartSession,
    provider: Option<&crate::machine_plugins::ProviderLaunchContext>,
) -> Result<()> {
    if let Some(plan) = &session.mcp_plan {
        plan.validate()?;
        ensure!(
            !session.system
                && session.extensions.is_some()
                && provider
                    .is_some_and(|installed| installed.mcp_bindings && installed.execution_jsonrpc)
                && session
                    .execution_binding
                    .as_ref()
                    .is_some_and(|binding| binding
                        .decode()
                        .is_ok_and(|binding| binding.managed.is_none())),
            "MCP launch requires an ordinary bound session and a capable signed Provider"
        );
    }
    Ok(())
}

impl Broker {
    /// Session extensions are resolved from this Machine's own active Plugins.
    /// Without a required MCP plan a failure disables them explicitly: the
    /// session starts without memory and never falls back to a host enrollment.
    pub(super) fn launch_extensions_environment(
        &self,
        session: &StartSession,
    ) -> Result<Option<String>> {
        let Some(switches) = session.extensions else {
            return Ok(None);
        };
        let written = self
            .args
            .call_context_root
            .as_deref()
            .context("Machine has no state root for session extensions")
            .and_then(|root| {
                self.args.provider_store.write_session_extensions(
                    root,
                    &session.session_id,
                    switches,
                    // A managed child never receives memory.
                    self.args
                        .memory_relay
                        .as_ref()
                        .filter(|_| !session.system)
                        .map(|relay| relay.session_access(&session.session_id)),
                    session.mcp_plan.as_deref(),
                )
            });
        let value = match written {
            Ok(path) => path.display().to_string(),
            Err(error) => {
                if session.mcp_plan.is_some() {
                    return Err(error.context("required MCP launch descriptor unavailable"));
                }
                tracing::warn!(
                    session = %session.session_id,
                    error = %format!("{error:#}"),
                    "session extensions disabled for this launch"
                );
                crate::machine_plugins::SESSION_EXTENSIONS_DISABLED.to_owned()
            }
        };
        Ok(Some(value))
    }
}

pub(super) fn requires_launch(session: &StartSession) -> bool {
    !session.provider_account_id.is_default() || session.mcp_plan.is_some()
}

pub(super) fn binding_matches(previous: &StartSession, incoming: &StartSession) -> bool {
    account_worker::binding_matches(previous, incoming)
        && previous.mcp_plan == incoming.mcp_plan
        && (previous.mcp_plan.is_none()
            || (previous.provider == incoming.provider
                && previous.provider_version == incoming.provider_version
                && previous.provider_generation_digest == incoming.provider_generation_digest
                && previous.provider_auth_generation == incoming.provider_auth_generation))
}

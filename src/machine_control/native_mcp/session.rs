//! Preparation using the exact keeper's observed identity, never a caller's CLI.
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};

use super::{
    ConnectionToken, MachineControl, NativeMcpLaunchPlan, NativeMcpLaunchPreparation,
    ProcessProfile,
};
use crate::execution_environment::BindingV1;
use crate::execution_protocol::{
    AGENT_CLI_INITIALIZATION_KEY, Command, Executor, Response as KeeperResponse, Scope,
};
use crate::machine_protocol::execution::{Action, Request, Response};

pub struct NativeMcpSessionPreparation {
    launch: NativeMcpLaunchPreparation,
    binding: BindingV1,
    cli: Executor,
    connection: ConnectionToken,
}

impl NativeMcpSessionPreparation {
    pub fn plan(&self) -> &NativeMcpLaunchPlan {
        self.launch.plan()
    }
    pub fn binding(&self) -> &BindingV1 {
        &self.binding
    }
    pub fn cli(&self) -> &Executor {
        &self.cli
    }
    /// This checks execution identity only; it never substitutes for live policy.
    pub fn is_current(&self, control: &MachineControl, binding: &BindingV1) -> bool {
        self.binding == *binding && control.is_current(&self.connection)
    }
}

impl MachineControl {
    /// Live policy remains the caller's obligation before and after this await.
    /// The returned owner retains receipts and the execution identity observed here.
    pub async fn prepare_session_native_mcp(
        self: &Arc<Self>,
        connection: &ConnectionToken,
        binding: BindingV1,
        profiles: Vec<ProcessProfile>,
    ) -> Result<NativeMcpSessionPreparation> {
        binding.validate().map_err(anyhow::Error::msg)?;
        super::plan::validate_profiles(connection, &profiles)?;
        ensure!(binding.managed.is_none(), "managed MCP launch denied");
        for profile in &profiles {
            ensure!(
                profile.service_id == self.service.as_str()
                    && profile.execution_machine == binding.environment.machine_id
                    && profile.runtime_machine == binding.runtime.machine_id
                    && profile.workspace_id == binding.workspace.id,
                "MCP policy and execution identity differ"
            );
        }
        let response = self
            .execution_request_on_connection(
                connection,
                Request {
                    service_id: self.service.as_str().into(),
                    machine_id: connection.machine_id().into(),
                    action: Action::Call {
                        session_id: profiles[0].session_id.clone(),
                        binding: Box::new(binding.clone()),
                        command: Command::Describe,
                    },
                },
            )
            .await
            .map_err(anyhow::Error::msg)?;
        let cli = observed_cli(response, &binding)?;
        let launch = self
            .prepare_native_mcp_plan(connection, &cli, profiles)
            .await?;
        ensure!(
            self.is_current(connection),
            "MCP execution connection changed"
        );
        Ok(NativeMcpSessionPreparation {
            launch,
            binding,
            cli,
            connection: connection.clone(),
        })
    }
}

fn observed_cli(response: Response, binding: &BindingV1) -> Result<Executor> {
    let Response::Call {
        response:
            KeeperResponse::Ready {
                scope,
                initialization,
                ..
            },
    } = response
    else {
        anyhow::bail!("MCP execution keeper is not ready");
    };
    ensure!(
        scope == Scope::from_binding(binding),
        "MCP execution incarnation changed"
    );
    let value = initialization
        .get(AGENT_CLI_INITIALIZATION_KEY)
        .context("keeper has no pinned CLI identity")?;
    let cli: Executor =
        serde_json::from_value(value.clone()).context("invalid keeper CLI identity")?;
    crate::execution_protocol::agent_cli_directory(&cli).map_err(anyhow::Error::msg)?;
    Ok(cli)
}

#[cfg(test)]
mod tests;

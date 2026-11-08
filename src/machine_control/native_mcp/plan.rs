//! A launch owns all confirmed descriptors or none. Policy and runtime readiness
//! remain separate checks at delivery; a prepared plan grants no authority.
use super::{ConnectionToken, MachineControl, NativeMcpPreparation, ProcessProfile};
use crate::mcp_binding::NativeLaunchBinding as NativeMcpLaunchBinding;
pub use crate::mcp_binding::NativeLaunchPlan as NativeMcpLaunchPlan;
use anyhow::{Result, ensure};
use std::sync::Arc;

pub struct NativeMcpLaunchPreparation {
    plan: NativeMcpLaunchPlan,
    _receipts: Vec<NativeMcpPreparation>,
}

impl NativeMcpLaunchPreparation {
    pub fn plan(&self) -> &NativeMcpLaunchPlan {
        &self.plan
    }
}

impl MachineControl {
    /// The caller supplies the target keeper's verified CLI identity and must
    /// retain this owner until the matching worker can no longer read its plan.
    pub async fn prepare_native_mcp_plan(
        self: &Arc<Self>,
        connection: &ConnectionToken,
        cli: &crate::execution_protocol::Executor,
        profiles: Vec<ProcessProfile>,
    ) -> Result<NativeMcpLaunchPreparation> {
        crate::execution_protocol::agent_cli_directory(cli).map_err(anyhow::Error::msg)?;
        validate_profiles(connection, &profiles)?;
        // At most MAX_BINDINGS operations; a failure drops completed owners and
        // cancels the rest, whose unknown pending receipts expire on the Machine.
        let receipts = futures::future::try_join_all(
            profiles
                .into_iter()
                .map(|profile| self.prepare_native_mcp(connection, profile)),
        )
        .await?;
        ensure!(self.is_current(connection), "MCP launch connection changed");
        let plan = NativeMcpLaunchPlan {
            schema: 1,
            command: cli.command.clone(),
            bindings: receipts
                .iter()
                .map(|receipt| NativeMcpLaunchBinding {
                    name: receipt.descriptor.binding.name.clone(),
                    descriptor: receipt.path().to_string_lossy().into_owned(),
                })
                .collect(),
        };
        Ok(NativeMcpLaunchPreparation {
            plan,
            _receipts: receipts,
        })
    }
}

pub(super) fn validate_profiles(
    connection: &ConnectionToken,
    profiles: &[ProcessProfile],
) -> Result<()> {
    ensure!(
        (1..=crate::mcp_binding::MAX_BINDINGS).contains(&profiles.len()),
        "MCP launch binding count is invalid"
    );
    crate::mcp_binding::validate_bindings(
        &profiles
            .iter()
            .map(|profile| profile.binding.clone())
            .collect::<Vec<_>>(),
    )?;
    let first = &profiles[0];
    for profile in profiles {
        profile.validate()?;
        ensure!(
            profile.service_id == first.service_id
                && profile.session_id == first.session_id
                && profile.runtime_machine == first.runtime_machine
                && profile.execution_machine == connection.machine_id()
                && profile.workspace_id == first.workspace_id
                && profile.epoch == first.epoch,
            "MCP launch mixes policy or placement"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;

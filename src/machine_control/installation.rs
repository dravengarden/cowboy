//! Installation replies are bound to one live connection, exact request and
//! distinct reply kind. No missing reply authorizes a resend.
#![cfg_attr(not(test), allow(dead_code))]

use super::{
    CommandFailure, CommandRequestError, ConnectionToken, MachineCommand, MachineControl,
    PROVIDER_COMMAND_TIMEOUT, Reply, ReplyKind, RequestBinding,
};
use crate::machine_protocol::DesiredPlugin;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallObservation, InstallStep, InstallTargetObservation, InstallTargetQuery,
};

fn fail(certainty: CommandFailure, detail: &str) -> CommandRequestError {
    CommandRequestError {
        certainty,
        detail: detail.to_owned(),
    }
}

impl MachineControl {
    pub(crate) async fn plugin_installation_target(
        &self,
        token: &ConnectionToken,
        query: &InstallTargetQuery,
    ) -> Result<InstallTargetObservation, CommandRequestError> {
        let expected = query
            .digest()
            .map_err(|_| fail(CommandFailure::NotSent, "invalid installation target query"))?;
        if query.machine_id != token.0.machine_id {
            return Err(fail(
                CommandFailure::NotSent,
                "installation target mismatch",
            ));
        }
        let request_id = self.request_id("install-target").map_err(|_| {
            fail(
                CommandFailure::NotSent,
                "installation query identity unavailable",
            )
        })?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::ObservePluginInstallation {
                    request_id: request_id.clone(),
                    query: Box::new(query.clone()),
                },
                ReplyKind::InstallationTarget,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "installation query channel unavailable",
                )
            })?;
        match tokio::time::timeout(PROVIDER_COMMAND_TIMEOUT, rx).await {
            Ok(Ok(Reply::InstallationTarget(observation))) => {
                if let InstallTargetObservation::Observed {
                    query_digest,
                    target,
                    ..
                } = &*observation
                    && (query_digest != &expected || target.validate().is_err())
                {
                    return Err(fail(
                        CommandFailure::Unknown,
                        "installation target evidence mismatch",
                    ));
                }
                Ok(*observation)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "installation target evidence unavailable",
            )),
        }
    }

    /// `None` is a read-only query, never a recoverable execution grant.
    pub(crate) async fn plugin_installation_step(
        &self,
        token: &ConnectionToken,
        step: &InstallStep,
        desired: Option<&DesiredPlugin>,
    ) -> Result<InstallObservation, CommandRequestError> {
        step.validate()
            .map_err(|_| fail(CommandFailure::NotSent, "invalid installation step"))?;
        if step.machine_id != token.0.machine_id
            || desired.is_some_and(|plugin| !step.matches_envelope(plugin))
        {
            return Err(fail(
                CommandFailure::NotSent,
                "installation target or envelope mismatch",
            ));
        }
        let request_id = self
            .request_id("install-step")
            .map_err(|_| fail(CommandFailure::NotSent, "installation identity unavailable"))?;
        let command = if let Some(plugin) = desired {
            MachineCommand::InstallPluginStep {
                request_id: request_id.clone(),
                step: Box::new(step.clone()),
                plugin: Box::new(plugin.clone()),
            }
        } else {
            MachineCommand::QueryPluginInstallStep {
                request_id: request_id.clone(),
                step: Box::new(step.clone()),
            }
        };
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                command,
                ReplyKind::InstallationStep,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "installation step channel unavailable",
                )
            })?;
        match tokio::time::timeout(PROVIDER_COMMAND_TIMEOUT, rx).await {
            Ok(Ok(Reply::InstallationStep(observation))) => {
                if let InstallLookup::Found { receipt } = &observation.result
                    && !receipt.matches(step)
                {
                    return Err(fail(
                        CommandFailure::Unknown,
                        "installation receipt identity mismatch",
                    ));
                }
                Ok(*observation)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "installation receipt unavailable",
            )),
        }
    }
}

#[cfg(test)]
mod tests;

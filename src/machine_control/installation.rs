//! Installation replies are bound to one live connection, exact request and
//! distinct reply kind. No missing reply authorizes a resend.

use super::{
    CommandFailure, CommandRequestError, ConnectionToken, MachineCommand, MachineControl,
    PROVIDER_COMMAND_TIMEOUT, Reply, ReplyKind, RequestBinding,
};
use crate::machine_protocol::DesiredPlugin;
use crate::machine_protocol::plugin_install::{
    InstallLookup, InstallObservation, InstallStep, InstallTargetObservation, InstallTargetQuery,
};
use std::time::Duration;

/// The Machine bounds one installation by the step deadline and never beyond
/// five minutes from receipt; the grace covers its final receipt fsync and the
/// reply's transport after the lease ends.
const INSTALL_REPLY_GRACE: Duration = Duration::from_secs(15);
const MAX_INSTALL_REPLY_WAIT: Duration = Duration::from_secs(5 * 60 + 15);

fn fail(certainty: CommandFailure, detail: &str) -> CommandRequestError {
    CommandRequestError {
        certainty,
        detail: detail.to_owned(),
    }
}

/// How long to observe an executing step before its outcome becomes unknown.
/// Staging a large runtime over a slow artifact path routinely outlasts the
/// generic command timeout while the Machine still holds a live lease, so an
/// executing step waits for that lease. A read-only query keeps the short bound.
fn install_reply_timeout(step: &InstallStep, executing: bool, now_ms: i64) -> Duration {
    if !executing {
        return PROVIDER_COMMAND_TIMEOUT;
    }
    let lease = u64::try_from(step.expires_at_ms.saturating_sub(now_ms)).unwrap_or_default();
    (Duration::from_millis(lease) + INSTALL_REPLY_GRACE)
        .clamp(PROVIDER_COMMAND_TIMEOUT, MAX_INSTALL_REPLY_WAIT)
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
        if !self.matches_site(token, &query.service_id, &query.machine_id) {
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
        if !self.matches_site(token, &step.service_id, &step.machine_id)
            || desired.is_some_and(|plugin| !step.matches_envelope(plugin))
        {
            return Err(fail(
                CommandFailure::NotSent,
                "installation target or envelope mismatch",
            ));
        }
        let reply_timeout = install_reply_timeout(
            step,
            desired.is_some(),
            chrono::Utc::now().timestamp_millis(),
        );
        let request_id = if desired.is_some() {
            format!("plugin-install-{}", step.operation_id)
        } else {
            self.request_id("install-query")
                .map_err(|_| fail(CommandFailure::NotSent, "installation identity unavailable"))?
        };
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
        match tokio::time::timeout(reply_timeout, rx).await {
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

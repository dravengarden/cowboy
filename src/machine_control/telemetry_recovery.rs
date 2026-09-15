//! Distinct reply kinds and original-connection correlation; never resends.
#![cfg_attr(not(test), allow(dead_code))]
use super::{
    CommandFailure, CommandRequestError, ConnectionToken, MachineCommand, MachineControl, Reply,
    ReplyKind, RequestBinding,
};
use crate::machine_protocol::telemetry_recovery::{
    RecoveryObservation, RecoveryRequest, RecoveryResult,
};
use crate::machine_protocol::telemetry_recovery_audit::{
    RecoveryAuditObservation, RecoveryAuditQuery,
};

fn fail(certainty: CommandFailure, detail: &str) -> CommandRequestError {
    CommandRequestError {
        certainty,
        detail: detail.into(),
    }
}

impl MachineControl {
    pub(crate) fn telemetry_recovery_audit_target_current(
        &self,
        token: &ConnectionToken,
        query: &RecoveryAuditQuery,
    ) -> bool {
        query.digest().is_ok()
            && self.matches_site(token, &query.step.service_id, &query.step.machine_id)
            && self.connection_supports(
                token,
                crate::machine_protocol::TELEMETRY_RECOVERY_AUDIT_PROTOCOL_VERSION,
            )
    }

    pub(crate) async fn telemetry_recovery_audit(
        &self,
        token: &ConnectionToken,
        query: &RecoveryAuditQuery,
    ) -> Result<RecoveryAuditObservation, CommandRequestError> {
        if !self.telemetry_recovery_audit_target_current(token, query) {
            return Err(fail(
                CommandFailure::NotSent,
                "recovery audit target unavailable",
            ));
        }
        let request_id = self.request_id("binding-recovery-audit").map_err(|_| {
            fail(
                CommandFailure::NotSent,
                "recovery audit identity unavailable",
            )
        })?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::QueryTelemetryRecoveryAudit {
                    request_id: request_id.clone(),
                    query: Box::new(query.clone()),
                },
                ReplyKind::TelemetryRecoveryAudit,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "recovery audit channel unavailable",
                )
            })?;
        match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
            Ok(Ok(Reply::TelemetryRecoveryAudit(observation))) if observation.matches(query) => {
                Ok(*observation)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "recovery audit evidence unavailable",
            )),
        }
    }

    pub(crate) fn telemetry_recovery_target_current(
        &self,
        token: &ConnectionToken,
        request: &RecoveryRequest,
    ) -> bool {
        request.validate().is_ok()
            && self.matches_site(token, &request.step.service_id, &request.step.machine_id)
            && self.connection_supports(
                token,
                crate::machine_protocol::TELEMETRY_BINDING_RECOVERY_PROTOCOL_VERSION,
            )
    }

    pub(crate) async fn recover_telemetry_binding(
        &self,
        token: &ConnectionToken,
        request: &RecoveryRequest,
    ) -> Result<RecoveryObservation, CommandRequestError> {
        if !self.telemetry_recovery_target_current(token, request) {
            return Err(fail(
                CommandFailure::NotSent,
                "binding recovery channel or target unavailable",
            ));
        }
        let request_id = self.request_id("binding-recovery").map_err(|_| {
            fail(
                CommandFailure::NotSent,
                "binding recovery identity unavailable",
            )
        })?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::RecoverTelemetryBinding {
                    request_id: request_id.clone(),
                    recovery: Box::new(request.clone()),
                },
                ReplyKind::TelemetryRecoveryCommit,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "binding recovery channel unavailable",
                )
            })?;
        match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
            Ok(Ok(Reply::TelemetryRecoveryCommit(result))) => match *result {
                RecoveryResult::Observed { observation } if observation.matches(request) => {
                    Ok(observation)
                }
                RecoveryResult::Unavailable {
                    failure:
                        crate::machine_protocol::telemetry_binding::BindingCommitFailure::Unavailable(
                            crate::machine_protocol::telemetry_binding::BindingUnavailable::Storage,
                        ),
                } => Err(fail(
                    CommandFailure::Unknown,
                    "binding recovery persistence uncertain",
                )),
                RecoveryResult::Unavailable { .. } => Err(fail(
                    CommandFailure::Rejected,
                    "binding recovery was not admitted",
                )),
                RecoveryResult::Observed { .. } => Err(fail(
                    CommandFailure::Unknown,
                    "binding recovery evidence mismatch",
                )),
            },
            _ => Err(fail(
                CommandFailure::Unknown,
                "binding recovery receipt unavailable",
            )),
        }
    }

    pub(crate) async fn telemetry_recovery_observation(
        &self,
        token: &ConnectionToken,
        request: &RecoveryRequest,
    ) -> Result<RecoveryObservation, CommandRequestError> {
        if !self.telemetry_recovery_target_current(token, request) {
            return Err(fail(
                CommandFailure::NotSent,
                "binding recovery query target unavailable",
            ));
        }
        let request_id = self.request_id("binding-recovery-query").map_err(|_| {
            fail(
                CommandFailure::NotSent,
                "binding recovery query identity unavailable",
            )
        })?;
        let (rx, _pending) = self
            .begin_request(
                &token.0.machine_id,
                &request_id,
                MachineCommand::QueryTelemetryRecovery {
                    request_id: request_id.clone(),
                    recovery: Box::new(request.clone()),
                },
                ReplyKind::TelemetryRecovery,
                Some(RequestBinding::Connection(token)),
            )
            .map_err(|_| {
                fail(
                    CommandFailure::NotSent,
                    "binding recovery query channel unavailable",
                )
            })?;
        match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
            Ok(Ok(Reply::TelemetryRecovery(observation))) if observation.matches(request) => {
                Ok(*observation)
            }
            _ => Err(fail(
                CommandFailure::Unknown,
                "binding recovery query evidence unavailable",
            )),
        }
    }
}

#[cfg(test)]
mod tests;

//! Owner-confirmed resets of a named native login's banked usage limit
//! (`ConsumeNativeReset`). The Machine holding the login runs the signed
//! collector; the Controller only dispatches and classifies the answer.

use std::future::Future;

use super::{ConnectionToken, MachineControl, Reply, ReplyKind, RequestBinding};
use crate::machine_protocol::{
    MachineCommand, NativeResetDispatch, NativeResetRequest, native_usage_error_code,
};

/// The margin after the dispatch expiry for the spend the Machine started
/// before it (the collector's own twelve-second process bound, plus slack).
const RESET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Why a dispatched reset has no collector answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResetFailure {
    /// The collector may have reached the Provider: the outcome is unknown and
    /// must never be retried automatically.
    pub call_may_have_reached_provider: bool,
    /// A closed reason code; never subprocess output.
    pub code: &'static str,
}

impl MachineControl {
    /// Enqueue under the owner's current native-login transaction. The returned
    /// future yields the collector's own reply (a result or its refusal).
    pub fn begin_native_reset<'a>(
        &'a self,
        connection: &'a ConnectionToken,
        profile: &crate::provider_account::NativeProfileReceipt,
        request: NativeResetRequest,
        dispatch: NativeResetDispatch,
    ) -> Result<impl Future<Output = Result<serde_json::Value, NativeResetFailure>> + 'a, String>
    {
        if profile.account_id.is_default() || profile.generation == 0 {
            return Err("named_native_profile_required".into());
        }
        if !request.valid() {
            return Err("native_reset_request_invalid".into());
        }
        let expires_at_ms = dispatch.expires_at_ms;
        let request_id = self.request_id("native-reset")?;
        let (rx, pending) = self.begin_request(
            &connection.0.machine_id,
            &request_id,
            MachineCommand::ConsumeNativeReset {
                request_id: request_id.clone(),
                profile: Box::new(profile.clone()),
                request,
                dispatch,
            },
            ReplyKind::PluginHost,
            Some(RequestBinding::Connection(connection)),
        )?;
        // Wait for the Machine's whole bounded window (its paced second start
        // and dispatch expiry), then a margin for the spend itself.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        let window = u64::try_from(expires_at_ms.saturating_sub(now)).unwrap_or(0);
        let wait = std::time::Duration::from_millis(window) + RESET_TIMEOUT;
        Ok(async move {
            let _pending = pending;
            match tokio::time::timeout(wait, rx).await {
                Ok(Ok(Reply::PluginHost {
                    accepted: true,
                    payload: Some(value),
                    ..
                })) if !value.is_null() => Ok(value),
                Ok(Ok(Reply::PluginHost {
                    accepted: false,
                    started,
                    detail: Some(code),
                    ..
                })) => Err(NativeResetFailure {
                    call_may_have_reached_provider: started,
                    code: native_usage_error_code(&code),
                }),
                // A lost, malformed or late reply may follow a sent reset.
                _ => Err(NativeResetFailure {
                    call_may_have_reached_provider: true,
                    code: "native_reset_outcome_unknown",
                }),
            }
        })
    }

    /// Enqueue an immediate banked reset of a named portable account on the
    /// Machine running the account's exact selected release; the future
    /// awaits its closed reply within the dispatch window.
    ///
    /// # Errors
    /// When the command never left the Controller.
    pub fn begin_portable_reset<'a>(
        &'a self,
        machine_id: &str,
        plugin: &crate::machine_protocol::PluginInventory,
        request: crate::machine_protocol::portable_usage::PortableResetRequest,
    ) -> Result<impl Future<Output = Result<serde_json::Value, NativeResetFailure>> + 'a, String>
    {
        if request.account_id.is_default() || request.auth_generation == 0 || !request.reset.valid()
        {
            return Err("native_reset_request_invalid".into());
        }
        let binding = self
            .bind_plugin_host(
                machine_id,
                plugin,
                crate::machine_protocol::PluginHostOperation::ResetUsage,
            )
            .map_err(|_| "portable_usage_machine_unavailable".to_owned())?;
        let expires_at_ms = request.expires_at_ms;
        let request_id = self.request_id("portable-reset")?;
        let (rx, pending) = self.begin_request(
            machine_id,
            &request_id,
            MachineCommand::ConsumePortableReset {
                request_id: request_id.clone(),
                request: Box::new(request),
            },
            ReplyKind::PluginHost,
            Some(RequestBinding::Plugin(&binding)),
        )?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        let window = u64::try_from(expires_at_ms.saturating_sub(now)).unwrap_or(0);
        let wait = std::time::Duration::from_millis(window) + RESET_TIMEOUT;
        Ok(async move {
            let _pending = pending;
            match tokio::time::timeout(wait, rx).await {
                Ok(Ok(Reply::PluginHost {
                    accepted: true,
                    payload: Some(value),
                    ..
                })) if !value.is_null() => Ok(value),
                Ok(Ok(Reply::PluginHost {
                    accepted: false,
                    started,
                    detail: Some(code),
                    ..
                })) => Err(NativeResetFailure {
                    call_may_have_reached_provider: started,
                    code: portable_reset_code(&code),
                }),
                // A lost, malformed or late reply may follow a sent reset.
                _ => Err(NativeResetFailure {
                    call_may_have_reached_provider: true,
                    code: "native_reset_outcome_unknown",
                }),
            }
        })
    }
}

/// A portable reset refusal as a closed code: the native reset codes, then
/// the portable usage admission codes.
#[must_use]
pub fn portable_reset_code(code: &str) -> &'static str {
    match native_usage_error_code(code) {
        "native_usage_unavailable" => {
            crate::machine_protocol::portable_usage::portable_usage_error_code(code)
        }
        native => native,
    }
}

#[cfg(test)]
mod portable_tests {
    use super::*;

    #[test]
    fn portable_reset_refusals_are_closed_codes() {
        assert_eq!(
            portable_reset_code("native_reset_expired"),
            "native_reset_expired"
        );
        assert_eq!(
            portable_reset_code("portable_usage_replica_pending"),
            "portable_usage_replica_pending"
        );
        assert_eq!(
            portable_reset_code("collector said {\"token\":\"x\"}"),
            "portable_usage_collector_failed"
        );
    }
}

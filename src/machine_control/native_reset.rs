//! Owner-confirmed resets of a named native login's banked usage limit
//! (`ConsumeNativeReset`). The Machine holding the login runs the signed
//! collector; the Controller only dispatches and classifies the answer.

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
}

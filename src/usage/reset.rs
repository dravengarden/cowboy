//! Usage reset results, admission and the signed collector's reset reply.
//! Service collector accounts and named native accounts share these types, so
//! both keep one confirmation, pacing and unknown-outcome contract.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{PluginCommandRoute, limiter, now_ms};
use crate::machine_protocol::PluginHostOperation;
use crate::plugin_host::PluginUsageSpec;
use crate::plugin_process::run_plugin_command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetSchedule {
    pub fire_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetResult {
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_id: Option<String>,
}

/// A Provider-paced admission held for one reset attempt. It owns the reset
/// serialization, so the charged start is the actual invocation time. Dropping
/// an unused reservation keeps its charged start but records no failure.
pub struct ResetReservation {
    account: String,
    permit: Option<limiter::Permit>,
    _serial: tokio::sync::OwnedMutexGuard<()>,
}

impl ResetReservation {
    /// `account` names the paced lane (`Limiter::account_key`) the permit was
    /// started for; `serial` serializes resets of that lane until drop.
    pub fn new(
        account: String,
        permit: limiter::Permit,
        serial: tokio::sync::OwnedMutexGuard<()>,
    ) -> Self {
        Self {
            account,
            permit: Some(permit),
            _serial: serial,
        }
    }

    /// Take the permit for the invocation of the same lane, at most once.
    pub fn take(&mut self, account: &str) -> std::result::Result<limiter::Permit, ResetError> {
        if self.account != account {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!("usage reset binding changed; no reset was sent"),
            });
        }
        self.permit.take().ok_or_else(|| ResetError {
            call_may_have_reached_provider: false,
            credit_id: None,
            source: anyhow::anyhow!("usage reset reservation was already used"),
        })
    }
}

impl Drop for ResetReservation {
    fn drop(&mut self) {
        if let Some(permit) = self.permit.take() {
            permit.finish(limiter::Outcome::Unused, now_ms());
        }
    }
}

#[derive(Debug)]
pub struct ResetError {
    pub call_may_have_reached_provider: bool,
    pub credit_id: Option<String>,
    pub(super) source: anyhow::Error,
}

impl ResetError {
    pub fn new(
        call_may_have_reached_provider: bool,
        credit_id: Option<String>,
        message: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
    ) -> Self {
        Self {
            call_may_have_reached_provider,
            credit_id,
            source: anyhow::Error::msg(message),
        }
    }
}

impl std::fmt::Display for ResetError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(formatter)
    }
}

impl std::error::Error for ResetError {}

pub(super) async fn consume_plugin_reset(
    binding: &PluginUsageSpec,
    idempotency_key: &str,
    expected_credit_id: Option<&str>,
    route: PluginCommandRoute,
    machine_control: &crate::machine_control::MachineControl,
) -> std::result::Result<ResetResult, ResetError> {
    let request = json!({
        "operation": "consume_reset",
        "provider": binding.account,
        "idempotency_key": idempotency_key,
        "expected_credit_id": expected_credit_id,
    });
    let response = match route {
        PluginCommandRoute::Machine { machine_id, plugin } => machine_control
            .plugin_host_request(
                &machine_id,
                &plugin,
                PluginHostOperation::ResetUsage,
                request,
            )
            .await
            .map_err(|error| ResetError {
                call_may_have_reached_provider: error.started,
                credit_id: None,
                source: anyhow::anyhow!(error.detail),
            })?,
        PluginCommandRoute::Unavailable(detail) => {
            return Err(ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::anyhow!(detail),
            });
        }
        PluginCommandRoute::Bootstrap => {
            let Some((program, args)) = binding.reset_argv.split_first() else {
                return Err(ResetError {
                    call_may_have_reached_provider: false,
                    credit_id: None,
                    source: anyhow::anyhow!("plugin reset argv is empty"),
                });
            };
            let request = serde_json::to_vec(&request).map_err(|source| ResetError {
                call_may_have_reached_provider: false,
                credit_id: None,
                source: anyhow::Error::new(source).context("serialize plugin usage reset request"),
            })?;
            let output = run_plugin_command(program, args, &request)
                .await
                .map_err(|failure| ResetError {
                    call_may_have_reached_provider: failure.started,
                    credit_id: None,
                    source: failure.error,
                })?;
            if !output.status.success() {
                return Err(ResetError {
                    call_may_have_reached_provider: true,
                    credit_id: None,
                    source: anyhow::anyhow!(
                        "plugin usage reset exited {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                });
            }
            serde_json::from_slice(&output.stdout).map_err(|source| ResetError {
                call_may_have_reached_provider: true,
                credit_id: None,
                source: anyhow::Error::new(source).context("parsing plugin usage reset result"),
            })?
        }
    };
    parse_reset_response(&response)
}

/// The signed collector's `consume_reset` reply: its result, or its refusal
/// with the collector's own reached-Provider ambiguity, projected onto
/// bounded fields first. Only explicit evidence is a success or a refusal that
/// sent nothing; anything else may follow a sent reset, so it is unknown.
pub fn parse_reset_response(response: &Value) -> std::result::Result<ResetResult, ResetError> {
    let response = crate::machine_protocol::portable_usage::project_reset_reply(response);
    match serde_json::from_value::<PluginResetCommandResponse>(response) {
        Ok(PluginResetCommandResponse::Success(result)) => Ok(result),
        Ok(PluginResetCommandResponse::Failure { error }) => Err(ResetError {
            call_may_have_reached_provider: error.call_may_have_reached_provider,
            credit_id: error.credit_id,
            source: anyhow::anyhow!(error.message),
        }),
        Err(source) => Err(ResetError {
            call_may_have_reached_provider: true,
            credit_id: None,
            source: anyhow::Error::new(source).context("parsing plugin usage reset result"),
        }),
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PluginResetCommandResponse {
    Success(ResetResult),
    Failure { error: PluginResetCommandError },
}

#[derive(Debug, Deserialize)]
struct PluginResetCommandError {
    message: String,
    call_may_have_reached_provider: bool,
    #[serde(default)]
    credit_id: Option<String>,
}

//! Verified startup metadata with connection-bound cleanup. Owning a preparation
//! does not authorize execution; the Controller still needs live session policy.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

use super::{ConnectionToken, MachineCommand, MachineControl};
use crate::mcp_binding::{NativeDescriptor, ProcessProfile, TransportRequest};

mod plan;
mod session;
pub use plan::{NativeMcpLaunchPlan, NativeMcpLaunchPreparation};
pub use session::NativeMcpSessionPreparation;

pub struct NativeMcpPreparation {
    pub descriptor: NativeDescriptor,
    path: PathBuf,
    _receipt: Receipt,
}

impl NativeMcpPreparation {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

struct Receipt {
    control: Weak<MachineControl>,
    connection: ConnectionToken,
    handle: String,
}

impl Drop for Receipt {
    fn drop(&mut self) {
        if let Some(control) = self.control.upgrade() {
            let _ = control.send_on_connection(
                &self.connection,
                MachineCommand::McpTransport {
                    request_id: format!("native-release-{:032x}", rand::random::<u128>()),
                    request: TransportRequest::ReleaseNative {
                        handle: self.handle.clone(),
                    },
                },
            );
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    handle: String,
    path: String,
    descriptor: NativeDescriptor,
}

impl MachineControl {
    pub async fn prepare_native_mcp(
        self: &Arc<Self>,
        connection: &ConnectionToken,
        profile: ProcessProfile,
    ) -> Result<NativeMcpPreparation> {
        ensure!(
            profile.execution_machine == connection.machine_id(),
            "MCP target changed"
        );
        let response = self
            .mcp_request_on_connection(
                connection,
                TransportRequest::PrepareNative {
                    profile: Box::new(profile.clone()),
                },
            )
            .await
            .map_err(anyhow::Error::msg)?;
        let handle = response
            .get("handle")
            .and_then(serde_json::Value::as_str)
            .context("MCP preparation receipt missing")?
            .to_owned();
        TransportRequest::ReleaseNative {
            handle: handle.clone(),
        }
        .validate()?;
        // Own cleanup before validating metadata, so malformed replies cannot
        // leak a known valid receipt on the original connection.
        let receipt = Receipt {
            control: Arc::downgrade(self),
            connection: connection.clone(),
            handle,
        };
        let prepared: Prepared = serde_json::from_value(response)?;
        prepared.descriptor.validate()?;
        ensure!(
            prepared.handle == receipt.handle
                && prepared.descriptor.binding == profile.binding
                && prepared.descriptor.epoch == profile.epoch
                && prepared.path.len() <= 4096
                && !prepared.path.contains('\0')
                && Path::new(&prepared.path).is_absolute()
                && self.is_current(connection),
            "MCP preparation does not match its admission"
        );
        let confirmed = self
            .mcp_request_on_connection(
                connection,
                TransportRequest::ConfirmNative {
                    handle: receipt.handle.clone(),
                },
            )
            .await
            .map_err(anyhow::Error::msg)?;
        ensure!(
            confirmed == serde_json::json!({"confirmed":true}) && self.is_current(connection),
            "MCP preparation confirmation failed"
        );
        Ok(NativeMcpPreparation {
            descriptor: prepared.descriptor,
            path: prepared.path.into(),
            _receipt: receipt,
        })
    }
}

#[cfg(test)]
mod tests;

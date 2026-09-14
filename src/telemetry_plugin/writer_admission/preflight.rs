//! Configuration-only inspection. No journal/identity, execution scope,
//! connection, destination, Plugin, exporter or payload is opened here.
use super::{MACHINE_POLICY_FILE, Purposes, WriterAdmission};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum WriterCheck {
    Unconfigured,
    ConfigurationValid { declared_purposes: Purposes },
}

/// Closed observations, never grants. In particular the persisted Machine id
/// may override CLI input, and enrollment may replace it again. Inspection must
/// not claim an effective owner or readiness from a policy's own declarations.
#[derive(Debug, Serialize)]
pub(crate) struct PreflightReport {
    schema: &'static str,
    writer_policy: WriterCheck,
    not_checked: [&'static str; 6],
}

pub(crate) fn inspect(state_dir: &Path) -> Result<PreflightReport> {
    let policy = WriterAdmission::load_optional(&state_dir.join(MACHINE_POLICY_FILE))?;
    Ok(PreflightReport {
        schema: "dravengarden.cowboy.machine-telemetry-writer-preflight/v1",
        writer_policy: policy.map_or(WriterCheck::Unconfigured, |policy| {
            WriterCheck::ConfigurationValid {
                declared_purposes: policy.config.purposes.clone(),
            }
        }),
        not_checked: [
            "runtime_and_enrolled_owner_identity",
            "other_machine_startup_configuration",
            "binding_journal_and_writer_activation",
            "signed_installation_and_private_destination_policy",
            "operator_authorization_and_connection",
            "local_recording_and_remote_delivery",
        ],
    })
}

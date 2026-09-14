//! Shared startup validation. Inspection drops policies without activating them,
//! creating authority, opening the Service store or consulting a Machine.
use super::{background_policy::BackgroundPolicy, writer_admission::WriterAdmission};
use crate::cli::ServeArgs;
use anyhow::{Result, ensure};
use serde::Serialize;
use std::sync::Arc;

pub(crate) struct ControllerPolicy {
    pub writer: Option<Arc<WriterAdmission>>,
    pub background: Option<Arc<BackgroundPolicy>>,
}

impl ControllerPolicy {
    pub(crate) fn load(args: &ServeArgs, service: &str) -> Result<Self> {
        let writer = args
            .telemetry_writer_policy
            .as_deref()
            .map(WriterAdmission::load)
            .transpose()?;
        ensure!(
            writer
                .as_ref()
                .is_none_or(|policy| policy.owns_service(service))
                && (writer.is_none() || args.database_url().is_some()),
            "telemetry writer policy requires its exact durable Service"
        );
        // Clap is not the only constructor: programmatic callers must reject
        // ambiguous modes too. Neither path consults the legacy selection file.
        ensure!(
            args.telemetry_managed_export_policy.is_none()
                || args.telemetry_plugin_config.is_none(),
            "managed and legacy telemetry configurations are mutually exclusive"
        );
        let background = args
            .telemetry_managed_export_policy
            .as_deref()
            .map(|path| BackgroundPolicy::load(path, service))
            .transpose()?;
        ensure!(
            background.is_none() || args.database_url().is_some(),
            "managed telemetry export requires the durable Service store"
        );
        Ok(Self { writer, background })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum PolicyCheck {
    Unconfigured,
    ConfigurationValid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum LegacyCheck {
    Unconfigured,
    NotChecked,
}

/// Closed, credential-free observations, never serialized permits or claims of
/// writer/export readiness. No path, owner, binding, epoch or policy digest.
#[derive(Debug, Serialize)]
pub(crate) struct PreflightReport {
    schema: &'static str,
    writer_policy: PolicyCheck,
    managed_background_policy: PolicyCheck,
    legacy_selection: LegacyCheck,
    not_checked: [&'static str; 6],
}

pub(crate) fn inspect(args: &ServeArgs) -> Result<PreflightReport> {
    let policy = if args.telemetry_writer_policy.is_some()
        || args.telemetry_managed_export_policy.is_some()
    {
        // Managed policy cannot choose/create its own Service identity. Plain
        // unconfigured host checks still work before first Service startup.
        let service = crate::service_identity::inspect(&args.data_dir)?;
        ControllerPolicy::load(args, &service)?
    } else {
        ControllerPolicy {
            writer: None,
            background: None,
        }
    };
    Ok(PreflightReport {
        schema: "dravengarden.cowboy.telemetry-policy-preflight/v1",
        writer_policy: if policy.writer.is_some() {
            PolicyCheck::ConfigurationValid
        } else {
            PolicyCheck::Unconfigured
        },
        managed_background_policy: if policy.background.is_some() {
            PolicyCheck::ConfigurationValid
        } else {
            PolicyCheck::Unconfigured
        },
        // Legacy startup first consults the durable fence. Inspection must not
        // read/adopt a possibly obsolete file to invent managed authority.
        legacy_selection: if args.telemetry_plugin_config.is_some() {
            LegacyCheck::NotChecked
        } else {
            LegacyCheck::Unconfigured
        },
        not_checked: [
            "database_connectivity_and_binding_journal",
            "current_binding_and_background_activation",
            "machine_admission_installation_and_destination_policy",
            "operator_authorization_and_connection",
            "legacy_fence_and_selection",
            "local_recording_and_remote_delivery",
        ],
    })
}

#[cfg(test)]
mod tests;

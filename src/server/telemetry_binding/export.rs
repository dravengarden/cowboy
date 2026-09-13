//! Staged single-attempt managed egress. No production endpoint/background
//! activation: accepted reader floors and independently authorized recovery
//! remain prerequisites. Completed binding receipts cannot construct this.
#![cfg_attr(not(feature = "machine-host"), allow(dead_code))]

use crate::machine_control::{ConnectionToken, MachineControl};
use crate::machine_protocol::telemetry_export::{ExportAttempt, ExportOutcome, ExportReceipt};
use crate::plugin_catalog::PluginCatalog;
use crate::server::operator_approval::{OperatorApproval, TelemetryExportAuthority};
use crate::server::{PluginLifecycleFences, ProductRequestAuth};
use crate::store::Store;
use anyhow::{Result, ensure};
use std::sync::Arc;

pub(super) struct ExportScope {
    attempt: ExportAttempt,
    authority: TelemetryExportAuthority,
    connection: ConnectionToken,
    control: Arc<MachineControl>,
    catalog: Arc<PluginCatalog>,
    fences: PluginLifecycleFences,
}

impl ExportScope {
    pub(super) fn capture(
        attempt: ExportAttempt,
        approval: OperatorApproval,
        control: Arc<MachineControl>,
        catalog: Arc<PluginCatalog>,
        fences: PluginLifecycleFences,
    ) -> Result<Self> {
        let authority = approval.bind_telemetry_export(&attempt)?;
        let connection = control
            .operation_connection(&attempt.machine_id)
            .map_err(|_| anyhow::anyhow!("managed export Machine is not connected"))?;
        let scope = Self {
            attempt,
            authority,
            connection,
            control,
            catalog,
            fences,
        };
        ensure!(scope.current(), "managed export target is not current");
        Ok(scope)
    }

    fn current(&self) -> bool {
        let valid = !self.authority.remaining().is_zero()
            && self
                .control
                .telemetry_export_target_current(&self.connection, &self.attempt)
            && self
                .attempt
                .binding
                .selection
                .as_ref()
                .is_some_and(|target| {
                    !self
                        .fences
                        .read()
                        .contains_key(&(self.attempt.machine_id.clone(), target.plugin_id.clone()))
                        && self
                            .catalog
                            .resolve_telemetry_backend(
                                &target.plugin_id,
                                &target.plugin_version,
                                &String::from(target.generation_digest.clone()),
                            )
                            .is_ok_and(|release| {
                                release
                                    .operation_for(Some(self.attempt.payload.signal))
                                    .is_some()
                                    && self.control.connected_plugin_inventory().iter().any(
                                        |entry| {
                                            entry.machine_id == self.attempt.machine_id
                                                && release.matches_inventory(&entry.plugin)
                                                && entry.plugin.installation_revision.as_ref()
                                                    == Some(&target.installation_revision)
                                                && entry.plugin.contract_fingerprint
                                                    == String::from(
                                                        target.contract_fingerprint.clone(),
                                                    )
                                        },
                                    )
                            })
                });
        if !valid {
            self.authority.revoke();
        }
        valid
    }

    async fn authorized(&self, store: &Store, auth: ProductRequestAuth<'_>) -> bool {
        let valid = self.current()
            && self.authority.check(auth, &self.attempt).await
            && store
                .telemetry_binding_ledger(&self.attempt.service_id)
                .await
                .is_ok_and(|ledger| {
                    ledger.is_some_and(|ledger| ledger.permits_export(&self.attempt))
                })
            && self.current()
            && self.authority.check(auth, &self.attempt).await;
        if !valid {
            self.authority.revoke();
        }
        valid
    }

    /// Consume even failures. This is one bounded grant, not a retryable
    /// exporter factory. Later revocation stops future grants; already-admitted
    /// HTTP can finish. Missing ACKs cannot justify replay or inverse emission.
    pub(super) async fn execute(
        self,
        store: &Store,
        auth: ProductRequestAuth<'_>,
    ) -> Result<ExportReceipt> {
        let budget = self.authority.remaining();
        let digest = self.attempt.request_digest()?;
        let receipt = ExportReceipt {
            request_digest: digest,
            outcome: ExportOutcome::Unknown {},
        };
        match tokio::time::timeout(budget, async {
            ensure!(
                self.authorized(store, auth).await,
                "managed export authorization ended"
            );
            Ok(self
                .control
                .export_bound_telemetry(&self.connection, &self.attempt)
                .await
                .unwrap_or_else(|failure| ExportReceipt {
                    request_digest: receipt.request_digest.clone(),
                    outcome: if failure.certainty == crate::machine_control::CommandFailure::NotSent
                    {
                        ExportOutcome::NotAdmitted {}
                    } else {
                        ExportOutcome::Unknown {}
                    },
                }))
        })
        .await
        {
            Ok(result) => result,
            // A timeout might follow enqueue. Do not report NotAdmitted.
            Err(_) => Ok(receipt),
        }
    }
}

//! Single-attempt managed egress. A fresh Operator confirmation and an explicit
//! host background policy are distinct authorities, never binding receipts.
#![cfg_attr(not(feature = "machine-host"), allow(dead_code))]

use crate::machine_control::{ConnectionToken, MachineControl};
use crate::machine_protocol::telemetry_export::{ExportAttempt, ExportOutcome, ExportReceipt};
use crate::plugin_catalog::PluginCatalog;
use crate::server::operator_approval::{OperatorApproval, TelemetryExportAuthority};
use crate::server::{PluginLifecycleFences, ProductRequestAuth};
use crate::store::Store;
use crate::telemetry_plugin::background_policy::BackgroundPermit;
use anyhow::{Result, ensure};
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;

pub(super) trait Authority: Sync {
    fn remaining(&self) -> Duration;
    fn revoke(&self);
    fn reject_binding(&self);
}

impl Authority for TelemetryExportAuthority {
    fn remaining(&self) -> Duration {
        self.remaining()
    }
    fn revoke(&self) {
        self.revoke();
    }
    fn reject_binding(&self) {
        self.revoke();
    }
}

impl Authority for BackgroundPermit {
    fn remaining(&self) -> Duration {
        self.remaining()
    }
    fn revoke(&self) {
        self.revoke();
    }
    fn reject_binding(&self) {
        self.reject_binding();
    }
}

pub(super) struct ExportScope<A = TelemetryExportAuthority> {
    attempt: ExportAttempt,
    authority: A,
    connection: ConnectionToken,
    control: Arc<MachineControl>,
    catalog: Arc<PluginCatalog>,
    fences: PluginLifecycleFences,
}

impl ExportScope<TelemetryExportAuthority> {
    pub(super) fn capture(
        attempt: ExportAttempt,
        approval: OperatorApproval,
        control: Arc<MachineControl>,
        catalog: Arc<PluginCatalog>,
        fences: PluginLifecycleFences,
    ) -> Result<Self> {
        let authority = approval.bind_telemetry_export(&attempt)?;
        Self::bind(attempt, authority, control, catalog, fences)
    }

    pub(super) async fn execute(
        self,
        store: &Store,
        auth: ProductRequestAuth<'_>,
    ) -> Result<ExportReceipt> {
        self.execute_with(store, || {
            Box::pin(self.authority.check(auth, &self.attempt))
        })
        .await
    }
}

impl ExportScope<BackgroundPermit> {
    pub(super) fn background(
        attempt: ExportAttempt,
        permit: BackgroundPermit,
        control: Arc<MachineControl>,
        catalog: Arc<PluginCatalog>,
        fences: PluginLifecycleFences,
    ) -> Result<Self> {
        ensure!(permit.check(&attempt), "background export permit changed");
        Self::bind(attempt, permit, control, catalog, fences)
    }

    pub(super) async fn execute_background(self, store: &Store) -> Result<ExportReceipt> {
        self.execute_with(store, || {
            Box::pin(async { self.authority.check(&self.attempt) })
        })
        .await
    }
}

impl<A: Authority> ExportScope<A> {
    fn bind(
        attempt: ExportAttempt,
        authority: A,
        control: Arc<MachineControl>,
        catalog: Arc<PluginCatalog>,
        fences: PluginLifecycleFences,
    ) -> Result<Self> {
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

    async fn authorized<'a>(
        &self,
        store: &Store,
        check: &(impl Fn() -> BoxFuture<'a, bool> + Sync),
    ) -> bool {
        if !self.current() || !check().await {
            self.authority.revoke();
            return false;
        }
        let binding_current = store
            .telemetry_binding_ledger(&self.attempt.service_id)
            .await
            .is_ok_and(|ledger| ledger.is_some_and(|ledger| ledger.permits_export(&self.attempt)));
        if !binding_current {
            self.authority.reject_binding();
            return false;
        }
        let valid = self.current() && check().await;
        if !valid {
            self.authority.revoke();
        }
        valid
    }

    /// Consume even failures. This is one bounded grant, not a retryable
    /// exporter factory. Later revocation stops future grants; already-admitted
    /// HTTP can finish. Missing ACKs cannot justify replay or inverse emission.
    async fn execute_with<'a>(
        &self,
        store: &Store,
        check: impl Fn() -> BoxFuture<'a, bool> + Send + Sync,
    ) -> Result<ExportReceipt> {
        let budget = self.authority.remaining();
        let digest = self.attempt.request_digest()?;
        let receipt = ExportReceipt {
            request_digest: digest,
            outcome: ExportOutcome::Unknown {},
        };
        match tokio::time::timeout(budget, async {
            ensure!(
                self.authorized(store, &check).await,
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

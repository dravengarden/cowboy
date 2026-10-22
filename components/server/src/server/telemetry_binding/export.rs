//! Single-attempt managed egress. A fresh Operator confirmation and an explicit
//! host background policy are distinct authorities, never binding receipts.
#![cfg_attr(not(feature = "machine-host"), allow(dead_code))]

use crate::composition::telemetry::ResolvedExport;
use crate::machine_control::MachineControl;
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
    resolved: ResolvedExport,
    authority: A,
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
            Box::pin(self.authority.check(auth, self.resolved.attempt()))
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
            Box::pin(async { self.authority.check(self.resolved.attempt()) })
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
        let resolved = ResolvedExport::resolve(&catalog, &control, attempt)?;
        let scope = Self {
            resolved,
            authority,
            control,
            catalog,
            fences,
        };
        ensure!(scope.current(), "managed export target is not current");
        Ok(scope)
    }

    fn current(&self) -> bool {
        let valid = !self.authority.remaining().is_zero()
            && self.resolved.current(&self.catalog, &self.control)
            && !self.fences.read().contains_key(&(
                self.resolved.attempt().machine_id.clone(),
                self.resolved.installation().plugin_id.clone(),
            ));
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
            .telemetry_binding_ledger(&self.resolved.attempt().service_id)
            .await
            .is_ok_and(|ledger| {
                ledger.is_some_and(|ledger| ledger.permits_export(self.resolved.attempt()))
            });
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
        let digest = self.resolved.attempt().request_digest()?;
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
                .resolved
                .dispatch(&self.catalog, &self.control)
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

//! Shared core HTTP dependencies. Plans and authorities retain distinct purposes.
use super::*;
use crate::machine_control::MachineControl;
use crate::server::{
    AppState, AuthenticatedProductRequest, ProductRequestAuth, operator_approval::OperatorApproval,
};
use crate::telemetry_binding::Ledger;
use crate::telemetry_plugin::writer_admission::{Purpose, WriteScope, WriterAdmission};
use axum::{
    Json,
    extract::FromRef,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

// Only the dependencies needed by this core surface, not Plugin-controlled
// state or an alternative authentication registry. Clone holds no authority.
#[derive(Clone)]
pub struct ApiState {
    pub service: String,
    pub store: Option<Store>,
    pub control: Arc<MachineControl>,
    pub plans: Arc<super::resolution::surface::Plans>,
    pub recovery_plans: Arc<super::recovery::surface::Plans>,
    pub binding_plans: Arc<super::surface::Plans>,
    pub catalog: Arc<crate::plugin_catalog::PluginCatalog>,
    pub fences: crate::server::PluginLifecycleFences,
    pub legacy_fence: LegacyFence,
    pub admission: Option<Arc<WriterAdmission>>,
    pub hub: crate::core::Hub,
    pub product_auth_enabled: bool,
    pub devices: Arc<crate::client_auth::DeviceAccessSessions>,
    pub authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    #[cfg(any(test, feature = "test-support"))]
    pub fixture_write_admission: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub fixture_recovery_admission: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub fixture_binding_admission: bool,
}

impl FromRef<Arc<AppState>> for ApiState {
    fn from_ref(state: &Arc<AppState>) -> Self {
        Self {
            service: state.service_id.clone(),
            store: state.store.clone(),
            control: state.machine_control.clone(),
            plans: state.telemetry_resolution_plans.clone(),
            recovery_plans: state.telemetry_recovery_plans.clone(),
            binding_plans: state.telemetry_binding_plans.clone(),
            catalog: state.plugin_catalog.clone(),
            fences: state.plugin_lifecycle_fences.clone(),
            legacy_fence: state.telemetry_binding_fence.clone(),
            admission: state.telemetry_writer_admission.clone(),
            hub: state.hub.clone(),
            product_auth_enabled: state.product_auth_enabled,
            devices: state.device_access.clone(),
            authentication: state.product_authentication.clone(),
            #[cfg(any(test, feature = "test-support"))]
            fixture_write_admission: false,
            #[cfg(any(test, feature = "test-support"))]
            fixture_recovery_admission: false,
            #[cfg(any(test, feature = "test-support"))]
            fixture_binding_admission: false,
        }
    }
}

impl ApiState {
    pub fn write_admitted<P: Purpose>(&self, machine: Option<&str>) -> bool {
        #[cfg(any(test, feature = "test-support"))]
        if [
            self.fixture_binding_admission,
            self.fixture_recovery_admission,
            self.fixture_write_admission,
        ][P::INDEX]
        {
            return true;
        }
        self.admission
            .as_ref()
            .is_some_and(|policy| policy.allows::<P>(&self.service, machine))
    }

    pub fn write_scope<P: Purpose>(&self, machine: &str) -> Option<WriteScope<P>> {
        #[cfg(any(test, feature = "test-support"))]
        if [
            self.fixture_binding_admission,
            self.fixture_recovery_admission,
            self.fixture_write_admission,
        ][P::INDEX]
        {
            return Some(WriteScope::fixture());
        }
        self.admission.as_ref()?.scope::<P>(&self.service, machine)
    }

    pub fn auth(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: self.product_auth_enabled,
            store: self.store.as_ref(),
            hub: &self.hub,
            device_access: &self.devices,
            product_authentication: &self.authentication,
        }
    }

    pub fn store(&self) -> Result<&Store, ApiError> {
        self.store.as_ref().ok_or(ApiError::EvidenceUnavailable)
    }

    pub async fn ledger(&self) -> Result<Option<Ledger>, ApiError> {
        self.store()?
            .telemetry_binding_ledger(&self.service)
            .await
            .map_err(|_| ApiError::EvidenceUnavailable)
    }

    pub async fn approve(
        &self,
        verified: Option<&AuthenticatedProductRequest>,
        headers: &HeaderMap,
    ) -> Result<OperatorApproval, ApiError> {
        let approval = OperatorApproval::capture(self.auth(), &self.service, verified, headers)
            .map_err(ApiError::Authorization)?;
        if approval.current_operator(self.auth()).await.as_ref() != Some(approval.actor()) {
            return Err(ApiError::Authorization(StatusCode::UNAUTHORIZED));
        }
        Ok(approval)
    }
}

#[derive(Debug)]
pub enum ApiError {
    Authorization(StatusCode),
    EvidenceUnavailable,
    NotFound,
    Changed,
    AdmissionClosed,
    OutcomeUnverified,
    RecoveryAdmissionClosed,
    BindingAdmissionClosed,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Authorization(status) => (status, "authorization_required"),
            Self::EvidenceUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "evidence_unavailable"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Changed => (StatusCode::CONFLICT, "preview_or_evidence_changed"),
            Self::AdmissionClosed => (StatusCode::CONFLICT, "resolution_admission_closed"),
            Self::RecoveryAdmissionClosed => (StatusCode::CONFLICT, "recovery_admission_closed"),
            Self::BindingAdmissionClosed => (StatusCode::CONFLICT, "binding_admission_closed"),
            Self::OutcomeUnverified => (StatusCode::CONFLICT, "outcome_unverified"),
        };
        (
            status,
            Json(serde_json::json!({"schema": 1, "error": code})),
        )
            .into_response()
    }
}

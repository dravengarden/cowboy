//! Shared core HTTP dependencies. Plans and authorities retain distinct purposes.
use super::*;
use crate::machine_control::MachineControl;
use crate::server::{
    AppState, AuthenticatedProductRequest, ProductRequestAuth, operator_approval::OperatorApproval,
};
use crate::telemetry_binding::Ledger;
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
pub(in crate::server) struct ApiState {
    pub(super) service: String,
    pub(super) store: Option<Store>,
    pub(super) control: Arc<MachineControl>,
    pub(super) plans: Arc<super::resolution::surface::Plans>,
    pub(super) recovery_plans: Arc<super::recovery::surface::Plans>,
    pub(super) binding_plans: Arc<super::surface::Plans>,
    pub(super) catalog: Arc<crate::plugin_catalog::PluginCatalog>,
    pub(super) fences: crate::server::PluginLifecycleFences,
    pub(super) legacy_fence: LegacyFence,
    pub(super) hub: crate::core::Hub,
    pub(super) product_auth_enabled: bool,
    pub(super) devices: Arc<crate::client_auth::DeviceAccessSessions>,
    pub(super) authentication: Arc<crate::auth_plugins::ProductAuthentication>,
    #[cfg(test)]
    pub(super) fixture_write_admission: bool,
    #[cfg(test)]
    pub(super) fixture_recovery_admission: bool,
    #[cfg(test)]
    pub(super) fixture_binding_admission: bool,
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
            hub: state.hub.clone(),
            product_auth_enabled: state.product_auth_enabled,
            devices: state.device_access.clone(),
            authentication: state.product_authentication.clone(),
            #[cfg(test)]
            fixture_write_admission: false,
            #[cfg(test)]
            fixture_recovery_admission: false,
            #[cfg(test)]
            fixture_binding_admission: false,
        }
    }
}

impl ApiState {
    pub(super) fn auth(&self) -> ProductRequestAuth<'_> {
        ProductRequestAuth {
            product_auth_enabled: self.product_auth_enabled,
            store: self.store.as_ref(),
            hub: &self.hub,
            device_access: &self.devices,
            product_authentication: &self.authentication,
        }
    }

    pub(super) fn store(&self) -> Result<&Store, ApiError> {
        self.store.as_ref().ok_or(ApiError::EvidenceUnavailable)
    }

    pub(super) async fn ledger(&self) -> Result<Option<Ledger>, ApiError> {
        self.store()?
            .telemetry_binding_ledger(&self.service)
            .await
            .map_err(|_| ApiError::EvidenceUnavailable)
    }

    pub(super) async fn approve(
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
pub(super) enum ApiError {
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

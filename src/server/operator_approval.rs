//! Core continuation of one freshly authenticated confirmation. Durable Actor
//! data cannot construct this type. No credential secret, proof nonce, or grant
//! is copied into a journal, Machine command, or diagnostic response.

use super::*;
use crate::admin::{AdminRole, hex_sha256};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::plugin_operation::{Actor, UninstallIntent};
use anyhow::{Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

enum Credential {
    Product(super::product_continuation::ProductContinuation),
    #[cfg(unix)]
    Host(Arc<crate::local_operator::Grant>),
    Admin {
        token_hash: String,
    },
}

// Intentionally no Clone, Debug, Serialize, or Deserialize on either authority.
pub(super) struct OperatorApproval {
    service: String,
    actor: Actor,
    credential: Credential,
    received: TimeSample,
}

impl OperatorApproval {
    #[cfg(unix)]
    pub(super) fn capture_host(
        service: &str,
        grant: Arc<crate::local_operator::Grant>,
    ) -> Result<Self, StatusCode> {
        if !grant.current() {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(Self {
            service: service.to_owned(),
            actor: grant.actor(),
            credential: Credential::Host(grant),
            received: TimeSample::now(),
        })
    }

    pub(super) fn capture(
        auth: ProductRequestAuth<'_>,
        service: &str,
        authenticated: Option<&AuthenticatedProductRequest>,
        headers: &HeaderMap,
    ) -> Result<Self, StatusCode> {
        let received = TimeSample::now();
        // Preserve middleware precedence once; later checks never switch to a
        // second cookie, a new login, another actor, or local-auth fallback.
        let (actor, credential) =
            if let Ok(principal) = require_admin_role(auth.hub, headers, AdminRole::Operator) {
                let token = crate::admin::cookie_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;
                (
                    Actor::Admin {
                        account: principal.account,
                    },
                    Credential::Admin {
                        token_hash: hex_sha256(token.as_bytes()),
                    },
                )
            } else {
                let mut approval = Self::capture_product(auth, service, authenticated, headers)?;
                approval.received = received;
                return Ok(approval);
            };
        Ok(Self {
            service: service.to_owned(),
            actor,
            credential,
            received,
        })
    }

    /// Product-only continuations must not inherit the separate admin-cookie
    /// precedence used by Plugin administration.
    pub(super) fn capture_product(
        auth: ProductRequestAuth<'_>,
        service: &str,
        authenticated: Option<&AuthenticatedProductRequest>,
        headers: &HeaderMap,
    ) -> Result<Self, StatusCode> {
        let received = TimeSample::now();
        let verified = authenticated.ok_or(StatusCode::UNAUTHORIZED)?;
        if !verified.principal.role.at_least(AdminRole::Operator) {
            return Err(StatusCode::FORBIDDEN);
        }
        // Automation read scopes must never become Plugin mutation authority.
        if verified
            .device_identity
            .as_ref()
            .is_some_and(|identity| identity.is_automation())
        {
            return Err(StatusCode::FORBIDDEN);
        }
        let credential =
            Credential::Product(super::product_continuation::ProductContinuation::capture(
                auth,
                authenticated,
                headers,
            )?);
        Ok(Self {
            service: service.to_owned(),
            actor: Actor::Product {
                user_id: verified.principal.user_id.clone(),
            },
            credential,
            received,
        })
    }

    pub(super) fn actor(&self) -> &Actor {
        &self.actor
    }

    /// A fresh confirmation of one CLOSED local recovery action, not renewal of
    /// the original uninstall or a capability to reactivate anything. Consuming
    /// self also consumes failures: later login/role repair cannot revive it.
    pub(super) async fn authorize_resolution(
        self,
        auth: ProductRequestAuth<'_>,
        service: &str,
        intent: crate::plugin_operation::resolution::ResolutionIntent,
    ) -> Result<crate::plugin_operation::resolution::ResolutionPermit> {
        use crate::plugin_operation::resolution::ResolutionPermit;
        intent.validate()?;
        let budget =
            OperationBudget::new(intent.expires_at_ms, Duration::from_mins(1), self.received);
        ensure!(
            self.service == service && intent.service_id == service && self.actor == intent.actor,
            "resolution confirmation owner changed"
        );
        ensure!(
            !budget.expired()
                && self.current_operator(auth).await.as_ref() == Some(&self.actor)
                && !budget.expired(),
            "resolution confirmation is no longer authorized"
        );
        Ok(ResolutionPermit::new(intent, budget))
    }

    pub(super) fn bind(self, intent: &UninstallIntent) -> Result<UninstallAuthority> {
        ensure!(
            self.actor == intent.actor && self.service == intent.service_id,
            "Plugin confirmation owner changed"
        );
        Ok(UninstallAuthority {
            request_digest: intent.machine_step()?.request_digest()?,
            budget: OperationBudget::new(
                intent.expires_at_ms,
                Duration::from_mins(5),
                self.received,
            ),
            approval: self,
            revoked: AtomicBool::new(false),
        })
    }

    // A separate closed operation kind; an uninstall/resolution grant cannot
    // be converted into a telemetry binding or used to restore old credentials.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn bind_telemetry(
        self,
        intent: &crate::telemetry_binding::Intent,
    ) -> Result<TelemetryBindingAuthority> {
        ensure!(
            self.actor == intent.actor && self.service == intent.service_id,
            "binding confirmation owner changed"
        );
        Ok(TelemetryBindingAuthority {
            request_digest: intent.machine_step()?.request_digest()?,
            budget: OperationBudget::new(
                intent.expires_at_ms,
                Duration::from_mins(1),
                self.received,
            ),
            approval: self,
            revoked: AtomicBool::new(false),
        })
    }

    pub(super) async fn current_operator(&self, auth: ProductRequestAuth<'_>) -> Option<Actor> {
        #[cfg(unix)]
        if let Credential::Host(grant) = &self.credential {
            return grant.current().then(|| grant.actor());
        }
        if let Credential::Admin { token_hash } = &self.credential {
            return admin_identities(auth.hub)
                .principal_by_token_hash(token_hash, auth_now_ms())
                .filter(|p| p.role.at_least(AdminRole::Operator))
                .map(|p| Actor::Admin { account: p.account });
        }
        self.current_product_operator(auth)
            .await
            .map(|principal| Actor::Product {
                user_id: principal.user_id,
            })
    }

    pub(super) async fn current_product_operator(
        &self,
        auth: ProductRequestAuth<'_>,
    ) -> Option<ProductPrincipal> {
        let Credential::Product(continuation) = &self.credential else {
            return None;
        };
        let principal = continuation.current(auth).await?;
        (principal.role.at_least(AdminRole::Operator)
            && self.actor
                == (Actor::Product {
                    user_id: principal.user_id.clone(),
                }))
        .then_some(principal)
    }
}

mod installation;
pub(super) use installation::InstallationAuthority;
mod telemetry_export;
pub(super) use telemetry_export::TelemetryExportAuthority;
mod telemetry_resolution;
pub(super) use telemetry_resolution::TelemetryResolutionAuthority;
mod telemetry_recovery;
pub(super) use telemetry_recovery::TelemetryRecoveryAuthority;

pub(super) struct UninstallAuthority {
    approval: OperatorApproval,
    request_digest: String,
    budget: OperationBudget,
    revoked: AtomicBool,
}

impl UninstallAuthority {
    pub(super) async fn check(
        &self,
        auth: ProductRequestAuth<'_>,
        service: &str,
        intent: &UninstallIntent,
    ) -> bool {
        let valid = self.within_budget()
            && self.approval.service == service
            && intent
                .machine_step()
                .and_then(|step| step.request_digest())
                .is_ok_and(|digest| digest == self.request_digest)
            && self.approval.current_operator(auth).await.as_ref() == Some(&self.approval.actor)
            && self.within_budget();
        if !valid {
            self.revoke();
        }
        valid
    }

    pub(super) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }

    pub(super) fn within_budget(&self) -> bool {
        !self.revoked.load(Ordering::Acquire) && !self.budget.expired()
    }
}

// Reader release: the authority is exercised in hermetic tests, not exposed by
// a production mutation endpoint. Intentionally no Clone/serde/Debug.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct TelemetryBindingAuthority {
    approval: OperatorApproval,
    request_digest: crate::machine_protocol::telemetry_binding::BindingDigest,
    budget: OperationBudget,
    revoked: AtomicBool,
}

#[cfg_attr(not(test), allow(dead_code))]
impl TelemetryBindingAuthority {
    pub(super) fn constrain_to_preview(mut self, preview: OperationBudget) -> Self {
        self.budget = self.budget.intersect(preview);
        self
    }

    pub(super) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    pub(super) async fn check(
        &self,
        auth: ProductRequestAuth<'_>,
        service: &str,
        intent: &crate::telemetry_binding::Intent,
    ) -> bool {
        let valid = self.within_budget()
            && self.approval.service == service
            && intent
                .machine_step()
                .and_then(|s| s.request_digest())
                .is_ok_and(|d| d == self.request_digest)
            && self.approval.current_operator(auth).await.as_ref() == Some(&self.approval.actor)
            && self.within_budget();
        if !valid {
            self.revoke();
        }
        valid
    }

    pub(super) fn within_budget(&self) -> bool {
        !self.revoked.load(Ordering::Acquire) && !self.budget.expired()
    }
}

#[cfg(test)]
mod tests;

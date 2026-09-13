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
    Local,
    Admin {
        token_hash: String,
    },
    Cookie {
        token_hash: String,
        session_id: String,
    },
    PersonalToken {
        token_hash: String,
    },
    Device {
        token_hash: String,
        identity: crate::client_auth::DeviceAccessIdentity,
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
    pub(super) fn capture(
        auth: ProductRequestAuth<'_>,
        service: &str,
        authenticated: Option<&AuthenticatedProductRequest>,
        headers: &HeaderMap,
    ) -> Result<Self, StatusCode> {
        let received = TimeSample::now();
        // Preserve middleware precedence once; later checks never switch to a
        // second cookie, a new login, another actor, or local-auth fallback.
        let (actor, credential) = if let Ok(principal) =
            require_admin_role(auth.hub, headers, AdminRole::Operator)
        {
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
            let verified = authenticated.ok_or(StatusCode::UNAUTHORIZED)?;
            if !verified.principal.role.at_least(AdminRole::Operator) {
                return Err(StatusCode::FORBIDDEN);
            }
            let credential = if !auth.product_auth_enabled {
                if verified.principal != crate::product_auth::local_product_principal() {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Credential::Local
            } else if let Some(token) = crate::product_auth::bearer_token(headers) {
                let token_hash = hex_sha256(token.as_bytes());
                if let Some(identity) = &verified.device_identity {
                    // Existing automation scopes do not authorize Plugin writes.
                    if identity.is_automation() || identity.user_id != verified.principal.user_id {
                        return Err(StatusCode::FORBIDDEN);
                    }
                    Credential::Device {
                        token_hash,
                        identity: identity.clone(),
                    }
                } else {
                    if verified.cookie_session.is_some() {
                        return Err(StatusCode::UNAUTHORIZED);
                    }
                    Credential::PersonalToken { token_hash }
                }
            } else {
                let session = verified
                    .cookie_session
                    .as_ref()
                    .ok_or(StatusCode::UNAUTHORIZED)?;
                let token = crate::product_auth::user_cookie_token(headers)
                    .ok_or(StatusCode::UNAUTHORIZED)?;
                if session.token_hash != hex_sha256(token.as_bytes())
                    || session.user_id != verified.principal.user_id
                    || verified.device_identity.is_some()
                {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Credential::Cookie {
                    token_hash: session.token_hash.clone(),
                    session_id: session.session_id.clone(),
                }
            };
            (
                Actor::Product {
                    user_id: verified.principal.user_id.clone(),
                },
                credential,
            )
        };
        Ok(Self {
            service: service.to_owned(),
            actor,
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
        match &self.credential {
            Credential::Admin { token_hash } => {
                return admin_identities(auth.hub)
                    .principal_by_token_hash(token_hash, auth_now_ms())
                    .filter(|p| p.role.at_least(AdminRole::Operator))
                    .map(|p| Actor::Admin { account: p.account });
            }
            Credential::Local => {
                return (!auth.product_auth_enabled).then(|| Actor::Product {
                    user_id: crate::product_auth::local_product_principal().user_id,
                });
            }
            _ => {}
        }
        if !auth.product_auth_enabled {
            return None;
        }
        let store = auth.store?;
        let Actor::Product {
            user_id: expected_user,
        } = &self.actor
        else {
            return None;
        };
        let user = store.user_by_id(expected_user).await.ok()??;
        if user.disabled_at_ms.is_some() {
            return None;
        }
        // Keep the credential check after user lookup. In particular, a device
        // revoke while the DB is waiting must be seen before local dispatch.
        let user_id = match &self.credential {
            Credential::Cookie {
                token_hash,
                session_id,
            } => {
                let session = store.user_session_by_token_hash(token_hash).await.ok()??;
                if &session.session_id != session_id
                    || session.revoked_at_ms.is_some()
                    || session.expires_at_ms <= auth_now_ms()
                    || ensure_product_session_fresh(store, &session, auth.product_authentication)
                        .await
                        .is_err()
                    || session.expires_at_ms <= auth_now_ms()
                {
                    return None;
                }
                session.user_id
            }
            Credential::PersonalToken { token_hash } => {
                let token = store.user_api_token_by_hash(token_hash).await.ok()??;
                if token.revoked_at_ms.is_some()
                    || token
                        .expires_at_ms
                        .is_some_and(|expires| expires <= auth_now_ms())
                {
                    return None;
                }
                token.user_id
            }
            Credential::Device {
                token_hash,
                identity,
            } => {
                // Recheck the original access token, not a replayed DPoP proof.
                if !auth
                    .device_access
                    .token_hash_still_valid(token_hash, identity, auth_now_ms())
                {
                    return None;
                }
                identity.user_id.clone()
            }
            Credential::Local | Credential::Admin { .. } => return None,
        };
        let principal = product_principal(auth.hub, &user);
        (user_id == user.id && principal.role.at_least(AdminRole::Operator)).then_some(
            Actor::Product {
                user_id: principal.user_id,
            },
        )
    }
}

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

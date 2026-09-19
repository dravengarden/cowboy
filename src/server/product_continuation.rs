//! Original product credential only. Captured from verified middleware evidence;
//! never re-authenticates a request, consumes another proof, or falls back to a
//! different credential. Operation-specific permissions remain with the caller.

use super::*;
use crate::admin::hex_sha256;

enum Credential {
    Local,
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

// No Clone/Debug/serde: this is request-local evidence, not a cache key or grant.
pub(super) struct ProductContinuation {
    user_id: String,
    credential: Credential,
    permissions: Option<crate::core::ProductPermissionObservation>,
}

impl ProductContinuation {
    pub(super) fn capture(
        auth: ProductRequestAuth<'_>,
        authenticated: Option<&AuthenticatedProductRequest>,
        headers: &HeaderMap,
    ) -> Result<Self, StatusCode> {
        let verified = authenticated.ok_or(StatusCode::UNAUTHORIZED)?;
        let permissions = if auth.product_auth_enabled {
            Some(
                verified
                    .permissions
                    .as_ref()
                    .filter(|scope| {
                        scope.current(auth.hub, &verified.principal.username)
                            && scope.role() == verified.principal.role
                    })
                    .ok_or(StatusCode::UNAUTHORIZED)?
                    .clone(),
            )
        } else {
            None
        };
        let credential = if !auth.product_auth_enabled {
            if verified.principal != crate::product_auth::local_product_principal() {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Credential::Local
        } else if let Some(token) = crate::product_auth::bearer_token(headers) {
            let token_hash = hex_sha256(token.as_bytes());
            if verified.cookie_session.is_some() {
                return Err(StatusCode::UNAUTHORIZED);
            }
            if let Some(identity) = &verified.device_identity {
                if identity.user_id != verified.principal.user_id {
                    return Err(StatusCode::UNAUTHORIZED);
                }
                Credential::Device {
                    token_hash,
                    identity: identity.clone(),
                }
            } else {
                Credential::PersonalToken { token_hash }
            }
        } else {
            let session = verified
                .cookie_session
                .as_ref()
                .ok_or(StatusCode::UNAUTHORIZED)?;
            let token =
                crate::product_auth::user_cookie_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;
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
        Ok(Self {
            user_id: verified.principal.user_id.clone(),
            credential,
            permissions,
        })
    }

    pub(super) async fn current(&self, auth: ProductRequestAuth<'_>) -> Option<ProductPrincipal> {
        if matches!(self.credential, Credential::Local) {
            return (!auth.product_auth_enabled).then(crate::product_auth::local_product_principal);
        }
        if !auth.product_auth_enabled {
            return None;
        }
        let permissions = self.permissions.as_ref()?;
        let store = auth.store?;
        let user = store.user_by_id(&self.user_id).await.ok()??;
        if user.disabled_at_ms.is_some() || !permissions.current(auth.hub, &user.username) {
            return None;
        }
        // Credential validation follows the user lookup: a device revocation
        // during that DB wait must be observed before returning authority.
        let user_id = match &self.credential {
            Credential::Local => return None,
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
                if !auth
                    .device_access
                    .token_hash_still_valid(token_hash, identity, auth_now_ms())
                    || (identity.is_automation() && !auth.product_authentication.automation.enabled)
                {
                    return None;
                }
                identity.user_id.clone()
            }
        };
        (user_id == self.user_id
            && user.id == self.user_id
            && permissions.current(auth.hub, &user.username))
        .then(|| ProductPrincipal {
            user_id: user.id,
            username: user.username,
            role: permissions.role(),
        })
    }
}

#[cfg(test)]
pub(super) mod tests;

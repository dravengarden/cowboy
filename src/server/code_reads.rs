//! Controller-owned observation around a complete buffered code-reader response.
//!
//! Keep cache hits, conditional responses and errors inside the same boundary as
//! local/remote I/O. Original product credentials and current Session visibility
//! are rechecked too. This is not a fence or compensation for dispatched work.

use std::future::Future;

pub(super) mod file_pages;
pub(super) mod session;
pub(super) mod workspace;

use super::{
    AppState, AuthenticatedProductRequest, CodeReadScope, IntoResponse as _, ProductRequestAuth,
    ResolvedCodeContext, Response, StatusCode, code_context_is_current, header,
    product_continuation::ProductContinuation, resolve_code_context,
};
use std::sync::Arc;

/// Consumed by one buffered read. The owner is the original core HTTP state;
/// neither a route name, a cache hit nor a serialized principal can construct it.
pub(super) struct Authority {
    owner: Arc<AppState>,
    product: ProductContinuation,
}

impl axum::extract::FromRequestParts<Arc<AppState>> for Authority {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        if !matches!(
            parts.method,
            axum::http::Method::GET | axum::http::Method::HEAD
        ) {
            return Err(Denial::Credential.into_response());
        }
        let product = ProductContinuation::capture(
            ProductRequestAuth::from(state.as_ref()),
            parts.extensions.get::<AuthenticatedProductRequest>(),
            &parts.headers,
        )
        .map_err(|_| Denial::Credential.into_response())?;
        Ok(Self {
            owner: Arc::clone(state),
            product,
        })
    }
}

enum Denial {
    Credential,
    Visibility,
    Context,
}

impl Denial {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Credential => (StatusCode::UNAUTHORIZED, "authentication required"),
            Self::Visibility => (StatusCode::NOT_FOUND, "unknown code context"),
            Self::Context => return stale_response(),
        };
        (status, [(header::CACHE_CONTROL, "no-store")], message).into_response()
    }
}

async fn authorized(
    auth: ProductRequestAuth<'_>,
    product: &ProductContinuation,
    owner_user_id: Option<&str>,
) -> Result<(), Denial> {
    let principal = product.current(auth).await.ok_or(Denial::Credential)?;
    if !principal.can_see(owner_user_id) {
        return Err(Denial::Visibility);
    }
    Ok(())
}

impl Authority {
    async fn current(&self, id: &str, scope: &CodeReadScope) -> Result<(), Denial> {
        if !code_context_is_current(&self.owner, id, scope).await {
            return Err(Denial::Context);
        }
        let owner = match scope {
            CodeReadScope::Session(scope) => scope.session().owner_user_id(),
            // Preserve the existing authenticated shared-workspace read policy.
            CodeReadScope::Workspace(_) => None,
        };
        authorized(
            ProductRequestAuth::from(self.owner.as_ref()),
            &self.product,
            owner,
        )
        .await?;
        // No await after credential/role validation: recheck the original live
        // route synchronously, including changes during the credential lookup.
        let current = match scope {
            CodeReadScope::Session(scope) => {
                session::current(&self.owner.hub, &self.owner.machine_control, scope)
            }
            CodeReadScope::Workspace(scope) => {
                self.owner.machine_control.workspace_scope_is_current(scope)
            }
        };
        if !current {
            return Err(Denial::Context);
        }
        Ok(())
    }
}

pub(super) async fn scoped<F: Future<Output = Response>>(
    authority: Authority,
    id: &str,
    read: impl FnOnce(ResolvedCodeContext) -> F,
) -> Response {
    let Some(context) = resolve_code_context(&authority.owner, id).await else {
        return (StatusCode::NOT_FOUND, "unknown code context").into_response();
    };
    let scope = context.scope.clone();
    guarded_response(|| authority.current(id, &scope), || read(context)).await
}

async fn guarded_response<C: Future<Output = Result<(), Denial>>, R: Future<Output = Response>>(
    current: impl Fn() -> C,
    read: impl FnOnce() -> R,
) -> Response {
    if let Err(denial) = current().await {
        return denial.into_response();
    }
    let response = read().await;
    if let Err(denial) = current().await {
        // Replace the entire response, including stale bytes, ETag and 304.
        return denial.into_response();
    }
    response
}

fn stale_response() -> Response {
    (
        StatusCode::GONE,
        [(header::CACHE_CONTROL, "no-store")],
        "code context changed",
    )
        .into_response()
}

#[cfg(test)]
mod tests;

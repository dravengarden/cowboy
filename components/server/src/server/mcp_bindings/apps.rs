//! Signed MCP Apps views (docs/widget-apps-mcp.md §5, §6): single-use frame
//! tokens and server-enforced frame leases.
//!
//! A view runs only while the owner's grant, the policy epoch and the exact
//! generation's signed contract still agree. The Web asks for a frame with an
//! authenticated request and gets a random token, consumed by the first
//! `GET /mcp-app/<token>.html`, and a lease it must renew. Authority checks
//! (step 3) require a live lease; the parent page's timers are never trusted.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::json;

use super::{
    super::AppState, super::AuthenticatedProductRequest, super::session_is_visible, Stored,
};
use crate::mcp_binding::AppCapabilities;

pub(super) const LEASE: Duration = Duration::from_secs(30);
const TOKEN_TTL: Duration = Duration::from_secs(30);
const MAX_FRAMES: usize = 64;
const MAX_FRAMES_PER_SESSION: usize = 8;

/// What a live lease authorizes: one view of one binding under one epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    pub(crate) session: String,
    pub(crate) binding: String,
    pub(crate) uri: String,
    pub(crate) epoch: String,
}

struct Lease {
    scope: Scope,
    expires: Instant,
}

struct Pending {
    lease: String,
    html: Arc<String>,
    issued: Instant,
}

#[derive(Default)]
pub(crate) struct Frames {
    state: Mutex<FrameState>,
}

#[derive(Default)]
struct FrameState {
    leases: HashMap<String, Lease>,
    tokens: HashMap<String, Pending>,
}

fn random_hex() -> String {
    format!(
        "{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    )
}

/// The view and capabilities the current policy grants, checked against the
/// exact generation's signed contract. Any mismatch grants nothing.
pub(crate) struct Granted {
    pub(crate) epoch: String,
    pub(crate) html: Arc<String>,
    pub(crate) capabilities: AppCapabilities,
}

pub(crate) fn current_grant(
    state: &AppState,
    session: &str,
    binding: &str,
    uri: &str,
) -> Result<Granted, &'static str> {
    let stored = Stored::load(&state.hub, session).ok_or("no_policy")?;
    let selected = stored
        .bindings
        .iter()
        .find(|candidate| candidate.name == binding)
        .ok_or("no_binding")?;
    let grant = stored
        .apps
        .get(binding)
        .and_then(|grants| grants.iter().find(|grant| grant.uri == uri))
        .ok_or("no_grant")?;
    let contract = state
        .plugin_catalog
        .resolve_mcp_server(
            &selected.server.plugin_id,
            &selected.server.generation_digest,
        )
        .map_err(|_| "release_unavailable")?;
    contract.validate().map_err(|_| "release_unavailable")?;
    let view = contract
        .apps
        .as_ref()
        .and_then(|apps| apps.ui_resources.iter().find(|view| view.uri == uri))
        .ok_or("undeclared_view")?;
    if !grant
        .capabilities
        .call_tools
        .iter()
        .all(|tool| contract.app_visible(tool))
    {
        return Err("undeclared_tool");
    }
    Ok(Granted {
        epoch: stored.epoch,
        html: Arc::new(view.html.clone()),
        capabilities: grant.capabilities.clone(),
    })
}

impl Frames {
    fn issue(&self, scope: Scope, html: Arc<String>) -> Result<(String, String), &'static str> {
        let now = Instant::now();
        let mut state = self.state.lock();
        state.leases.retain(|_, lease| lease.expires > now);
        state
            .tokens
            .retain(|_, pending| now.duration_since(pending.issued) < TOKEN_TTL);
        let per_session = state
            .leases
            .values()
            .filter(|lease| lease.scope.session == scope.session)
            .count();
        if state.leases.len() >= MAX_FRAMES || per_session >= MAX_FRAMES_PER_SESSION {
            return Err("too_many_frames");
        }
        let lease = random_hex();
        let token = random_hex();
        state.leases.insert(
            lease.clone(),
            Lease {
                scope,
                expires: now + LEASE,
            },
        );
        state.tokens.insert(
            token.clone(),
            Pending {
                lease: lease.clone(),
                html,
                issued: now,
            },
        );
        Ok((token, lease))
    }

    /// Consumes `token` once; the lease it started must still be live.
    fn take(&self, token: &str) -> Option<(Scope, Arc<String>)> {
        let now = Instant::now();
        let mut state = self.state.lock();
        let pending = state.tokens.remove(token)?;
        if now.duration_since(pending.issued) >= TOKEN_TTL {
            return None;
        }
        let lease = state.leases.get(&pending.lease)?;
        (lease.expires > now).then(|| (lease.scope.clone(), pending.html))
    }

    /// The scope of a live lease of `session`, if any.
    pub(crate) fn live(&self, session: &str, lease: &str) -> Option<Scope> {
        let state = self.state.lock();
        state
            .leases
            .get(lease)
            .filter(|entry| entry.expires > Instant::now() && entry.scope.session == session)
            .map(|entry| entry.scope.clone())
    }

    fn renew(&self, lease: &str, scope: &Scope) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock();
        match state.leases.get_mut(lease) {
            Some(entry) if entry.expires > now && entry.scope == *scope => {
                entry.expires = now + LEASE;
                true
            }
            _ => {
                state.leases.remove(lease);
                false
            }
        }
    }

    fn end(&self, session: &str, lease: &str) {
        let mut state = self.state.lock();
        if state
            .leases
            .get(lease)
            .is_some_and(|entry| entry.scope.session == session)
        {
            state.leases.remove(lease);
            state.tokens.retain(|_, pending| pending.lease != lease);
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FrameRequest {
    binding: String,
    uri: String,
}

fn owner_session(
    state: &AppState,
    auth: &AuthenticatedProductRequest,
    session: &str,
) -> Result<(), StatusCode> {
    if !session_is_visible(&state.hub, &auth.principal, session) {
        return Err(StatusCode::NOT_FOUND);
    }
    let info = state
        .hub
        .session_info(session)
        .ok_or(StatusCode::NOT_FOUND)?;
    if auth
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

fn refused(code: &'static str) -> Response {
    let status = match code {
        "too_many_frames" => StatusCode::TOO_MANY_REQUESTS,
        "release_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::CONFLICT,
    };
    (status, Json(json!({"error":code}))).into_response()
}

/// `POST /api/sessions/{id}/mcp-apps/frames`
pub(super) async fn open(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(session): Path<String>,
    Json(request): Json<FrameRequest>,
) -> Response {
    if let Err(status) = owner_session(&state, &auth, &session) {
        return status.into_response();
    }
    let granted = match current_grant(&state, &session, &request.binding, &request.uri) {
        Ok(granted) => granted,
        Err(code) => return refused(code),
    };
    let scope = Scope {
        session,
        binding: request.binding,
        uri: request.uri,
        epoch: granted.epoch,
    };
    match state.extensions.apps.issue(scope, granted.html) {
        Ok((token, lease)) => Json(json!({
            "url": format!("/mcp-app/{token}.html"),
            "lease": lease,
            "lease_ms": LEASE.as_millis(),
            "capabilities": granted.capabilities,
        }))
        .into_response(),
        Err(code) => refused(code),
    }
}

/// `POST /api/sessions/{id}/mcp-apps/frames/{lease}/renew`: the grant, epoch
/// and signed view are rechecked; any change ends the lease.
pub(super) async fn renew(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((session, lease)): Path<(String, String)>,
) -> Response {
    if let Err(status) = owner_session(&state, &auth, &session) {
        return status.into_response();
    }
    let frames = &state.extensions.apps;
    let Some(scope) = frames.live(&session, &lease) else {
        return refused("lease_ended");
    };
    let current = current_grant(&state, &session, &scope.binding, &scope.uri)
        .is_ok_and(|granted| granted.epoch == scope.epoch);
    if current && frames.renew(&lease, &scope) {
        Json(json!({"lease_ms": LEASE.as_millis()})).into_response()
    } else {
        frames.end(&session, &lease);
        refused("lease_ended")
    }
}

/// `DELETE /api/sessions/{id}/mcp-apps/frames/{lease}`
pub(super) async fn close(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((session, lease)): Path<(String, String)>,
) -> Response {
    if let Err(status) = owner_session(&state, &auth, &session) {
        return status.into_response();
    }
    state.extensions.apps.end(&session, &lease);
    StatusCode::NO_CONTENT.into_response()
}

fn valid_token_path(requested: &str) -> Option<&str> {
    let token = requested.strip_prefix("mcp-app/")?.strip_suffix(".html")?;
    (token.len() == 64
        && token
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
    .then_some(token)
}

/// `GET /mcp-app/<token>.html`, exactly, from a same-origin frame. Anything
/// else is an empty 404: an unknown, used or expired token, a query string,
/// another path, a top-level or cross-site load, or a grant that changed since
/// the token was issued.
pub(in crate::server) fn serve(
    state: &AppState,
    requested: &str,
    query: bool,
    headers: &HeaderMap,
) -> Response {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let Some(token) = valid_token_path(requested) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if query
        || header("sec-fetch-dest") != Some("iframe")
        || header("sec-fetch-site") != Some("same-origin")
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some((scope, html)) = state.extensions.apps.take(token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let current = current_grant(state, &scope.session, &scope.binding, &scope.uri)
        .is_ok_and(|granted| granted.epoch == scope.epoch && granted.html == html);
    if !current {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut response = Response::builder().status(StatusCode::OK);
    for (name, value) in super::super::web_security::mcp_app_headers(html.as_bytes()) {
        response = response.header(name, value);
    }
    response
        .body(axum::body::Body::from(html.as_bytes().to_vec()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> Scope {
        Scope {
            session: "s".into(),
            binding: "suger".into(),
            uri: "ui://suger/status".into(),
            epoch: "e".repeat(32),
        }
    }

    #[test]
    fn tokens_are_single_use_and_bound_to_a_live_lease() {
        let frames = Frames::default();
        let html = Arc::new("<p>x</p>".to_owned());
        let (token, lease) = frames.issue(scope(), Arc::clone(&html)).unwrap();
        assert_eq!(frames.live("s", &lease), Some(scope()));
        assert!(frames.live("other", &lease).is_none());
        let (taken, body) = frames.take(&token).unwrap();
        assert_eq!((taken, body), (scope(), html.clone()));
        assert!(frames.take(&token).is_none(), "a token is consumed once");
        // Ending the lease refuses the scope and any token it issued.
        let (token, lease) = frames.issue(scope(), html).unwrap();
        frames.end("s", &lease);
        assert!(frames.live("s", &lease).is_none());
        assert!(frames.take(&token).is_none());
    }

    #[test]
    fn renewal_requires_the_same_scope_and_frames_are_bounded() {
        let frames = Frames::default();
        let html = Arc::new(String::new());
        let (_, lease) = frames.issue(scope(), Arc::clone(&html)).unwrap();
        assert!(frames.renew(&lease, &scope()));
        let changed = Scope {
            epoch: "f".repeat(32),
            ..scope()
        };
        assert!(!frames.renew(&lease, &changed));
        assert!(
            frames.live("s", &lease).is_none(),
            "a failed renewal ends it"
        );
        for _ in 0..MAX_FRAMES_PER_SESSION {
            frames.issue(scope(), Arc::clone(&html)).unwrap();
        }
        assert_eq!(frames.issue(scope(), html).unwrap_err(), "too_many_frames");
    }

    #[test]
    fn only_exact_token_paths_are_accepted() {
        let token = "a".repeat(64);
        assert_eq!(
            valid_token_path(&format!("mcp-app/{token}.html")),
            Some(token.as_str())
        );
        for path in [
            format!("mcp-app/{}.html", "A".repeat(64)),
            format!("mcp-app/{}.html", "a".repeat(63)),
            format!("mcp-app/{token}.htm"),
            format!("mcp-app/{token}.html/x"),
            format!("mcp-app/x/{token}.html"),
            "mcp-app/".to_owned(),
        ] {
            assert!(valid_token_path(&path).is_none(), "{path}");
        }
    }
}

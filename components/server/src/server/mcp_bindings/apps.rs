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
    hub: &crate::core::Hub,
    catalog: &crate::plugin_catalog::PluginCatalog,
    session: &str,
    binding: &str,
    uri: &str,
) -> Result<Granted, &'static str> {
    let stored = Stored::load(hub, session).ok_or("no_policy")?;
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
    let contract = catalog
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
    let granted = match current_grant(
        &state.hub,
        &state.plugin_catalog,
        &session,
        &request.binding,
        &request.uri,
    ) {
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
    let current = current_grant(
        &state.hub,
        &state.plugin_catalog,
        &session,
        &scope.binding,
        &scope.uri,
    )
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
    respond(
        &state.hub,
        &state.plugin_catalog,
        &state.extensions.apps,
        requested,
        query,
        headers,
    )
}

fn respond(
    hub: &crate::core::Hub,
    catalog: &crate::plugin_catalog::PluginCatalog,
    frames: &Frames,
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
    let Some((scope, html)) = frames.take(token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let current = current_grant(hub, catalog, &scope.session, &scope.binding, &scope.uri)
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

    /// A signed MCP server release with one view, published into a disposable
    /// Catalog the way the Controller reads it.
    fn publish_mcp_view(root: &std::path::Path) -> cowboy_plugin_sdk::PluginRelease {
        use cowboy_plugin_sdk::{
            PLUGIN_RELEASE_SIGNATURE_NAMESPACE, PluginPackage, PluginPayload, PluginRelease,
        };
        let manifest = serde_json::from_value(json!({
            "schema_version":1,"id":"view-mcp","version":"1.0.0",
            "component_release":"3.108.0","publisher":"view-publisher",
            "kind":"mcp_server","entrypoint":"contract.json",
            "components":[{"id":"cowboy.plugin-contract","version":"1.13.0"},{"id":"cowboy.plugin-sdk","version":"1.15.0"}]
        }))
        .unwrap();
        let contract = serde_json::from_value(json!({
            "schema_version":1,"id":"view-mcp","version":"1.0.0",
            "display_name":"View MCP","description":"Signed server with a view",
            "supported_platforms":[{"os":"linux","architecture":"x86_64"}],
            "protocol":"stdio_session_v1","command":"view-mcp","arguments":[],
            "tools":["view_status","view_refresh"],
            "apps":{
                "ui_resources":[{"uri":"ui://view/status","html":VIEW}],
                "tools":[
                    {"name":"view_status","resource":"ui://view/status","visibility":["model","app"],"read_only":true},
                    {"name":"view_refresh","visibility":["app"]}
                ]
            }
        }))
        .unwrap();
        let package = PluginPackage::new(
            manifest,
            "3.108.0".into(),
            PluginPayload::McpServer(contract),
        )
        .unwrap();
        let bytes = package.canonical_bytes().unwrap();
        let runtime = b"#!/bin/sh\n";
        let mut release: PluginRelease = serde_json::from_value(json!({
            "release_schema":6,"plugin_id":"view-mcp","plugin_version":"1.0.0",
            "plugin_kind":"mcp_server","package_digest":PluginPackage::artifact_digest(&bytes),
            "artifact_digest":"","artifact_url":"https://plugins.example/view-mcp.cowboy-plugin",
            "publisher":"view-publisher","contract_fingerprint":package.contract_fingerprint,
            "component_release":"3.108.0","signature":"",
            "supported_platforms":[{"os":"linux","architecture":"x86_64"}],
            "runtime_artifacts":[{"os":"linux","architecture":"x86_64","components":[{
                "kind":"mcp_server","slot":"server","dependency":"view-mcp","version":"1.0.0",
                "command":"view-mcp","artifact_url":"https://plugins.example/view-mcp",
                "artifact_digest":PluginPackage::artifact_digest(runtime),
                "artifact_format":"raw","probe":{"args":["--version"],"timeout_ms":1000}
            }]}]
        }))
        .unwrap();
        let identity =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("identity")).unwrap();
        release.artifact_digest = release.computed_artifact_digest().unwrap();
        release.signature = identity
            .sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())
            .unwrap();
        release.validate_bytes(&bytes).unwrap();
        let catalog = root.join("external");
        std::fs::create_dir_all(catalog.join("trusted-publishers")).unwrap();
        std::fs::write(
            catalog.join("trusted-publishers/view-publisher.pub"),
            identity.public_key(),
        )
        .unwrap();
        std::fs::write(catalog.join("view-mcp.cowboy-plugin"), bytes).unwrap();
        std::fs::write(
            catalog.join("view-mcp.release.json"),
            serde_json::to_vec(&release).unwrap(),
        )
        .unwrap();
        release
    }

    const VIEW: &str = "<!doctype html><style>p{margin:0}</style><p>status</p><script>parent.postMessage({jsonrpc:'2.0',id:1,method:'ui/initialize'},'*')</script>";

    fn frame_headers(dest: &str, site: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-dest", dest.parse().unwrap());
        headers.insert("sec-fetch-site", site.parse().unwrap());
        headers
    }

    #[tokio::test]
    async fn a_granted_signed_view_is_served_once_and_only_while_the_grant_holds() {
        let fixture = tempfile::tempdir().unwrap();
        let release = publish_mcp_view(fixture.path());
        let catalog = crate::plugin_catalog::PluginCatalog::inspect(
            fixture.path(),
            Some(fixture.path().join("external")),
        )
        .unwrap();
        assert!(
            catalog
                .resolve_mcp_server("view-mcp", &release.artifact_digest)
                .is_ok()
        );
        assert!(
            catalog
                .resolve_mcp_server("view-mcp", &format!("sha256:{}", "0".repeat(64)))
                .is_err()
        );

        let (hub, meta, policies, store, _root) = super::super::tests::fixture().await;
        let binding: crate::mcp_binding::Binding = serde_json::from_value(json!({
            "name":"view","server":{"plugin_id":"view-mcp","generation_digest":release.artifact_digest},
            "tools":["view_status"]
        }))
        .unwrap();
        let grant = |tools: serde_json::Value| -> std::collections::BTreeMap<String, Vec<crate::mcp_binding::AppGrant>> {
            serde_json::from_value(json!({"view":[{"uri":"ui://view/status","capabilities":{"call_tools":tools}}]})).unwrap()
        };
        let mut update = super::super::tests::update(None, vec![binding.clone()]);
        update.apps = grant(json!(["view_refresh"]));
        let stored = policies.replace(&hub, &store, &meta, update).await.unwrap();

        let granted = current_grant(&hub, &catalog, &meta.id, "view", "ui://view/status").unwrap();
        assert_eq!(granted.html.as_str(), VIEW);
        assert_eq!(granted.epoch, stored.epoch);
        assert!(current_grant(&hub, &catalog, &meta.id, "view", "ui://view/other").is_err());
        assert!(current_grant(&hub, &catalog, &meta.id, "other", "ui://view/status").is_err());

        let frames = Frames::default();
        let scope = Scope {
            session: meta.id.clone(),
            binding: "view".into(),
            uri: "ui://view/status".into(),
            epoch: granted.epoch.clone(),
        };
        let serve = |token: &str, headers: &HeaderMap, query: bool| {
            respond(
                &hub,
                &catalog,
                &frames,
                &format!("mcp-app/{token}.html"),
                query,
                headers,
            )
        };
        let iframe = frame_headers("iframe", "same-origin");
        // Top-level, cross-site and query-string loads never consume the token.
        let (token, _) = frames
            .issue(scope.clone(), Arc::clone(&granted.html))
            .unwrap();
        for headers in [
            frame_headers("document", "same-origin"),
            frame_headers("iframe", "cross-site"),
        ] {
            assert_eq!(
                serve(&token, &headers, false).status(),
                StatusCode::NOT_FOUND
            );
        }
        assert_eq!(serve(&token, &iframe, true).status(), StatusCode::NOT_FOUND);
        let served = serve(&token, &iframe, false);
        assert_eq!(served.status(), StatusCode::OK);
        let csp = served.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(csp.starts_with("sandbox allow-scripts;") && csp.contains("connect-src 'none'"));
        let body = axum::body::to_bytes(served.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), VIEW.as_bytes());
        assert_eq!(
            serve(&token, &iframe, false).status(),
            StatusCode::NOT_FOUND,
            "single use"
        );

        // A token issued under an epoch that has since changed serves nothing.
        let (stale, _) = frames
            .issue(scope.clone(), Arc::clone(&granted.html))
            .unwrap();
        let mut regrant =
            super::super::tests::update(Some(stored.epoch.clone()), vec![binding.clone()]);
        regrant.apps = grant(json!([]));
        let replaced = policies
            .replace(&hub, &store, &meta, regrant)
            .await
            .unwrap();
        assert_eq!(
            serve(&stale, &iframe, false).status(),
            StatusCode::NOT_FOUND
        );

        // A grant naming a tool the signed contract does not offer to views grants nothing.
        let mut model_only = super::super::tests::update(Some(replaced.epoch), vec![binding]);
        model_only.apps = grant(json!(["view_status", "unsigned"]));
        let revoked = policies
            .replace(&hub, &store, &meta, model_only)
            .await
            .unwrap();
        assert_eq!(
            current_grant(&hub, &catalog, &meta.id, "view", "ui://view/status").err(),
            Some("undeclared_tool")
        );
        // Revoking the binding revokes the view.
        policies
            .replace(
                &hub,
                &store,
                &meta,
                super::super::tests::update(Some(revoked.epoch), vec![]),
            )
            .await
            .unwrap();
        assert_eq!(
            current_grant(&hub, &catalog, &meta.id, "view", "ui://view/status").err(),
            Some("no_binding")
        );
    }
}

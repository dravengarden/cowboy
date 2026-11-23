//! View-originated effects (docs/widget-apps-mcp.md §4, §6, §7). Every
//! `tools/call`, `ui/message` and `ui/open-link` names its frame lease; the
//! Controller requires the original, unexpired lease and the owner's current
//! grant at admission and again before it delivers a result, whatever the
//! page does. A view call runs in the view scope, never in the agent's.
use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::apps::{Granted, Scope, owner_session, refused, scope_holds};
use super::relay::view::{self, ViewCall, ViewError};
use super::{super::AppState, super::AuthenticatedProductRequest};

const ARGUMENT_BYTES: usize = 64 * 1024;
const URL_BYTES: usize = 2048;

/// The live lease of `session` and the grant it still holds: its row is open,
/// names the same view and digest, and the owner still grants those exact
/// bytes under the lease's epoch.
pub async fn authorized(
    state: &AppState,
    session: &str,
    lease: &str,
) -> Result<(Scope, Granted), &'static str> {
    let scope = state
        .extensions
        .apps
        .live(session, lease)
        .ok_or("lease_ended")?;
    let granted = scope_holds(state, &scope)
        .await
        .map_err(|_| "lease_ended")?;
    Ok((scope, granted))
}

/// A view's `ui/message` (§4): allowed only under a live lease of that very
/// row's view whose grant includes `message`. The text itself follows the
/// phase A confirmation and durable send of `widget-apps.md` §7.
pub async fn message_allowed(
    state: &AppState,
    session: &str,
    lease: &str,
    widget: &str,
    node: &crate::widgets::interactions::McpAppRef,
) -> Result<(), &'static str> {
    let (scope, granted) = authorized(state, session, lease).await?;
    if scope.widget != widget
        || scope.binding != node.binding
        || scope.uri != node.uri
        || scope.sha256 != node.sha256
    {
        return Err("lease_ended");
    }
    if !granted.capabilities.message {
        return Err("not_granted");
    }
    Ok(())
}

/// What the person confirms for a call that is not read-only: the tool and
/// its exact arguments, bound to this run.
fn confirmation_digest(lease: &str, name: &str, arguments: &Value) -> String {
    let seen = json!([lease, name, arguments]);
    format!("{:x}", Sha256::digest(seen.to_string().as_bytes()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallRequest {
    name: String,
    #[serde(default)]
    arguments: Option<serde_json::Map<String, Value>>,
    /// The digest of the call the person confirmed; required unless the
    /// signed tool is read-only.
    #[serde(default)]
    confirmed: Option<String>,
}

/// `POST /api/sessions/{id}/mcp-apps/frames/{lease}/call`
pub async fn call(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((session, lease)): Path<(String, String)>,
    Json(request): Json<CallRequest>,
) -> Response {
    if let Err(status) = owner_session(&state, &auth, &session) {
        return status.into_response();
    }
    let (scope, granted) = match authorized(&state, &session, &lease).await {
        Ok(authorized) => authorized,
        Err(code) => return refused(code),
    };
    if !granted.capabilities.call_tools.contains(&request.name) {
        return refused("not_granted");
    }
    if !granted.relayable.contains(&request.name) {
        return refused("app_only_tool_unsupported");
    }
    let arguments = Value::Object(request.arguments.unwrap_or_default());
    if serde_json::to_vec(&arguments).map_or(usize::MAX, |bytes| bytes.len()) > ARGUMENT_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error":"arguments_too_large"})),
        )
            .into_response();
    }
    if !granted.read_only.contains(&request.name) {
        let digest = confirmation_digest(&lease, &request.name, &arguments);
        if request.confirmed.as_deref() != Some(digest.as_str()) {
            return Json(json!({"state":"confirm","digest":digest,
                "tool":request.name,"arguments":arguments}))
            .into_response();
        }
    }
    let mut tools = granted.relayable.clone();
    tools.sort();
    let outcome = view::call(
        &state,
        ViewCall {
            session: session.clone(),
            binding: scope.binding,
            epoch: scope.epoch,
            tools,
            name: request.name,
            arguments,
            lease: lease.clone(),
        },
    )
    .await;
    // Delivery needs the same lease again: a result never reaches a frame
    // whose run ended while the call was out.
    if authorized(&state, &session, &lease).await.is_err() {
        return refused("lease_ended");
    }
    match outcome {
        Ok(response) => Json(json!({"state":"ok","response":response})).into_response(),
        Err(ViewError::Refused(code)) => refused(code),
        Err(ViewError::Uncertain(code)) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error":code,"uncertain":true})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkRequest {
    url: String,
}

/// `POST /api/sessions/{id}/mcp-apps/frames/{lease}/open-link`: admits an
/// HTTPS link for Cowboy's opener; the page opens it only after this answers.
pub async fn open_link(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((session, lease)): Path<(String, String)>,
    Json(request): Json<LinkRequest>,
) -> Response {
    if let Err(status) = owner_session(&state, &auth, &session) {
        return status.into_response();
    }
    let granted = match authorized(&state, &session, &lease).await {
        Ok((_, granted)) => granted,
        Err(code) => return refused(code),
    };
    if !granted.capabilities.open_links {
        return refused("not_granted");
    }
    match https_link(&request.url) {
        Some(url) => Json(json!({"state":"ok","url":url})).into_response(),
        None => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"invalid_link"})),
        )
            .into_response(),
    }
}

fn https_link(raw: &str) -> Option<String> {
    if raw.len() > URL_BYTES {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_https_without_credentials() {
        assert_eq!(
            https_link("https://example.com/a?b=1").as_deref(),
            Some("https://example.com/a?b=1")
        );
        for refused in [
            "http://example.com",
            "javascript:alert(1)",
            "https://user:pw@example.com",
            "data:text/html,x",
            "/relative",
        ] {
            assert!(https_link(refused).is_none(), "{refused}");
        }
        assert!(https_link(&format!("https://e.com/{}", "a".repeat(URL_BYTES))).is_none());
    }

    #[test]
    fn a_confirmation_binds_the_run_tool_and_exact_arguments() {
        let one = confirmation_digest("lease-a", "suger_refresh", &json!({"x":1}));
        assert_ne!(
            one,
            confirmation_digest("lease-b", "suger_refresh", &json!({"x":1}))
        );
        assert_ne!(
            one,
            confirmation_digest("lease-a", "suger_other", &json!({"x":1}))
        );
        assert_ne!(
            one,
            confirmation_digest("lease-a", "suger_refresh", &json!({"x":2}))
        );
    }
}

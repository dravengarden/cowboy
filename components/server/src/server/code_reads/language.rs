//! Legacy path queries use the same original product/Session/read-route
//! continuation as buffered filesystem reads. No open/close/reload grant is
//! expressible here. Discarding a reply does not undo native query effects.

use super::{Authority, CodeReadScope, Response, StatusCode, guarded_response};
use crate::server::{
    CodeHoverResponse, CodeLanguageResponse, CodeNavigationKind, CodeNavigationResponse,
    CodeOutlineResponse, Json, ZedAdapterResponse, zed_language_target,
};
use axum::response::IntoResponse as _;
use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "type")]
pub(in crate::server) enum Query {
    #[serde(rename = "bufferLanguage")]
    Language,
    #[serde(rename = "bufferHover")]
    Hover { row: u32, column: u32 },
    #[serde(rename = "bufferNavigate")]
    Navigation {
        row: u32,
        column: u32,
        kind: CodeNavigationKind,
    },
    #[serde(rename = "bufferSymbols")]
    Outline,
}

#[derive(Serialize)]
struct Request<'a> {
    worktree: &'a str,
    path: &'a str,
    #[serde(flatten)]
    query: Query,
}

pub(in crate::server) async fn read(
    authority: Authority,
    id: &str,
    path: &str,
    query: Query,
) -> Response {
    let state = &authority.owner;
    // Deliberately Session-only: Workspace filesystem observations do not
    // establish a native buffer owner or permit implicit acquisition.
    let Some(scope) =
        super::session::resolve(&state.hub, &state.machine_control, &state.service_id, id)
    else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let CodeReadScope::Session(session) = &scope else {
        unreachable!("Session resolver only returns Session observations")
    };
    guarded_response(
        || authority.current(id, &scope),
        || async {
            if path.is_empty() {
                return (StatusCode::BAD_REQUEST, "invalid buffer path").into_response();
            }
            let outline = matches!(query, Query::Outline);
            let requested = path;
            if !state
                .hub
                .language_servers_allowed(session.session().session_id())
            {
                if outline {
                    return structure_outline(state, &scope, session.session().cwd(), path).await;
                }
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "project language servers are off for this session",
                )
                    .into_response();
            }
            let Some((worktree, path)) = zed_language_target(
                session.session().machine_id(),
                session.session().cwd(),
                path,
            ) else {
                if outline {
                    return structure_outline(state, &scope, session.session().cwd(), path).await;
                }
                return (StatusCode::UNPROCESSABLE_ENTITY, "buffer lease unavailable")
                    .into_response();
            };
            let request = serde_json::to_value(Request {
                worktree: &worktree,
                path: &path,
                query,
            })
            .expect("closed language query serialization");
            match super::session::adapter_request(
                &state.hub,
                &state.machine_control,
                state.zed_adapter_socket.as_deref(),
                session,
                request,
            )
            .await
            {
                Ok(response) => project(query, response),
                Err(_) if outline => {
                    structure_outline(state, &scope, session.session().cwd(), requested).await
                }
                Err(error) => {
                    tracing::debug!(session = %id, %error, "Zed language query failed");
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "language intelligence unavailable",
                    )
                        .into_response()
                }
            }
        },
    )
    .await
}

/// The language engine's last observation of an already open buffer, for an
/// agent's own Session. It opens nothing: a path no Code pane holds is refused
/// by the engine. Projected checkouts outside the worktree are refused too.
pub(in crate::server) async fn observe_open_buffer(
    state: &crate::server::AppState,
    session: &crate::machine_control::SessionReadScope,
    path: &str,
) -> anyhow::Result<ZedAdapterResponse> {
    let observed = session.session();
    let (worktree, path) = zed_language_target(observed.machine_id(), observed.cwd(), path)
        .filter(|(worktree, _)| {
            std::path::Path::new(worktree) == std::path::Path::new(observed.cwd())
                || std::path::Path::new(observed.cwd())
                    .canonicalize()
                    .is_ok_and(|cwd| cwd == std::path::Path::new(worktree))
        })
        .ok_or_else(|| anyhow::anyhow!("path is outside this session's worktree"))?;
    let request = serde_json::to_value(Request {
        worktree: &worktree,
        path: &path,
        query: Query::Language,
    })?;
    super::session::adapter_request(
        &state.hub,
        &state.machine_control,
        state.zed_adapter_socket.as_deref(),
        session,
        request,
    )
    .await
}

/// Without a usable language engine the outline comes from syntax. It keeps
/// the response shape and says `precision: "heuristic"`.
async fn structure_outline(
    state: &crate::server::AppState,
    scope: &CodeReadScope,
    cwd: &str,
    path: &str,
) -> Response {
    match crate::server::structure_outline(state, scope, cwd.to_owned(), path.to_owned()).await {
        Ok(symbols) => Json(serde_json::json!({
            "apiVersion": 1,
            "path": path,
            "symbols": symbols,
            "precision": "heuristic",
        }))
        .into_response(),
        Err((status, message)) => (status, message).into_response(),
    }
}

fn project(query: Query, response: ZedAdapterResponse) -> Response {
    match (query, response) {
        (
            Query::Language,
            ZedAdapterResponse::BufferLanguage {
                path,
                version,
                diagnostics,
                inlay_hints,
                semantic_tokens,
                ..
            },
        ) => Json(CodeLanguageResponse {
            api_version: 1,
            path,
            version,
            diagnostics,
            inlay_hints,
            semantic_tokens,
        })
        .into_response(),
        (Query::Hover { .. }, ZedAdapterResponse::BufferHover { path, contents, .. }) => {
            Json(CodeHoverResponse {
                api_version: 1,
                path,
                contents,
            })
            .into_response()
        }
        (
            Query::Navigation { .. },
            ZedAdapterResponse::BufferNavigation {
                path, locations, ..
            },
        ) => Json(CodeNavigationResponse {
            api_version: 1,
            path,
            locations,
        })
        .into_response(),
        (Query::Outline, ZedAdapterResponse::BufferSymbols { path, symbols, .. }) => {
            Json(CodeOutlineResponse {
                api_version: 1,
                path,
                symbols,
            })
            .into_response()
        }
        _ => (
            StatusCode::BAD_GATEWAY,
            "unexpected language service response",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests;

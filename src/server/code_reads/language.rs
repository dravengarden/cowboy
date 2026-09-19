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
            let Some((worktree, path)) = zed_language_target(
                session.session().machine_id(),
                session.session().cwd(),
                path,
            ) else {
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

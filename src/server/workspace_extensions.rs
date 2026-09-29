//! Workspace-scoped extension reads always execute on the original Machine.

use super::*;
use crate::workspace_extensions::{
    Failure, Identity, Operation, Request, Response as ExtensionResponse,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ResourceQuery {
    plugin_id: String,
    plugin_version: String,
    generation_digest: String,
    remote: String,
    view: String,
    item: Option<String>,
    filter: Option<String>,
    page: Option<u32>,
}

async fn read(
    state: &AppState,
    context: ResolvedCodeContext,
    operation: Operation,
) -> ExtensionResponse {
    let value = match &context.scope {
        CodeReadScope::Workspace(scope) => {
            state
                .machine_control
                .workspace_extension_request(scope, operation)
                .await
        }
        CodeReadScope::Session(scope) => {
            let Some(connection) = scope.connection() else {
                return ExtensionResponse::Unavailable {
                    code: Failure::MachineUnavailable,
                };
            };
            let Ok(payload) = serde_json::to_value(Request {
                root: context.cwd,
                operation,
            }) else {
                return ExtensionResponse::Unavailable {
                    code: Failure::InvalidRequest,
                };
            };
            state
                .machine_control
                .adapter_request_on_connection(connection, "workspace-extension", payload)
                .await
        }
    };
    value
        .ok()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or(ExtensionResponse::Unavailable {
            code: Failure::MachineUnavailable,
        })
}

fn response(value: ExtensionResponse) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

pub(super) async fn inventory(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(id): Path<String>,
) -> Response {
    code_reads::scoped(authority, &id, |context| async move {
        response(read(&state, context, Operation::Inventory).await)
    })
    .await
}

pub(super) async fn resources(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(id): Path<String>,
    Query(query): Query<ResourceQuery>,
) -> Response {
    code_reads::scoped(authority, &id, |context| async move {
        if [
            &query.plugin_id,
            &query.plugin_version,
            &query.generation_digest,
            &query.remote,
            &query.view,
        ]
        .iter()
        .any(|v| v.len() > 256)
            || query.item.as_ref().is_some_and(|v| v.len() > 24)
            || query.filter.as_ref().is_some_and(|v| v.len() > 64)
        {
            return response(ExtensionResponse::Unavailable {
                code: Failure::InvalidRequest,
            });
        }
        response(
            read(
                &state,
                context,
                Operation::Read {
                    identity: Identity {
                        plugin_id: query.plugin_id,
                        plugin_version: query.plugin_version,
                        generation_digest: query.generation_digest,
                    },
                    remote: query.remote,
                    view: query.view,
                    item: query.item,
                    filter: query.filter,
                    page: query.page.unwrap_or(1),
                },
            )
            .await,
        )
    })
    .await
}

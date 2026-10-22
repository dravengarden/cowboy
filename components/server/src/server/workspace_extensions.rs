//! Workspace-scoped extension reads always execute on the original Machine.

use super::*;
use crate::workspace_extensions::{
    Failure, Identity, Operation, PullDiscovery, Request, Response as ExtensionResponse, ReviewRead,
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
    review: Option<bool>,
    repository_id: Option<String>,
    revision: Option<String>,
    repository: Option<String>,
    discovery: Option<bool>,
    relation: Option<String>,
    state: Option<String>,
    current_repository: Option<bool>,
    account: Option<String>,
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
            || query.repository_id.as_ref().is_some_and(|v| v.len() > 24)
            || query.revision.as_ref().is_some_and(|v| v.len() > 256)
            || query.repository.as_ref().is_some_and(|v| v.len() > 201)
            || query.account.as_ref().is_some_and(|v| v.len() > 100)
            || query.relation.as_ref().is_some_and(|v| v.len() > 32)
            || query.state.as_ref().is_some_and(|v| v.len() > 16)
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
                    review: query
                        .review
                        .unwrap_or(false)
                        .then_some(Box::new(ReviewRead {
                            repository_id: query.repository_id,
                            revision: query.revision,
                            repository: query.repository,
                        })),
                    discovery: query.discovery.unwrap_or(false).then_some(Box::new(
                        PullDiscovery {
                            relation: query.relation.unwrap_or_else(|| "author".into()),
                            state: query.state.unwrap_or_else(|| "open".into()),
                            current_repository: query.current_repository.unwrap_or(false),
                            account: query.account,
                        },
                    )),
                },
            )
            .await,
        )
    })
    .await
}

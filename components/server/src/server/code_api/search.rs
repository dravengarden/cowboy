//! Code search: fuzzy filename search and bounded content search (`search@1`).
//! Both run no project code, so they serve restricted worktrees too.

use super::*;
use crate::code_review::content_search::{ContentQuery, ContentSearchResult, DEFAULT_LIMIT};

pub(in crate::server) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/code/sessions/{id}/search", get(api_code_search))
        .route(
            "/api/code/sessions/{id}/content-search",
            get(api_code_content_search),
        )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodeSearchResponse {
    api_version: u8,
    files: Vec<String>,
}

async fn api_code_search(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<FileSearchQuery>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let limit = query.limit.clamp(1, 100);
        let files = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::Search {
                query: query.q.clone(),
                limit,
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Search(files))) => files,
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote code search unavailable").into_response();
            }
            Ok(None) => tokio::task::spawn_blocking(move || {
                crate::code_review::LocalCodeProvider::new(cwd).search(&query.q, limit)
            })
            .await
            .unwrap_or_default(),
        };
        Json(CodeSearchResponse {
            api_version: 1,
            files,
        })
        .into_response()
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::server) struct ContentSearchParams {
    pub(in crate::server) q: String,
    #[serde(default)]
    pub(in crate::server) regex: bool,
    /// `true`/`false`; absent is smart case.
    pub(in crate::server) case_sensitive: Option<bool>,
    pub(in crate::server) path: Option<String>,
    pub(in crate::server) limit: Option<usize>,
}

impl ContentSearchParams {
    pub(in crate::server) fn query(self) -> ContentQuery {
        ContentQuery {
            query: self.q,
            regex: self.regex,
            case_sensitive: self.case_sensitive,
            path: self.path.filter(|path| !path.is_empty()),
            limit: self.limit.unwrap_or(DEFAULT_LIMIT),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentSearchResponse {
    api_version: u8,
    #[serde(flatten)]
    result: ContentSearchResult,
}

/// Run one bounded content search in a resolved code context. Remote Machines
/// whose adapter predates the operation refuse it; that is `unavailable`, never
/// an empty result.
pub(in crate::server) async fn content_search(
    state: &AppState,
    scope: &CodeReadScope,
    cwd: String,
    query: ContentQuery,
) -> Result<ContentSearchResult, (StatusCode, String)> {
    match remote_code_request(
        state,
        scope,
        CodeOperation::ContentSearch {
            query: query.clone(),
        },
    )
    .await
    {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::ContentSearch(result))) => Ok(result),
        Ok(Some(_)) | Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "content search is unavailable on this Machine".to_owned(),
        )),
        Ok(None) => tokio::task::spawn_blocking(move || {
            crate::code_review::content_search::search(std::path::Path::new(&cwd), &query)
        })
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "content search failed".to_owned(),
            )
        })?
        .map_err(|error| (StatusCode::BAD_REQUEST, error)),
    }
}

async fn api_code_content_search(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(params): Query<ContentSearchParams>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        match content_search(&state, &context.scope, context.cwd, params.query()).await {
            Ok(result) => Json(ContentSearchResponse {
                api_version: 1,
                result,
            })
            .into_response(),
            Err((status, message)) => (status, message).into_response(),
        }
    })
    .await
}

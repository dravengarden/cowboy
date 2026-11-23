//! HTTP routes for code browsing, git/diff reads, workspace extensions, and code intelligence.

use super::*;

/// Code API, search, check, workspace, and buffer lifecycle routes.
pub fn routes() -> Router<Arc<CodeState>> {
    Router::new()
        .route(
            "/api/code/sessions/{id}/extensions",
            get(workspace_extensions::inventory),
        )
        .route(
            "/api/code/sessions/{id}/extensions/resources",
            get(workspace_extensions::resources),
        )
        .route(
            "/api/sessions/reconcile-project",
            post(api_reconcile_project_sessions),
        )
        .route("/api/sessions/{id}/review-fetch", post(api_review_fetch))
        .route("/api/workspaces", get(api_workspaces))
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/api/code/sessions/{id}/tree", get(api_file_tree))
        .merge(code_api::search_routes())
        .merge(code_api::check_routes())
        .route("/api/code/sessions/{id}/manifest", get(api_code_manifest))
        .route("/api/code/sessions/{id}/changes", get(api_code_changes))
        .route(
            "/api/code/sessions/{id}/repository",
            get(api_code_repository),
        )
        .route("/api/code/sessions/{id}/commit", get(api_code_commit))
        .route(
            "/api/code/sessions/{id}/commit-diff",
            get(api_code_commit_diff),
        )
        .route("/api/code/sessions/{id}/diff", get(api_code_diff))
        .route("/api/code/sessions/{id}/file", get(api_code_file))
        .route("/api/code/sessions/{id}/file-raw", get(api_code_file_raw))
        .route("/api/code/sessions/{id}/blob", get(api_code_blob))
        .route("/api/code/sessions/{id}/file-blob", get(api_code_file_blob))
        .route("/api/code/sessions/{id}/language", get(api_code_language))
        .route(
            "/api/code/sessions/{id}/intelligence/hover",
            get(api_code_hover),
        )
        .route(
            "/api/code/sessions/{id}/intelligence/navigation",
            get(api_code_navigation),
        )
        .route(
            "/api/code/sessions/{id}/intelligence/outline",
            get(api_code_outline),
        )
        .route(
            "/api/code/sessions/{id}/buffer",
            put(api_code_buffer_open).delete(api_code_buffer_close),
        )
}

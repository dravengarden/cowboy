//! Widget HTTP routes owned by the Agent API crate.

use super::*;

/// Interactive widget decision and full-DOM app message routes.
pub fn widget_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/sessions/{id}/widgets/{widget_id}/interactions",
            post(agent_api::api_widget_interaction),
        )
        .route(
            "/api/sessions/{id}/widgets/{widget_id}/apps/{node_id}/messages",
            post(agent_api::api_widget_app_message),
        )
}

/// Independent authenticated Machine HTTPS route for Antigravity post-edit
/// diagnostic pulls (Decision 2A / R4).
pub fn machine_post_edit_routes() -> Router<Arc<AppState>> {
    Router::new().route(
        crate::post_edit_inspect::POST_EDIT_INSPECT_PATH,
        post(agent_api::api_machine_post_edit_inspect),
    )
}

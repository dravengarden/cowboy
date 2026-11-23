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

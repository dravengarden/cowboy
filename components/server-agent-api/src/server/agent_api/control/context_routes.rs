//! Session context adapters: links, fresh-agent handoff, copies and context
//! renewal, bound to the product handlers.
use super::*;

/// The product route behind a session context operation, if `name` is one.
pub(super) fn context_route(
    name: &str,
) -> Option<(&'static str, &'static str, MethodRouter<Arc<AppState>>)> {
    Some(match name {
        "sessions.duplicate" => (
            "POST",
            "/api/sessions/{session_id}/duplicate",
            post(web::session_context::duplicate),
        ),
        "sessions.context" => (
            "GET",
            "/api/sessions/{session_id}/context",
            get(web::session_context::get),
        ),
        "sessions.lineage" => (
            "GET",
            "/api/sessions/{session_id}/lineage",
            get(web::session_context::lineage_get),
        ),
        "sessions.lineage_history" => (
            "GET",
            "/api/sessions/{session_id}/lineage/{hop_id}/history",
            get(web::session_context::lineage_history),
        ),
        "sessions.lineage_question_pages" => (
            "GET",
            "/api/sessions/{session_id}/lineage/{hop_id}/question-pages",
            get(web::session_context::lineage_question_pages),
        ),
        "sessions.lineage_question_page" => (
            "GET",
            "/api/sessions/{session_id}/lineage/{hop_id}/question-pages/{page_id}",
            get(web::session_context::lineage_question_page),
        ),
        "sessions.context_set" => (
            "PUT",
            "/api/sessions/{session_id}/context",
            put(web::session_context::set),
        ),
        "sessions.context_send" => (
            "POST",
            "/api/sessions/{session_id}/context/send",
            post(web::session_context::send),
        ),
        "sessions.fork" => (
            "POST",
            "/api/sessions/{session_id}/fork",
            post(web::session_context::fork),
        ),
        "sessions.context_rollover" => (
            "GET",
            "/api/sessions/{session_id}/context-rollover",
            get(web::session_context::rollover::get),
        ),
        "sessions.context_rollover_set" => (
            "PUT",
            "/api/sessions/{session_id}/context-rollover",
            put(web::session_context::rollover::set),
        ),
        "sessions.cache_metrics" => (
            "GET",
            "/api/sessions/{session_id}/cache-metrics",
            get(web::cache_metrics_api::session_cache_metrics),
        ),
        "accounts.cache_metrics" => (
            "GET",
            "/api/provider-accounts/{account}/cache-metrics",
            get(web::cache_metrics_api::account_cache_metrics),
        ),
        _ => return None,
    })
}

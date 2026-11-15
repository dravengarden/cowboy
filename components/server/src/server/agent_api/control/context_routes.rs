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
        _ => return None,
    })
}

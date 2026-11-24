//! Account-control adapters: the Antigravity account controls, named-account
//! usage resets and the uncertain-effect acknowledgement an agent may drive
//! for its owner, bound to the product handlers.
use super::*;

/// The product route behind an account-control operation, if `name` is one.
pub(super) fn account_route(
    name: &str,
) -> Option<(
    &'static str,
    &'static str,
    MethodRouter<Arc<web::AccountsState>>,
)> {
    Some(match name {
        "accounts.control" => (
            "POST",
            "/api/provider-accounts/{account}/controls",
            post(web::provider_accounts::controls::http::http_control),
        ),
        "usage.account_reset" => (
            "POST",
            "/api/provider-accounts/{account}/usage/reset",
            post(web::provider_accounts::usage::reset::http_reset),
        ),
        "usage.account_reset_schedule" => (
            "PUT",
            "/api/provider-accounts/{account}/usage/reset/schedule",
            put(web::provider_accounts::usage::reset::schedule::http_schedule),
        ),
        "usage.account_reset_cancel" => (
            "DELETE",
            "/api/provider-accounts/{account}/usage/reset/schedule",
            delete(web::provider_accounts::usage::reset::schedule::http_cancel),
        ),
        "sessions.acknowledge_uncertain_effect" => (
            "POST",
            "/api/sessions/{session_id}/uncertain-effect/acknowledge",
            post(web::provider_accounts::controls::http::http_acknowledge),
        ),
        _ => return None,
    })
}

/// A write's receipt status. An effect that may have reached the Provider (an
/// unknown or still-running reset) is never a rejection: its receipt stays
/// unknown so no caller replays it as refused.
pub(super) fn receipt_status(status: axum::http::StatusCode, value: &Value) -> &'static str {
    if status.is_success() {
        "accepted"
    } else if matches!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("reset_outcome_unknown")
    ) {
        "unknown"
    } else {
        "rejected"
    }
}

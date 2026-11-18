//! Account-control adapters: the Antigravity account controls and the
//! uncertain-effect acknowledgement an agent may drive for its owner, bound
//! to the product handlers.
use super::*;

/// The product route behind an account-control operation, if `name` is one.
pub(super) fn account_route(
    name: &str,
) -> Option<(&'static str, &'static str, MethodRouter<Arc<AppState>>)> {
    Some(match name {
        "accounts.control" => (
            "POST",
            "/api/provider-accounts/{account}/controls",
            post(web::provider_accounts::controls::http::http_control),
        ),
        "sessions.acknowledge_uncertain_effect" => (
            "POST",
            "/api/sessions/{session_id}/uncertain-effect/acknowledge",
            post(web::provider_accounts::controls::http::http_acknowledge),
        ),
        _ => return None,
    })
}

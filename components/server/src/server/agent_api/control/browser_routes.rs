//! Browser control adapters: the Machine-browser operations an agent may
//! drive for its owner's named accounts, bound to the product handlers.
use super::*;

/// The product route behind a browser control operation, if `name` is one.
pub(super) fn browser_route(
    name: &str,
) -> Option<(&'static str, &'static str, MethodRouter<Arc<AppState>>)> {
    Some(match name {
        "accounts.browser_open" => (
            "POST",
            "/api/provider-accounts/{account}/browser/open",
            post(web::provider_accounts::browser::http_open),
        ),
        "accounts.browser_navigate" => (
            "POST",
            "/api/provider-accounts/{account}/browser/navigate",
            post(web::provider_accounts::browser::http_navigate),
        ),
        "accounts.browser_snapshot" => (
            "POST",
            "/api/provider-accounts/{account}/browser/snapshot",
            post(web::provider_accounts::browser::http_snapshot),
        ),
        "accounts.browser_hibernate" => (
            "POST",
            "/api/provider-accounts/{account}/browser/hibernate",
            post(web::provider_accounts::browser::http_hibernate),
        ),
        "accounts.browser_reset" => (
            "POST",
            "/api/provider-accounts/{account}/browser/reset",
            post(web::provider_accounts::browser::http_reset),
        ),
        _ => return None,
    })
}

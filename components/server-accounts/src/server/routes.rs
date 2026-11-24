//! HTTP and WebSocket routes for Provider accounts, controls, logins, revocation, and browsers.

use super::*;
use axum::extract::DefaultBodyLimit;

/// Named-account management, usage, revocation, login, controls, and browser routes.
pub fn accounts_routes() -> Router<Arc<AccountsState>> {
    Router::new()
        .route(
            "/api/provider-accounts",
            get(provider_accounts::read)
                .put(provider_accounts::update)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/usage",
            get(provider_accounts::usage::list),
        )
        .route(
            "/api/provider-accounts/{account_id}/usage",
            get(provider_accounts::usage::read).post(provider_accounts::usage::refresh),
        )
        .route(
            "/api/provider-accounts/{account_id}/usage/reset",
            post(provider_accounts::usage::reset::http_reset),
        )
        .route(
            "/api/provider-accounts/{account_id}/usage/reset/schedule",
            put(provider_accounts::usage::reset::schedule::http_schedule)
                .delete(provider_accounts::usage::reset::schedule::http_cancel),
        )
        .route(
            "/api/provider-accounts/{account_id}/revocation",
            get(provider_accounts::lifecycle::http_status)
                .post(provider_accounts::lifecycle::http_start)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/revocation/reconcile",
            post(provider_accounts::lifecycle::http_resume).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/login-operations/{operation}",
            get(provider_accounts::login::http_receipt)
                .delete(provider_accounts::login::http_cancel),
        )
        .route(
            "/api/provider-accounts/{account_id}/logins",
            get(provider_accounts::login::http_logins),
        )
        .route(
            "/api/provider-accounts/{account_id}/logins/{request}",
            get(provider_accounts::login::http_continue)
                .delete(provider_accounts::login::http_continue_cancel),
        )
        .route(
            "/api/provider-accounts/{account_id}/credentials",
            post(provider_accounts::portable::http_commit).layer(DefaultBodyLimit::max(32 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/login",
            get(provider_accounts::login::options)
                .post(provider_accounts::login::start)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/controls",
            post(provider_accounts::controls::http::http_control)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .merge(provider_browser_routes())
}

/// Owner-operated Machine browsers for named accounts: state, lifecycle
/// commands and the viewer's display stream. No route couples a browser to
/// a Provider login.
fn provider_browser_routes() -> Router<Arc<AccountsState>> {
    Router::new()
        .route(
            "/api/provider-accounts/{account_id}/browser",
            get(provider_accounts::browser::http_state),
        )
        .route(
            "/api/provider-accounts/{account_id}/browser/open",
            post(provider_accounts::browser::http_open).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/browser/navigate",
            post(provider_accounts::browser::http_navigate).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/browser/resize",
            post(provider_accounts::browser::http_resize).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/browser/snapshot",
            post(provider_accounts::browser::http_snapshot).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/browser/hibernate",
            post(provider_accounts::browser::http_hibernate)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/browser/reset",
            post(provider_accounts::browser::http_reset).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/provider-accounts/{account_id}/browser/stream",
            get(provider_accounts::browser::viewer::upgrade),
        )
}

/// Machine-side browser display stream WebSocket route.
pub fn machine_stream_routes() -> Router<Arc<AccountsState>> {
    Router::new().route(
        crate::machine_protocol::machine_browser::STREAM_PATH,
        any(provider_accounts::browser::machine_stream::upgrade),
    )
}

/// Session uncertain-effect acknowledgement route owned by account controls.
pub fn session_acknowledge_routes() -> Router<Arc<AccountsState>> {
    Router::new().route(
        "/api/sessions/{id}/uncertain-effect/acknowledge",
        post(provider_accounts::controls::http::http_acknowledge),
    )
}

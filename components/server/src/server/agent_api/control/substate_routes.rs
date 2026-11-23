//! Sub-state route adapters (plugins, accounts, browsers, code) for agent control dispatch.

use super::*;

fn plugin_route(
    name: &str,
) -> Option<(
    &'static str,
    &'static str,
    MethodRouter<Arc<web::PluginsState>>,
)> {
    Some(match name {
        "plugins.machine_receipt" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/operations/{operation}/machine-receipt",
            get(web::api_machine_plugin_operation_receipt),
        ),
        "plugins.recovery_assessment" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/operations/{operation}/recovery-assessment",
            get(web::api_machine_plugin_recovery_assessment),
        ),
        "plugins.operations" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/operations",
            get(web::api_machine_plugin_operations),
        ),
        "plugins.install_operations" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/installation-operations",
            get(web::plugin_install::api_machine_plugin_install_operations),
        ),
        _ => return None,
    })
}

fn code_route(
    name: &str,
) -> Option<(
    &'static str,
    &'static str,
    MethodRouter<Arc<web::CodeState>>,
)> {
    Some(match name {
        "projects.workspaces" => ("GET", "/api/workspaces", get(web::api_workspaces)),
        "sessions.review_fetch" => (
            "POST",
            "/api/sessions/{session_id}/review-fetch",
            post(web::api_review_fetch),
        ),
        _ => return None,
    })
}

pub(super) fn resolve_router(
    state: &Arc<AppState>,
    name: &str,
) -> Option<(&'static str, &'static str, Router<()>)> {
    if let Some((m, p, h)) =
        browser_routes::browser_route(name).or_else(|| account_routes::account_route(name))
    {
        return Some((
            m,
            p,
            Router::new()
                .route(p, h)
                .with_state(Arc::clone(&state.accounts)),
        ));
    }
    if let Some((m, p, h)) = plugin_route(name) {
        return Some((
            m,
            p,
            Router::new()
                .route(p, h)
                .with_state(Arc::clone(&state.plugins)),
        ));
    }
    if let Some((m, p, h)) = code_route(name) {
        return Some((
            m,
            p,
            Router::new()
                .route(p, h)
                .with_state(Arc::clone(&state.code)),
        ));
    }
    let (m, p, h) = route(name)?;
    Some((
        m,
        p,
        Router::new().route(p, h).with_state(Arc::clone(state)),
    ))
}

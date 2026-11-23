use super::*;
use crate::product_auth::ProductPrincipal;
use tower::ServiceExt as _;

fn principal(user_id: &str, role: crate::admin::AdminRole) -> ProductPrincipal {
    ProductPrincipal {
        user_id: user_id.to_owned(),
        username: user_id.to_owned(),
        role,
    }
}

fn hub_with_owned_session() -> Hub {
    let hub = Hub::new();
    hub.create_session(crate::core::SessionRegistration {
        id: "sess-5".into(),
        provider: "codex".into(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "hawk".into(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/tmp".into(),
        title: String::new(),
        origin: crate::core::SessionOrigin::Web,
        system: false,
        owner_user_id: Some("owner".into()),
        owner_username: None,
    });
    hub
}

/// Runs the check in front of a handler that reads the decoded `Path`, so
/// the test observes what the real extractor would have delivered.
async fn status(hub: Hub, who: ProductPrincipal, method: Method, uri: &str) -> StatusCode {
    let guard = move |request: axum::extract::Request, next: Next| {
        let (hub, who) = (hub.clone(), who.clone());
        async move {
            let path = request.uri().path().to_owned();
            match authorize_session_route(&hub, &who, request.method(), &path) {
                Ok(()) => next.run(request).await,
                Err(status) => status.into_response(),
            }
        }
    };
    let reached = |Path(id): Path<String>| async move {
        assert_eq!(id, "sess-5");
        StatusCode::IM_A_TEAPOT
    };
    Router::new()
        .route("/api/sessions/{id}/prompt", post(reached))
        .route("/api/history/{id}", get(reached))
        .layer(middleware::from_fn(guard))
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_percent_encoded_id_reaches_the_same_ownership_check() {
    use crate::admin::AdminRole::{Operator, Viewer};
    for (method, uri) in [
        (Method::POST, "/api/sessions/%73ess-5/prompt"),
        (Method::GET, "/api/history/%73ess-5"),
        (Method::POST, "/api/sessions/sess-5/prompt"),
        (Method::GET, "/api/history/sess-5"),
    ] {
        assert_eq!(
            status(
                hub_with_owned_session(),
                principal("other", Viewer),
                method.clone(),
                uri
            )
            .await,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
        assert_eq!(
            status(
                hub_with_owned_session(),
                principal("owner", Operator),
                method,
                uri
            )
            .await,
            StatusCode::IM_A_TEAPOT,
            "{uri}"
        );
    }
}

#[test]
fn undecodable_and_unknown_ids_stop_before_session_handlers() {
    let hub = hub_with_owned_session();
    let who = principal("owner", crate::admin::AdminRole::Owner);
    for path in ["/api/sessions/%ff/prompt", "/api/sessions/sess-404/prompt"] {
        assert_eq!(
            authorize_session_route(&hub, &who, &Method::POST, path),
            Err(StatusCode::NOT_FOUND),
            "{path}"
        );
    }
    assert_eq!(
        authorize_session_route(&hub, &who, &Method::GET, "/api/code/sessions/%ff/tree"),
        Err(StatusCode::NOT_FOUND)
    );
}

#[test]
fn code_routes_keep_workspace_contexts_for_their_own_authority() {
    let hub = hub_with_owned_session();
    let who = principal("owner", crate::admin::AdminRole::Viewer);
    assert_eq!(
        authorize_session_route(
            &hub,
            &who,
            &Method::GET,
            "/api/code/sessions/workspace%3Ahawk%3Acowboy/tree"
        ),
        Ok(())
    );
}

//! Signed sessions, active-client leases, devices and API tokens the
//! current user owns.

use super::*;

pub(in crate::server) async fn api_auth_list_sessions(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some((session, user)) = product_session_and_user_from_store_cookie(store, &headers).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(response) =
        ensure_product_session_fresh(store, &session, &state.product_authentication).await
    {
        return response;
    }
    let now_ms = auth_now_ms();
    match tokio::try_join!(
        store.list_user_sessions_for_user(&user.id, now_ms),
        store.list_active_clients_for_user(&user.id, now_ms),
        store.list_user_devices_for_user(&user.id),
    ) {
        Ok((sessions, active_clients, authorized_clients)) => no_store_json(
            StatusCode::OK,
            serde_json::json!({
                "sessions": sessions
                    .into_iter()
                    .map(|item| session_public_json(item, Some(&session.session_id)))
                    .collect::<Vec<_>>(),
                "active_clients": active_clients,
                "authorized_clients": authorized_clients.len(),
                "limit": state.product_authentication.capacity.signed_in_sessions_per_user,
                "active_limit": state.product_authentication.capacity.active_clients_per_user,
                "enforcement": state.product_authentication.capacity.enforcement,
            }),
        ),
        Err(error) => {
            tracing::error!(%error, "user_session_list");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_delete_session(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some((current_session, user)) =
        product_session_and_user_from_store_cookie(store, &headers).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(response) =
        ensure_product_session_fresh(store, &current_session, &state.product_authentication).await
    {
        return response;
    }
    if !product_session_has_recent_step_up(&current_session, auth_now_ms()) {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Recent login or Passkey verification required",
        )
            .into_response();
    }
    match store
        .revoke_user_session_for_user(&user.id, &id, "user_revoked", auth_now_ms())
        .await
    {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {
            tracing::info!(
                user_id = user.id,
                session_id = id,
                current = id == current_session.session_id,
                "user_session_revoked"
            );
            state.capacity_runtime.notify_changed();
            let mut response = no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }));
            if id == current_session.session_id
                && let Ok(value) = crate::product_auth::clear_session_cookie(
                    crate::product_auth::request_is_https(&headers),
                )
                .parse()
            {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            response
        }
        Err(error) => {
            tracing::error!(%error, "user_session_revoke");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_release_active_client(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    Query(query): Query<ActiveClientReleaseQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    if query.fencing_token <= 0 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let user = match require_fresh_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store
        .release_active_client_for_user(
            &user.id,
            &ActiveClientRelease {
                client_id: &id,
                fencing_token: query.fencing_token,
                reason: "user_reclaimed",
                now_ms: auth_now_ms(),
                reservation_ms: state.product_authentication.capacity.reservation_ms,
                per_user_limit: state
                    .product_authentication
                    .capacity
                    .active_clients_per_user,
            },
        )
        .await
    {
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Ok(true) => {
            tracing::info!(user_id = user.id, client_id = id, "active_client_reclaimed");
            state.capacity_runtime.notify_changed();
            no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Err(error) => {
            tracing::error!(%error, "active_client_reclaim");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_list_devices(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let user = match require_fresh_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store.list_user_devices_for_user(&user.id).await {
        Ok(devices) => no_store_json(
            StatusCode::OK,
            serde_json::json!({
                "devices": devices.into_iter().map(device_public_json).collect::<Vec<_>>(),
            }),
        ),
        Err(error) => {
            tracing::error!(%error, "device_list");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_delete_device(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    let user = match require_recent_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    match store
        .revoke_user_device_for_user(&user.id, &id, auth_now_ms())
        .await
    {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {
            state.device_access.revoke_device(&id);
            tracing::info!(user_id = user.id, device_id = id, "user_device_revoked");
            no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Err(error) => {
            tracing::error!(%error, "device_revoke");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

pub(in crate::server) async fn api_auth_list_tokens(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    let Some((principal, cookie_session)) = resolve_product_request_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    if let Some(session) = cookie_session.as_ref()
        && let Err(response) =
            ensure_product_session_fresh(store, session, &state.product_authentication).await
    {
        return response;
    }
    match store
        .list_user_api_tokens_for_user(&principal.user_id)
        .await
    {
        Ok(tokens) => Json(serde_json::json!({
            "tokens": tokens.iter().map(token_public_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(in crate::server) async fn api_auth_create_token(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CreateApiTokenRequest>,
) -> Response {
    let peer = peer_addr(peer);
    if crate::product_auth::bearer_token(&headers).is_none()
        && let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins)
    {
        return rejected;
    }
    let Some((principal, cookie_session)) = resolve_product_request_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !principal.role.at_least(crate::admin::AdminRole::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(store) = durable_store(&state.store).cloned() else {
        return missing_store();
    };
    if let Some(session) = cookie_session.as_ref()
        && let Err(response) =
            ensure_product_session_fresh(&store, session, &state.product_authentication).await
    {
        return response;
    }
    let name = match crate::product_auth::ensure_api_token_name(&request.name) {
        Ok(name) => name,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let ttl_secs = match crate::product_auth::api_token_ttl_secs(request.ttl_seconds) {
        Ok(ttl) => ttl,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let secret = match crate::product_auth::new_api_token_secret() {
        Ok(secret) => secret,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let now = auth_now_ms();
    let token = crate::store::ProductApiToken {
        id: match crate::product_auth::new_user_id() {
            Ok(id) => id,
            Err(error) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
            }
        },
        user_id: principal.user_id.clone(),
        name,
        token_prefix: crate::product_auth::api_token_prefix(&secret),
        token_hash: crate::admin::hex_sha256(secret.as_bytes()),
        created_at_ms: now,
        expires_at_ms: Some(now.saturating_add(ttl_secs.saturating_mul(1_000))),
        last_used_at_ms: None,
        revoked_at_ms: None,
    };
    if let Err(error) = store.insert_user_api_token(&token).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    tracing::info!(
        user_id = principal.user_id,
        token_id = token.id,
        token_prefix = %token.token_prefix,
        "product_api_token_created"
    );
    let mut body = token_public_json(&token);
    body["token"] = serde_json::Value::String(secret);
    (StatusCode::CREATED, Json(body)).into_response()
}

pub(in crate::server) async fn api_auth_delete_token(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let peer = peer_addr(peer);
    if crate::product_auth::bearer_token(&headers).is_none()
        && let Some(rejected) = reject_bad_origin(&headers, peer, &state.public_origins)
    {
        return rejected;
    }
    let Some((principal, cookie_session)) = resolve_product_request_principal(
        state.product_auth_enabled,
        state.store.as_ref(),
        &state.hub,
        &headers,
    )
    .await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    if let Some(session) = cookie_session.as_ref()
        && let Err(response) =
            ensure_product_session_fresh(store, session, &state.product_authentication).await
    {
        return response;
    }
    match store
        .revoke_user_api_token_for_user(&principal.user_id, &id)
        .await
    {
        Ok(0) => StatusCode::NOT_FOUND.into_response(),
        Ok(_) => {
            tracing::info!(
                user_id = principal.user_id,
                token_id = id,
                "product_api_token_revoked"
            );
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

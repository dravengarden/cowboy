//! Device authorization grants and device credential exchange and refresh,
//! including Cardea device sign-in.

use super::*;

pub(in crate::server) fn device_authorization_error(error: &anyhow::Error) -> Response {
    tracing::info!(%error, "device_authorization_rejected");
    let message = error.to_string();
    let status = if message.contains("too many") {
        StatusCode::TOO_MANY_REQUESTS
    } else if message.contains("unavailable") || message.contains("no longer") {
        StatusCode::GONE
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, "device authorization is unavailable").into_response()
}

pub(in crate::server) fn device_verification_url(
    public_origins: &[String],
    request_id: &str,
    approval_token: &str,
) -> String {
    let path = format!("/auth/device#request_id={request_id}&approval_token={approval_token}");
    public_origins
        .first()
        .map_or(path.clone(), |origin| format!("{origin}{path}"))
}

pub(in crate::server) async fn api_auth_device_authorization_start(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::client_auth::StartAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if durable_store(&state.store).is_none() {
        return missing_store();
    }
    let source_ip = crate::product_auth::client_ip(&headers, peer_addr(peer));
    match state
        .device_authorizations
        .start(request, source_ip, auth_now_ms())
    {
        Ok((request_id, approval_token, expires_at_ms)) => {
            let verification_url = device_verification_url(
                state.public_origins.as_ref(),
                &request_id,
                &approval_token,
            );
            no_store_json(
                StatusCode::CREATED,
                crate::client_auth::StartAuthorizationResponse {
                    request_id,
                    verification_url,
                    expires_at_ms,
                },
            )
        }
        Err(error) => device_authorization_error(&error),
    }
}

pub(in crate::server) async fn api_auth_device_authorization_inspect(
    State(state): State<ProductAuthState>,
    Json(request): Json<crate::client_auth::BrowserAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    match state
        .device_authorizations
        .browser_info(&request.request_id, &request.approval_token)
    {
        Ok(info) => no_store_json(StatusCode::OK, info),
        Err(error) => device_authorization_error(&error),
    }
}

pub(in crate::server) async fn api_auth_device_authorization_approve(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::client_auth::BrowserAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    let user = match require_recent_product_user(&state, &headers).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    match state.device_authorizations.approve(&request, &user.id) {
        Ok(()) => {
            tracing::info!(
                user_id = user.id,
                request_id = request.request_id,
                "device_authorization_approved"
            );
            no_store_json(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Err(error) => device_authorization_error(&error),
    }
}

pub(in crate::server) async fn api_auth_device_authorization_deny(
    State(state): State<ProductAuthState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<crate::client_auth::BrowserAuthorizationRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(rejected) = reject_bad_origin(&headers, peer_addr(peer), &state.public_origins) {
        return rejected;
    }
    if let Err(response) = require_product_user(&state, &headers).await {
        return response;
    }
    match state.device_authorizations.deny(&request) {
        Ok(()) => no_store_json(StatusCode::OK, serde_json::json!({ "ok": true })),
        Err(error) => device_authorization_error(&error),
    }
}

pub(in crate::server) async fn api_auth_device_authorization_events(
    State(state): State<ProductAuthState>,
    ws: WebSocketUpgrade,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    ws.max_message_size(4 * 1_024)
        .max_frame_size(4 * 1_024)
        .on_upgrade(move |socket| handle_device_authorization_events(socket, state))
}

pub(in crate::server) async fn handle_device_authorization_events(
    mut socket: WebSocket,
    state: ProductAuthState,
) {
    let handshake =
        match tokio::time::timeout(DEVICE_AUTH_EVENTS_HANDSHAKE_TIMEOUT, socket.recv()).await {
            Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<
                crate::client_auth::AuthorizationEventsHandshake,
            >(text.as_str())
            .ok(),
            _ => None,
        };
    let Some(handshake) = handshake else {
        let _ = socket.close().await;
        return;
    };
    let mut events = match state
        .device_authorizations
        .subscribe(&handshake.request_id, &handshake.code_verifier)
    {
        Ok(events) => events,
        Err(error) => {
            tracing::info!(%error, "device_authorization_events_rejected");
            let _ = send_json(
                &mut socket,
                &crate::client_auth::AuthorizationEventStatus {
                    status: "unavailable".to_owned(),
                },
            )
            .await;
            let _ = socket.close().await;
            return;
        }
    };
    let deadline = tokio::time::sleep(DEVICE_AUTH_EVENTS_TTL);
    tokio::pin!(deadline);
    let mut heartbeat = tokio::time::interval(DEVICE_AUTH_EVENTS_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    loop {
        let terminal = match *events.borrow_and_update() {
            crate::client_auth::AuthorizationEvent::Pending => None,
            crate::client_auth::AuthorizationEvent::Approved => Some("approved"),
            crate::client_auth::AuthorizationEvent::Denied => Some("denied"),
        };
        if let Some(status) = terminal {
            let _ = send_json(
                &mut socket,
                &crate::client_auth::AuthorizationEventStatus {
                    status: status.to_owned(),
                },
            )
            .await;
            let _ = socket.close().await;
            return;
        }
        tokio::select! {
            changed = events.changed() => {
                if changed.is_err() {
                    let _ = send_json(
                        &mut socket,
                        &crate::client_auth::AuthorizationEventStatus {
                            status: "unavailable".to_owned(),
                        },
                    ).await;
                    let _ = socket.close().await;
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(Message::Ping(payload))) => {
                    if socket.send(Message::Pong(payload)).await.is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(_)) => {
                    let _ = socket.close().await;
                    return;
                }
            },
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
            }
            () = &mut deadline => {
                let _ = send_json(
                    &mut socket,
                    &crate::client_auth::AuthorizationEventStatus {
                        status: "unavailable".to_owned(),
                    },
                ).await;
                let _ = socket.close().await;
                return;
            }
        }
    }
}

pub(in crate::server) async fn api_auth_cardea_device_configuration(
    State(state): State<ProductAuthState>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(provider) = state.product_authentication.provider("cardea") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match provider.cardea_device_configuration() {
        Ok(configuration) => Json(configuration).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(in crate::server) async fn api_auth_cardea_device_exchange(
    State(state): State<ProductAuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CardeaDeviceExchangeRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(provider) = state.product_authentication.provider("cardea") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let source_ip = crate::product_auth::client_ip(&headers, peer);
    apply_rate_limit(&state, "cardea-device-exchange", &source_ip.to_string()).await;
    if request.name.is_empty()
        || request.name.chars().count() > 80
        || request
            .name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>'))
        || crate::client_auth::decode_public_key(&request.public_key).is_err()
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let user = match store.user_by_username(provider.account()).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let identity = match provider
        .exchange_cardea_device(
            &request.access_token,
            &request.assertion,
            &request.public_key,
        )
        .await
    {
        Ok(identity) => identity,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    // One stable Cowboy device per Cardea grant; refresh cannot bypass local
    // device revocation or allocate a new capacity slot on every request.
    let id = match provider.cardea_product_device_id(&identity.grant_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let devices = match store.list_user_devices_for_user(&user.id).await {
        Ok(devices) => devices,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let now = auth_now_ms();
    if let Some(device) = devices.iter().find(|d| d.id == id) {
        if device.public_key != identity.public_key {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    } else {
        let device = crate::store::ProductDevice {
            id: id.clone(),
            user_id: user.id.clone(),
            name: request.name,
            public_key: identity.public_key.clone(),
            created_at_ms: now,
            last_used_at_ms: Some(now),
            revoked_at_ms: None,
        };
        // Admission owns capacity and its transactional lock. No product
        // refresh bearer is returned: subsequent mints recheck Cardea.
        let secret = match crate::client_auth::new_refresh_token() {
            Ok(v) => v,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        let refresh = crate::store::ProductDeviceRefreshToken {
            token_hash: crate::admin::hex_sha256(secret.as_bytes()),
            device_id: id.clone(),
            family_id: id.clone(),
            created_at_ms: now,
            expires_at_ms: now + 1,
            used_at_ms: None,
            revoked_at_ms: Some(now),
        };
        match store
            .admit_user_device(
                &device,
                &refresh,
                state
                    .product_authentication
                    .capacity
                    .authorized_clients_per_user,
                state.product_authentication.capacity.enforcement
                    == crate::auth_plugins::CapacityEnforcement::Enforce,
            )
            .await
        {
            Ok(crate::store::ProductDeviceAdmission::Admitted { .. }) => {}
            Ok(crate::store::ProductDeviceAdmission::CapacityFull { .. }) => {
                return StatusCode::CONFLICT.into_response();
            }
            // A revoked device keeps its primary key: insertion fails closed.
            // Concurrent first admissions can safely retry with fresh evidence.
            Err(_) => return StatusCode::CONFLICT.into_response(),
        }
    }
    let deadline = match i64::try_from(identity.expires_at)
        .ok()
        .and_then(|t| t.checked_mul(1000))
    {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    }
    .min(now.saturating_add(300_000));
    match state.device_access.issue_identity_device(
        &id,
        &user.id,
        &identity.public_key,
        now,
        deadline,
    ) {
        Ok((token, expiry)) => no_store_json(
            StatusCode::OK,
            serde_json::json!({
                "schema":"dravengarden.cowboy.cardea-device-token/v1", "device_id":id,
                "access_token":token, "access_expires_at_ms":expiry,
            }),
        ),
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

pub(in crate::server) async fn api_auth_device_exchange(
    State(state): State<ProductAuthState>,
    Json(request): Json<crate::client_auth::ExchangeRequest>,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let approved = match state
        .device_authorizations
        .begin_exchange(&request, auth_now_ms())
    {
        Ok(approved) => approved,
        Err(error) => return device_authorization_error(&error),
    };
    let finish_failed = || {
        state
            .device_authorizations
            .finish_exchange(&request.request_id, false)
    };
    let user = match store.user_by_id(&approved.user_id).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => {
            finish_failed();
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_authorization_user_lookup");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let now = auth_now_ms();
    let device_id = match crate::product_auth::new_user_id() {
        Ok(id) => id,
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_id_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let family_id = match crate::product_auth::new_user_id() {
        Ok(id) => id,
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_refresh_family_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let refresh_token = match crate::client_auth::new_refresh_token() {
        Ok(token) => token,
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_refresh_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let refresh_expires_at_ms = now.saturating_add(crate::client_auth::REFRESH_TOKEN_TTL_MS);
    let device = crate::store::ProductDevice {
        id: device_id.clone(),
        user_id: user.id.clone(),
        name: approved.name,
        public_key: approved.public_key,
        created_at_ms: now,
        last_used_at_ms: Some(now),
        revoked_at_ms: None,
    };
    let refresh = crate::store::ProductDeviceRefreshToken {
        token_hash: crate::admin::hex_sha256(refresh_token.as_bytes()),
        device_id: device_id.clone(),
        family_id,
        created_at_ms: now,
        expires_at_ms: refresh_expires_at_ms,
        used_at_ms: None,
        revoked_at_ms: None,
    };
    let admission = store
        .admit_user_device(
            &device,
            &refresh,
            state
                .product_authentication
                .capacity
                .authorized_clients_per_user,
            state.product_authentication.capacity.enforcement
                == crate::auth_plugins::CapacityEnforcement::Enforce,
        )
        .await;
    match admission {
        Ok(crate::store::ProductDeviceAdmission::Admitted {
            observed_overflow, ..
        }) => {
            if observed_overflow {
                tracing::warn!(
                    user_id = user.id,
                    "authorized-client capacity exceeded in observation mode"
                );
            }
        }
        Ok(crate::store::ProductDeviceAdmission::CapacityFull {
            active_clients,
            limit,
        }) => {
            finish_failed();
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "code": "authorized_client_capacity_full",
                    "active": active_clients,
                    "limit": limit,
                    "retryable": false,
                    "action": "revoke_an_authorized_client",
                })),
            )
                .into_response();
        }
        Err(error) => {
            finish_failed();
            tracing::error!(%error, "device_authorization_persist");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    let (access_token, access_expires_at_ms) =
        match state
            .device_access
            .issue(&device.id, &device.user_id, &device.public_key, now)
        {
            Ok(access) => access,
            Err(error) => {
                let _ = store.revoke_user_device(&device.id, now).await;
                finish_failed();
                tracing::error!(%error, "device_access_issue");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    state
        .device_authorizations
        .finish_exchange(&request.request_id, true);
    tracing::info!(
        user_id = user.id,
        device_id,
        "device_authorization_complete"
    );
    no_store_json(
        StatusCode::OK,
        crate::client_auth::DeviceTokenResponse {
            device_id,
            access_token,
            access_expires_at_ms,
            refresh_token,
            refresh_expires_at_ms,
        },
    )
}

pub(in crate::server) async fn api_auth_device_refresh(
    State(state): State<ProductAuthState>,
    headers: HeaderMap,
) -> Response {
    if !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(store) = durable_store(&state.store) else {
        return missing_store();
    };
    let Some(refresh_token) = refresh_bearer(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let token_hash = crate::admin::hex_sha256(refresh_token.as_bytes());
    let (device, previous) = match store.user_device_refresh_by_hash(&token_hash).await {
        Ok(Some(value)) => value,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(error) => {
            tracing::error!(%error, "device_refresh_lookup");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let now = auth_now_ms();
    // Refresh tokens are sender-constrained. Authenticate the key before
    // treating reuse as theft; possession of an obsolete token alone must not
    // let an attacker revoke the legitimate device as a denial of service.
    if let Err(error) = state.device_access.verify_token_proof(
        &headers,
        crate::client_auth::TokenProofContext {
            method: &Method::POST,
            path_and_query: "/api/auth/device/refresh",
            token: &refresh_token,
            device_id: &device.id,
            public_key: &device.public_key,
            now_ms: now,
        },
    ) {
        tracing::info!(%error, device_id = device.id, "device_refresh_proof_rejected");
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if previous.used_at_ms.is_some() {
        tracing::warn!(device_id = device.id, "device_refresh_replay_detected");
        revoke_replayed_device(&state, store, &device.id, now).await;
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if previous.revoked_at_ms.is_some()
        || device.revoked_at_ms.is_some()
        || previous.expires_at_ms <= now
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let user = match store.user_by_id(&device.user_id).await {
        Ok(Some(user)) if user.disabled_at_ms.is_none() => user,
        Ok(_) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(error) => {
            tracing::error!(%error, "device_refresh_user_lookup");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let replacement = match crate::client_auth::new_refresh_token() {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "device_refresh_generation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let next = crate::store::ProductDeviceRefreshToken {
        token_hash: crate::admin::hex_sha256(replacement.as_bytes()),
        device_id: device.id.clone(),
        family_id: previous.family_id,
        created_at_ms: now,
        expires_at_ms: previous.expires_at_ms,
        used_at_ms: None,
        revoked_at_ms: None,
    };
    match store
        .rotate_user_device_refresh(&token_hash, &next, now)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(device_id = device.id, "device_refresh_rotation_race");
            revoke_replayed_device(&state, store, &device.id, now).await;
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(error) => {
            tracing::error!(%error, "device_refresh_rotate");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    let (access_token, access_expires_at_ms) =
        match state
            .device_access
            .issue(&device.id, &user.id, &device.public_key, now)
        {
            Ok(access) => access,
            Err(error) => {
                tracing::error!(%error, "device_access_refresh_issue");
                revoke_replayed_device(&state, store, &device.id, now).await;
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    no_store_json(
        StatusCode::OK,
        crate::client_auth::DeviceTokenResponse {
            device_id: device.id,
            access_token,
            access_expires_at_ms,
            refresh_token: replacement,
            refresh_expires_at_ms: next.expires_at_ms,
        },
    )
}

pub(in crate::server) fn device_public_json(
    device: crate::store::ProductDevice,
) -> serde_json::Value {
    serde_json::json!({
        "id": device.id,
        "name": device.name,
        "created_at_ms": device.created_at_ms,
        "last_used_at_ms": device.last_used_at_ms,
    })
}

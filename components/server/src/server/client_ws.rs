//! The product client WebSocket: upgrade and admission, the command
//! dispatcher for Inbound messages, sync, and outbound projection.

use super::*;
use std::ops::ControlFlow;

mod fanout;
use fanout::*;

pub(super) fn authorize_ws_upgrade(
    headers: &HeaderMap,
    peer: SocketAddr,
    public_origins: &[String],
    principal: Option<ProductPrincipal>,
) -> Result<ProductPrincipal, StatusCode> {
    let via_cookie = crate::product_auth::user_cookie_token(headers).is_some()
        && crate::product_auth::bearer_token(headers).is_none();
    if via_cookie && !crate::product_auth::origin_allowed(headers, peer, public_origins) {
        return Err(StatusCode::FORBIDDEN);
    }
    match principal {
        Some(principal) => Ok(principal),
        None => {
            tracing::info!(reason = "auth_required", "ws_rejected");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

pub(super) fn websocket_capacity_identity(
    product_auth_enabled: bool,
    authenticated: &AuthenticatedProductRequest,
    query: &WebSocketQuery,
) -> Result<WebSocketCapacityIdentity, StatusCode> {
    let requested = query.client_id.as_deref().map(str::trim);
    if requested.is_some_and(|value| {
        !(16..=128).contains(&value.len())
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
    }) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let requested_kind = query.client_kind.as_deref().map(str::trim);
    if requested_kind.is_some_and(|kind| {
        !matches!(
            kind,
            "browser" | "native_shell" | "cli" | "acp" | "automation"
        )
    }) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let (fallback, session_id, principal_class, default_kind, allowed_kinds) =
        if !product_auth_enabled {
            (
                "local-default-client".to_owned(),
                None,
                "system".to_owned(),
                "browser".to_owned(),
                &["browser", "native_shell", "cli", "acp"][..],
            )
        } else if let Some(session) = authenticated.cookie_session.as_ref() {
            (
                format!("session:{}", session.session_id),
                Some(session.session_id.clone()),
                session.principal_class.clone(),
                session.client_kind.clone(),
                &["browser", "native_shell"][..],
            )
        } else if let Some(identity) = authenticated.device_identity.as_ref() {
            if identity.is_automation() {
                (
                    format!("device:{}", identity.device_id),
                    None,
                    identity.principal_class.clone(),
                    identity.client_kind.clone(),
                    &["automation"][..],
                )
            } else {
                (
                    format!("device:{}", identity.device_id),
                    None,
                    identity.principal_class.clone(),
                    identity.client_kind.clone(),
                    &["cli", "acp"][..],
                )
            }
        } else {
            (
                format!("principal:{}", authenticated.principal.user_id),
                None,
                "human".to_owned(),
                "cli".to_owned(),
                &["cli", "acp"][..],
            )
        };
    let client_kind = requested_kind.unwrap_or(&default_kind);
    if !allowed_kinds.contains(&client_kind) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let seed = requested.unwrap_or(&fallback);
    // Namespace untrusted client labels by the authenticated account. A client
    // may reconnect under its own stable label, but cannot collide with or
    // release another account's logical client by guessing that label.
    let client_id = format!(
        "client-{}",
        crate::admin::hex_sha256(format!("{}:{seed}", authenticated.principal.user_id).as_bytes())
    );
    Ok(WebSocketCapacityIdentity {
        client_id,
        user_id: (principal_class == "human").then(|| authenticated.principal.user_id.clone()),
        principal_class,
        session_id,
        client_kind: client_kind.to_owned(),
    })
}

pub(super) async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Query(query): Query<WebSocketQuery>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let capacity =
        match websocket_capacity_identity(state.product_auth_enabled, &authenticated, &query) {
            Ok(capacity) => capacity,
            Err(status) => return status.into_response(),
        };
    if let Err(status) = sync_dataset::socket_check(
        &state.service_id,
        &authenticated.principal,
        matches!(capacity.client_kind.as_str(), "browser" | "native_shell"),
        query.dataset.as_deref(),
    ) {
        return (
            status,
            "Product client or dataset changed; update and reload before reconnecting",
        )
            .into_response();
    }
    // Negotiation proves this peer actually validated the dataset; a retained
    // Controller that ignores the query cannot silently admit a new client.
    let ws = if query.dataset.is_some() {
        if !sync_dataset::offers_protocol(&headers) {
            return StatusCode::UPGRADE_REQUIRED.into_response();
        }
        ws.protocols([sync_dataset::SUBPROTOCOL])
    } else {
        ws
    };
    let principal = match authorize_ws_upgrade(
        &headers,
        peer,
        &state.public_origins,
        Some(authenticated.principal),
    ) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    let lazy_bootstrap = query.bootstrap.as_deref() == Some("lazy");
    let via_cookie = state.product_auth_enabled
        && crate::product_auth::user_cookie_token(&headers).is_some()
        && crate::product_auth::bearer_token(&headers).is_none();
    let cookie_token = crate::product_auth::user_cookie_token(&headers);
    let bearer = crate::product_auth::bearer_token(&headers);
    let device_identity = authenticated.device_identity;
    ws.on_upgrade(move |socket| {
        handle_ws(
            socket,
            state,
            lazy_bootstrap,
            WebSocketAuthentication {
                principal,
                via_cookie,
                cookie_token,
                bearer,
                device_identity,
                capacity,
            },
        )
    })
}

/// An Owner sees every session, so a broadcast usually reaches its socket as
/// the shared pre-serialized frame. `workspace-order` and `drafts` are stored
/// for every user at once (`{user_id: ...}`) and must still be narrowed to the
/// receiving principal's own value; the raw map would replace the client's
/// array or reveal another user's document titles.
pub(super) fn fanout_sends_shared_frame(principal: &ProductPrincipal, outbound: &Outbound) -> bool {
    principal.sees_every_session()
        && !matches!(outbound, Outbound::AccountUsageChanged { .. })
        && !matches!(outbound, Outbound::SyncPatch { state, .. } if principal_owned_sync_state(state))
}

pub(super) fn project_outbound(
    hub: &Hub,
    principal: &ProductPrincipal,
    visible: &std::collections::HashSet<String>,
    message: Outbound,
) -> Option<Outbound> {
    match message {
        Outbound::AccountUsageChanged { ref owner_user_id } => {
            (principal.user_id == *owner_user_id).then_some(message)
        }
        Outbound::Sessions { sessions } => Some(Outbound::Sessions {
            sessions: sessions
                .into_iter()
                .filter(|meta| principal.can_see(meta.owner_user_id.as_deref()))
                .collect(),
        }),
        Outbound::Snapshot { ref session_id, .. }
        | Outbound::ConfigOptions { ref session_id, .. }
        | Outbound::CallApproval { ref session_id, .. } => {
            session_is_visible(hub, principal, session_id).then_some(message)
        }
        Outbound::Event { ref envelope } => {
            session_is_visible(hub, principal, &envelope.session_id).then_some(message)
        }
        Outbound::Error {
            session_id: Some(ref session_id),
            ..
        } => session_is_visible(hub, principal, session_id).then_some(message),
        // An addressed refusal names an unguessable client message id and a
        // generic reason; the session may already be gone, so it cannot be gated
        // on visibility. Only the device that minted the id acts on it.
        Outbound::CommandResult { .. }
        | Outbound::Error {
            session_id: None, ..
        }
        | Outbound::Machines { .. }
        | Outbound::AuthSession { .. }
        | Outbound::ClientCapacity { .. }
        | Outbound::Ping
        | Outbound::ConnectionProbe { .. }
        | Outbound::BootstrapComplete
        | Outbound::Usage { .. }
        | Outbound::Settings { .. } => Some(message),
        Outbound::SyncPatch {
            state,
            version,
            value,
            confirmed,
            resync,
        } => {
            if state == "title" || state == "order" {
                Some(Outbound::SyncPatch {
                    value: project_sync_value(&state, value, visible),
                    state,
                    version,
                    confirmed,
                    resync,
                })
            } else if state == "workspace-order" {
                Some(Outbound::SyncPatch {
                    value: value
                        .get(&principal.user_id)
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!([])),
                    state,
                    version,
                    confirmed,
                    resync,
                })
            } else if state == "drafts" || state == "preferences" {
                // Another user's document change is not even announced.
                value
                    .get(&principal.user_id)
                    .cloned()
                    .map(|value| Outbound::SyncPatch {
                        value,
                        state,
                        version,
                        confirmed,
                        resync,
                    })
            } else if state == "folders" {
                Some(Outbound::SyncPatch {
                    value: project_folders_value(value, visible, |owner| principal.can_see(owner)),
                    state,
                    version,
                    confirmed,
                    resync,
                })
            } else if let Some(session_id) = session_id_from_sync_state(&state) {
                session_is_visible(hub, principal, session_id).then_some(Outbound::SyncPatch {
                    state,
                    version,
                    value,
                    confirmed,
                    resync,
                })
            } else {
                Some(Outbound::SyncPatch {
                    state,
                    version,
                    value,
                    confirmed,
                    resync,
                })
            }
        }
    }
}

pub(super) fn ws_close_auth_required() -> Message {
    Message::Close(Some(CloseFrame {
        code: WS_AUTH_REQUIRED_CLOSE_CODE,
        reason: "auth_required".into(),
    }))
}

pub(super) fn ws_close_capacity_required() -> Message {
    Message::Close(Some(CloseFrame {
        code: WS_CAPACITY_REQUIRED_CLOSE_CODE,
        reason: "active_client_capacity".into(),
    }))
}

pub(super) async fn principal_still_valid(
    state: &AppState,
    via_cookie: bool,
    cookie_token: Option<&str>,
    bearer: Option<&str>,
    device_identity: Option<&crate::client_auth::DeviceAccessIdentity>,
    expected: &ProductPrincipal,
) -> bool {
    if !state.product_auth_enabled {
        return true;
    }
    let Some(store) = state.store.as_ref() else {
        return false;
    };
    let current = if let Some(identity) = device_identity {
        let Some(token) = bearer else {
            return false;
        };
        if !state
            .device_access
            .token_still_valid(token, identity, auth_now_ms())
        {
            return false;
        }
        if !identity.is_automation()
            && !store
                .list_user_devices_for_user(&identity.user_id)
                .await
                .is_ok_and(|devices| {
                    devices.iter().any(|device| {
                        device.id == identity.device_id && device.revoked_at_ms.is_none()
                    })
                })
        {
            state.device_access.revoke_device(&identity.device_id);
            return false;
        }
        match store.user_by_id(&identity.user_id).await {
            Ok(Some(user)) if user.disabled_at_ms.is_none() => {
                Some(product_principal(&state.hub, &user))
            }
            _ => None,
        }
    } else if via_cookie {
        let Some(token) = cookie_token else {
            return false;
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            format!("{}={token}", crate::product_auth::USER_SESSION_COOKIE)
                .parse()
                .unwrap_or_else(|_| header::HeaderValue::from_static("")),
        );
        match product_session_and_user_from_store_cookie(store, &headers).await {
            Some((session, user))
                if ensure_product_session_fresh(store, &session, &state.product_authentication)
                    .await
                    .is_ok() =>
            {
                Some(product_principal(&state.hub, &user))
            }
            _ => None,
        }
    } else if let Some(token) = bearer {
        product_from_bearer(store, &state.hub, token).await
    } else {
        None
    };
    current.is_some_and(|principal| {
        principal.user_id == expected.user_id && !principal.username.is_empty()
    })
}

pub(super) async fn handle_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    lazy_bootstrap: bool,
    authentication: WebSocketAuthentication,
) {
    let (mut sink, mut stream) = socket.split();

    let capacity_policy = state.product_authentication.capacity;
    let enforce_capacity =
        capacity_policy.enforcement == crate::auth_plugins::CapacityEnforcement::Enforce;
    let channel_count = match state.capacity_runtime.open_channel(
        &authentication.capacity.client_id,
        capacity_policy.websocket_channels_per_client,
        enforce_capacity,
    ) {
        Ok(count) => count,
        Err(count) => {
            let message = active_capacity_message(
                "channel_limit",
                None,
                None,
                None,
                Some(count),
                capacity_policy.websocket_channels_per_client,
            );
            let _ = send_json(&mut sink, &message).await;
            let _ = sink.send(ws_close_capacity_required()).await;
            return;
        }
    };
    if !enforce_capacity && channel_count > capacity_policy.websocket_channels_per_client {
        tracing::warn!(
            client_id = authentication.capacity.client_id,
            channel_count,
            limit = capacity_policy.websocket_channels_per_client,
            "observed logical-client WebSocket channel overflow"
        );
    }
    let _capacity_channel = ActiveCapacityChannel {
        runtime: state.capacity_runtime.clone(),
        store: state.store.clone(),
        client_id: authentication.capacity.client_id.clone(),
        reservation_ms: capacity_policy.reservation_ms,
        per_user_limit: capacity_policy.active_clients_per_user,
    };

    // A cookie session must pass its current idle, Passkey, and primary-login
    // deadlines before any transcript or Machine snapshot is disclosed. Push
    // the required proof to the client first, then close without bootstrapping
    // when the session is no longer fresh.
    let initial_auth_deadline_ms = if let Some(token) = authentication.cookie_token.as_deref() {
        let Some(snapshot) = product_auth_session_message(&state, token).await else {
            let _ = sink.send(ws_close_auth_required()).await;
            return;
        };
        if send_json(&mut sink, &snapshot.message).await.is_err() {
            return;
        }
        if !snapshot.valid || snapshot.user_id != authentication.principal.user_id {
            let _ = sink.send(ws_close_auth_required()).await;
            return;
        }
        Some(snapshot.next_due_at_ms)
    } else {
        None
    };

    let Some(capacity_fencing_token) = admit_active_client(
        &state,
        &mut sink,
        &mut stream,
        &authentication,
        channel_count,
        enforce_capacity,
    )
    .await
    else {
        return;
    };
    let WebSocketAuthentication {
        principal,
        via_cookie,
        cookie_token,
        bearer,
        device_identity,
        capacity,
    } = authentication;

    // Subscribe BEFORE snapshotting so no event slips through the gap; the
    // client dedups by (session_id, seq), so a brief overlap is harmless.
    let rx = state.hub.subscribe();
    let shutdown = state.shutdown.clone();

    let machine_snapshot = match state.machine_snapshots.connect_message().await {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            // Never project an unavailable registry as an empty one: the
            // browser keeps its last authoritative snapshot across reconnects.
            tracing::warn!(%error, "loading Machine WebSocket bootstrap");
            None
        }
    };
    for message in connect_bootstrap(&state.hub, lazy_bootstrap, &principal, machine_snapshot) {
        if send_json(&mut sink, &message).await.is_err() {
            return;
        }
    }
    // Fan-out task: broadcast events → this socket, plus a periodic app-level
    // heartbeat (Outbound::Ping) so a client can detect a HALF-OPEN socket that
    // never fires `onclose` (see Outbound::Ping). Per-client interval — a failed
    // heartbeat send reaps a dead client here too.
    let fanout_identity = ClientFanout {
        state: Arc::clone(&state),
        principal: principal.clone(),
        via_cookie,
        cookie: cookie_token.clone(),
        bearer: bearer.clone(),
        device: device_identity.clone(),
        initial_auth_deadline_ms,
        capacity_store: state.store.clone(),
        capacity_runtime: state.capacity_runtime.clone(),
        capacity_client_id: capacity.client_id.clone(),
        capacity_fencing_token,
        capacity_lease_ms: capacity_policy.active_lease_ms,
        capacity_heartbeat_ms: capacity_policy.heartbeat_ms,
        capacity_channel_limit: capacity_policy.websocket_channels_per_client,
    };
    let (direct_tx, direct_rx) =
        tokio::sync::mpsc::unbounded_channel::<Option<ProductAuthSessionSnapshot>>();
    let (probe_tx, probe_rx) = tokio::sync::mpsc::channel::<u64>(1);
    let mut fanout = tokio::spawn(run_client_fanout(
        fanout_identity,
        sink,
        rx,
        shutdown,
        direct_rx,
        probe_rx,
    ));

    // Edit-holds this connection set (session_id → message id + lease epoch). The
    // editing hold is GLOBAL server state, so a client that disconnects mid-edit
    // would otherwise leave the head pinned and stall the queue forever. We
    // track what this socket held and release it after a short reload grace.
    let mut held: HashMap<String, (String, u64)> = HashMap::new();
    let mut wait_for_auth_close = false;

    // Inbound command loop.
    loop {
        tokio::select! {
            _ = &mut fanout => break,
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if !principal_still_valid(
                            &state,
                            via_cookie,
                            cookie_token.as_deref(),
                            bearer.as_deref(),
                            device_identity.as_ref(),
                            &principal,
                        )
                        .await
                        {
                            tracing::info!(reason = "expired_or_step_up_required", "ws_rejected");
                            let _ = direct_tx.send(None);
                            wait_for_auth_close = true;
                            break;
                        }
                        let command = serde_json::from_str::<Inbound>(&text);
                        if let Ok(Inbound::ConnectionProbe { nonce }) = &command {
                            // Already authenticated above. Reply only on this socket;
                            // probes do not touch idle activity or the event log.
                            let _ = probe_tx.try_send(*nonce);
                            continue;
                        }
                        if matches!(command, Ok(Inbound::AuthActivity)) {
                            if let (Some(store), Some(token)) =
                                (state.store.as_ref(), cookie_token.as_deref())
                            {
                                let token_hash = crate::admin::hex_sha256(token.as_bytes());
                                if let Err(error) = store
                                    .touch_user_session_activity(&token_hash, auth_now_ms())
                                    .await
                                {
                                    tracing::warn!(%error, "touching user session activity");
                                }
                                let direct = product_auth_session_message(&state, token).await;
                                let _ = direct_tx.send(direct);
                            }
                            continue;
                        }
                        let command_trace = Observability::begin_command_trace(&text);
                        let accepted = handle_command(
                            &state,
                            &principal,
                            &text,
                            &mut held,
                            command_trace.as_ref(),
                        );
                        if let Some(trace) = command_trace {
                            state.observability.finish_command_trace(&principal.user_id, trace, accepted);
                        }
                    }
                    // Other frame types (ping/pong/binary) are ignored.
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                    // Close, transport error, or stream end: tear down.
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                }
            }
        }
    }
    // A service-worker update or iOS WebView restart briefly replaces the socket.
    // Keep the queue pinned while the restored local edit reconnects and reasserts
    // its hold. Epoch matching prevents this old socket's timer from releasing a
    // replacement connection; a genuine abandoned edit drains after the grace.
    for (session_id, (id, epoch)) in held {
        let hub = state.hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(QUEUE_EDIT_RECONNECT_GRACE).await;
            hub.release_queue_editing_if_epoch(&session_id, &id, epoch);
        });
    }
    if wait_for_auth_close {
        let _ = fanout.await;
    } else {
        fanout.abort();
    }
}

/// Claims this logical client's active-client lease, waiting in line while
/// capacity is full. `None` means the socket must close; `Some(token)` admits
/// it, with no fencing token for system principals that hold no lease.
async fn admit_active_client(
    state: &Arc<AppState>,
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    stream: &mut futures::stream::SplitStream<WebSocket>,
    authentication: &WebSocketAuthentication,
    channel_count: u32,
    enforce_capacity: bool,
) -> Option<Option<i64>> {
    let WebSocketAuthentication {
        principal,
        via_cookie,
        cookie_token,
        bearer,
        device_identity,
        capacity,
    } = authentication;
    let capacity_policy = state.product_authentication.capacity;
    if capacity.principal_class == "system" {
        return Some(None);
    }
    let Some(store) = state.store.as_ref() else {
        let message = active_capacity_message(
            "unavailable",
            None,
            None,
            Some(capacity_policy.reservation_ms),
            Some(channel_count),
            capacity_policy.websocket_channels_per_client,
        );
        let _ = send_json(sink, &message).await;
        let _ = sink.send(ws_close_capacity_required()).await;
        return None;
    };
    let mut capacity_shutdown = state.shutdown.clone();
    loop {
        if !principal_still_valid(
            state,
            *via_cookie,
            cookie_token.as_deref(),
            bearer.as_deref(),
            device_identity.as_ref(),
            principal,
        )
        .await
        {
            let _ = sink.send(ws_close_auth_required()).await;
            return None;
        }
        let claim = crate::store::ActiveClientClaim {
            client_id: capacity.client_id.clone(),
            user_id: capacity.user_id.clone(),
            principal_class: capacity.principal_class.clone(),
            session_id: capacity.session_id.clone(),
            client_kind: capacity.client_kind.clone(),
            now_ms: auth_now_ms(),
            lease_ms: capacity_policy.active_lease_ms,
            reservation_ms: capacity_policy.reservation_ms,
            per_user_limit: capacity_policy.active_clients_per_user,
            service_limit: capacity_policy.active_clients_service,
            automation_limit: state.product_authentication.automation.active_clients,
            enforce: enforce_capacity,
        };
        match store.claim_active_client(&claim).await {
            Ok(crate::store::ActiveClientAdmission::Granted {
                lease,
                capacity: counts,
                observed_overflow,
            }) => {
                state
                    .capacity_runtime
                    .record_fence(&capacity.client_id, lease.fencing_token);
                if observed_overflow {
                    tracing::warn!(
                        client_id = capacity.client_id,
                        user_id = capacity.user_id,
                        "observed active-client capacity overflow"
                    );
                }
                let message = active_capacity_message(
                    "active",
                    Some(&counts),
                    None,
                    None,
                    Some(channel_count),
                    capacity_policy.websocket_channels_per_client,
                );
                if send_json(sink, &message).await.is_err() {
                    return None;
                }
                break Some(Some(lease.fencing_token));
            }
            Ok(crate::store::ActiveClientAdmission::Waiting {
                position,
                retry_after_ms,
                capacity: counts,
            }) => {
                let message = active_capacity_message(
                    "waiting",
                    Some(&counts),
                    Some(position),
                    Some(retry_after_ms),
                    Some(channel_count),
                    capacity_policy.websocket_channels_per_client,
                );
                if send_json(sink, &message).await.is_err() {
                    return None;
                }
                tokio::select! {
                    () = state.capacity_runtime.changed.notified() => {}
                    () = tokio::time::sleep(std::time::Duration::from_millis(
                        u64::try_from(retry_after_ms.max(1_000)).unwrap_or(1_000),
                    )) => {}
                    changed = capacity_shutdown.changed() => {
                        if changed.is_err() || *capacity_shutdown.borrow() {
                            return None;
                        }
                    }
                    frame = stream.next() => match frame {
                        Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
                        Some(Ok(_)) => {}
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, client_id = capacity.client_id, "active-client admission");
                let message = active_capacity_message(
                    "unavailable",
                    None,
                    None,
                    Some(capacity_policy.reservation_ms),
                    Some(channel_count),
                    capacity_policy.websocket_channels_per_client,
                );
                let _ = send_json(sink, &message).await;
                let _ = sink.send(ws_close_capacity_required()).await;
                return None;
            }
        }
    }
}

pub(super) fn handle_command(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    text: &str,
    held: &mut HashMap<String, (String, u64)>,
    command_trace: Option<&crate::runtime_trace::SpanTimer>,
) -> bool {
    let cmd: Inbound = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "bad inbound command");
            state
                .hub
                .broadcast_error(None, format!("bad inbound command: {e}"));
            return false;
        }
    };
    // Capture session_id ahead of the match for error attribution. Most
    // commands carry one; NewSession doesn't (the session id is assigned
    // by the daemon after success).
    let session_id_for_err: Option<String> = command_session_id(&cmd);
    // A client-authored prompt or draft carries its `cmid` from a durable
    // outbox that is replayed after every reconnect. A refusal that can never
    // change must name that id, otherwise the device retries it forever and the
    // user only ever sees an unaddressed toast.
    let authored_cmid: Option<String> = command_authored_cmid(&cmd);
    if refuse_command(state, principal, &cmd, &session_id_for_err, &authored_cmid) {
        return false;
    }
    // Serialize runtime admission against Provider lifecycle changes. Holding
    // this read guard through the command match closes the check-then-dispatch
    // race with uninstall's active-turn snapshot.
    let provider_prompt_key = session_id_for_err
        .as_deref()
        .filter(|_| plugin_uninstall::requires_runtime_fence(&cmd))
        .and_then(|sid| provider_fence_key_for_session(&state.hub, sid));
    let plugin_prompt_fence = provider_prompt_key
        .as_ref()
        .map(|_| state.plugin_lifecycle_fences.read());
    if let Some(refusal) = provider_prompt_key
        .as_ref()
        .zip(plugin_prompt_fence.as_ref())
        .and_then(|(key, fences)| fences.get(key))
        .and_then(|fence| provider_fence_refusal(*fence))
    {
        state
            .hub
            .broadcast_error(session_id_for_err, refusal.to_owned());
        return false;
    }
    let result = match cmd {
        Inbound::AuthActivity => Ok(()),
        Inbound::ConnectionProbe { .. } => Err("connection probes require a WebSocket".to_owned()),
        Inbound::NewSession { .. } => Err(
            "legacy WebSocket session creation is disabled; use POST /api/sessions with a connected Machine"
                .to_owned(),
        ),
        Inbound::Prompt {
            session_id,
            text,
            content,
        } => match prompt_command(state, session_id, text, content) {
            ControlFlow::Break(done) => return done,
            ControlFlow::Continue(result) => result,
        },
        Inbound::Cancel { session_id } => state.supervisor.send(&session_id, AgentCommand::Cancel),
        Inbound::CancelSubagent {
            session_id,
            subagent_session_id,
        } => {
            if subagent_session_id.is_empty() || subagent_session_id.len() > 256 {
                Err("invalid subagent".to_owned())
            } else {
                state.supervisor.send(
                    &session_id,
                    AgentCommand::CancelSubagent {
                        subagent_session_id,
                    },
                )
            }
        }
        Inbound::CancelSubmitted { session_id, cmid } => {
            if state.hub.remove_queued_by_cmid(&session_id, &cmid) {
                // Explicit acknowledgement for the ACP bridge. Absence of this
                // event means the prompt crossed the queue→dispatch boundary;
                // the bridge then waits for its cmid echo and cancels the now-
                // active turn, avoiding both a false cancellation response and
                // interruption of another surface's turn.
                state.hub.push(
                    &session_id,
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "cowboy_prompt_cancelled",
                            "cmid": cmid,
                        }),
                    },
                );
            }
            Ok(())
        }
        Inbound::Permission {
            session_id,
            request_id,
            option_id,
        } => state.supervisor.send(
            &session_id,
            AgentCommand::Permission {
                request_id,
                option_id,
            },
        ),
        Inbound::DeleteSession { session_id } => session_trash::delete_now(state, &session_id),
        Inbound::TrashItems {
            session_ids,
            folder_ids,
        } => session_trash::trash(state, principal, &folder_ids, &session_ids),
        Inbound::RestoreItems {
            session_ids,
            folder_ids,
        } => session_trash::restore(state, principal, &folder_ids, &session_ids),
        Inbound::PurgeItems {
            session_ids,
            folder_ids,
        } => session_trash::purge(state, principal, &folder_ids, &session_ids),
        Inbound::RenameSession { session_id, title } => {
            // Empty title is a UI bug; reject server-side so the toast lands.
            let trimmed = title.trim().to_owned();
            if trimmed.is_empty() {
                Err("title cannot be empty".to_owned())
            } else {
                state.hub.rename_session(&session_id, trimmed);
                Ok(())
            }
        }
        Inbound::Sync {
            state: sync_state,
            id,
            name,
            args,
        } => apply_inbound_sync(state, principal, &sync_state, id, &name, &args),
        Inbound::SetSessionAutoResume { .. }
        | Inbound::ResumeTurn { .. }
        | Inbound::SetSetting { .. } => Ok(()),
        Inbound::SetPaused { session_id, paused } => {
            state.hub.set_paused(&session_id, paused);
            Ok(())
        }
        Inbound::RetryTurn { session_id } => {
            state.supervisor.prepare_session(&session_id).map(|_| {
                state.hub.retry_turn(&session_id);
            })
        }
        Inbound::SetConfigOption {
            session_id,
            config_id,
            value,
        } => config_revert::apply(state, &session_id, config_id, value, true),
        // Opening is inspection; stopped sessions remain stopped until work is
        // submitted. Live ownership is reconciled without a throwaway prompt.
        // Opening a trashed session is inspection only; it never wakes.
        Inbound::OpenSession { session_id } if state.hub.session_is_trashed(&session_id) => Ok(()),
        Inbound::OpenSession { session_id } => open_session_command(state, session_id),
        Inbound::ResetSession { session_id } => match reset_session_command(state, session_id) {
            ControlFlow::Break(done) => return done,
            ControlFlow::Continue(result) => result,
        },

        // --- Server-authoritative queue + drafts ------------------------------
        // These mutate Hub state, which broadcasts the new queue/drafts to every
        // terminal. They never fail in a way worth a toast, so all return Ok.
        Inbound::Submit {
            session_id,
            text,
            content,
            cmid,
            force,
            front,
        } => match submit_command(state, principal, command_trace, session_id, text, content, cmid, force, front) {
            ControlFlow::Break(done) => return done,
            ControlFlow::Continue(result) => result,
        },
        Inbound::RemoveQueued {
            session_id,
            id,
            cmid,
        } => {
            if !state.hub.remove_queued(&session_id, &id) {
                report_stale_row(state, &session_id, cmid.as_deref());
            }
            Ok(())
        }
        Inbound::EditQueued {
            session_id,
            id,
            text,
            content,
            cmid,
        } => {
            if !state.hub.edit_queued(&session_id, &id, text, content) {
                report_stale_row(state, &session_id, cmid.as_deref());
            }
            Ok(())
        }
        Inbound::ClearQueue { session_id } => {
            state.hub.clear_queue(&session_id);
            Ok(())
        }
        Inbound::RequestSendQueued { session_id, id } => {
            state.hub.request_send_queued(&session_id, &id);
            Ok(())
        }
        Inbound::ForcePushQueued { session_id, id } => {
            // Interrupt the running turn so the promoted prompt runs next; on an
            // idle session there's nothing to cancel, so just send it now.
            state.hub.request_send_queued(&session_id, &id); // promote to front either way
            if matches!(
                state.hub.status(&session_id),
                Some(Status::Busy | Status::Starting)
            ) {
                force_cancel_with_watchdog(state, &session_id)
            } else {
                Ok(())
            }
        }
        Inbound::QueuedToDraft { session_id, id } => {
            state.hub.queued_to_draft(&session_id, &id);
            Ok(())
        }
        Inbound::SetQueueEditing { session_id, id } => set_queue_editing_command(state, held, session_id, id),
        Inbound::AddDraft {
            session_id,
            text,
            content,
            cmid,
        } => {
            state.hub.add_draft(&session_id, text, content, cmid);
            Ok(())
        }
        Inbound::EditDraft {
            session_id,
            id,
            text,
            content,
            cmid,
        } => {
            if !state.hub.edit_draft(&session_id, &id, text, content) {
                report_stale_row(state, &session_id, cmid.as_deref());
            }
            Ok(())
        }
        Inbound::RemoveDraft {
            session_id,
            id,
            cmid,
        } => {
            if !state.hub.remove_draft(&session_id, &id) {
                report_stale_row(state, &session_id, cmid.as_deref());
            }
            Ok(())
        }
        Inbound::ClearDrafts { session_id } => {
            state.hub.clear_drafts(&session_id);
            Ok(())
        }
        Inbound::ActivateDraft { session_id, id, cmid } => {
            state.hub.activate_draft(&session_id, &id, cmid);
            Ok(())
        }
        Inbound::ActivateAllDrafts { session_id } => {
            state.hub.activate_all_drafts(&session_id);
            Ok(())
        }
        Inbound::MoveDraft {
            session_id,
            id,
            to_session,
        } => {
            state.hub.move_draft(&session_id, &id, &to_session);
            Ok(())
        }
        Inbound::ScheduleDraft {
            session_id,
            id,
            cmid,
            text,
            content,
            fire_at_ms,
            delivery,
        } => {
            state
                .hub
                .schedule_draft(&session_id, id, cmid, text, content, fire_at_ms, delivery);
            Ok(())
        }
        Inbound::UnscheduleDraft { session_id, id } => {
            state.hub.unschedule_draft(&session_id, &id);
            Ok(())
        }
        Inbound::ReorderSessions { order } => apply_visible_reorder(state, principal, &order),
        Inbound::ReorderQueue { session_id, order } => {
            state.hub.reorder_queue(&session_id, &order);
            Ok(())
        }
        Inbound::ReorderDrafts { session_id, order } => {
            state.hub.reorder_drafts(&session_id, &order);
            Ok(())
        }
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "command failed");
        state
            .hub
            .broadcast_error(session_id_for_err, format!("command failed: {e}"));
        return false;
    }
    true
}

pub(super) fn apply_inbound_sync(
    state: &AppState,
    principal: &ProductPrincipal,
    sync_state: &str,
    id: String,
    name: &str,
    args: &serde_json::Value,
) -> Result<(), String> {
    if sync_state == "order" {
        if !principal.can_reorder() {
            return Err("viewers cannot reorder sessions".to_owned());
        }
        let submitted = args
            .get("order")
            .and_then(serde_json::Value::as_array)
            .ok_or("reorder: missing order")?;
        let filtered = filter_submitted_order(state, principal, submitted);
        return state.hub.sync_apply(
            sync_state,
            id,
            name,
            &serde_json::json!({ "order": filtered }),
        );
    }
    if sync_state == "workspace-order" {
        if !principal.can_reorder() {
            return Err("viewers cannot organize the workspace".to_owned());
        }
        return state.hub.sync_apply_as(
            &crate::session_folders::FolderActor {
                user_id: Some(principal.user_id.clone()),
                sees_all: false,
            },
            sync_state,
            id,
            name,
            args,
        );
    }
    if sync_state == "folders" {
        if !principal.can_reorder() {
            return Err("viewers cannot organize sessions".to_owned());
        }
        let actor = folder_actor(state, principal);
        if name == "place" {
            // Only sessions the principal may mutate get filed; the rest of the
            // batch is dropped rather than rejected, so one foreign id cannot
            // block a whole move.
            let session_ids = args
                .get("session_ids")
                .and_then(serde_json::Value::as_array)
                .ok_or("place: missing session_ids")?;
            let allowed: Vec<String> = session_ids
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|session_id| {
                    state.hub.session_info(session_id).is_some_and(|info| {
                        principal.can_mutate(info.meta.owner_user_id.as_deref())
                    })
                })
                .map(str::to_owned)
                .collect();
            return state.hub.sync_apply_as(
                &actor,
                sync_state,
                id,
                name,
                &serde_json::json!({
                    "session_ids": allowed,
                    "folder": args.get("folder").cloned().unwrap_or(serde_json::Value::Null),
                }),
            );
        }
        return state.hub.sync_apply_as(&actor, sync_state, id, name, args);
    }
    if sync_state == "title" {
        let session_id = args
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .ok_or("rename: missing session_id")?;
        let owner = state
            .hub
            .session_info(session_id)
            .and_then(|info| info.meta.owner_user_id);
        if !principal.can_mutate(owner.as_deref()) {
            return Err("not allowed to rename this session".to_owned());
        }
        return state.hub.sync_apply(sync_state, id, name, args);
    }
    if let Some(session_id) = session_id_from_sync_state(sync_state) {
        let owner = state
            .hub
            .session_info(session_id)
            .and_then(|info| info.meta.owner_user_id);
        if !principal.can_mutate(owner.as_deref()) {
            return Err("not allowed to mutate this session".to_owned());
        }
        return state.hub.sync_apply(sync_state, id, name, args);
    }
    Err(format!("unknown sync mutation {sync_state}/{name}"))
}

pub(super) async fn send_json<S, T>(sink: &mut S, msg: &T) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
    T: Serialize,
{
    send_json_with_timeout(sink, msg, WEBSOCKET_FRAME_SEND_TIMEOUT).await
}

pub(super) async fn send_frame<S>(sink: &mut S, frame: &FanoutFrame) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = frame.json().map_err(|_| ())?;
    tokio::time::timeout(
        WEBSOCKET_FRAME_SEND_TIMEOUT,
        sink.send(Message::Text(text.to_owned().into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

pub(super) async fn send_json_with_timeout<S, T>(
    sink: &mut S,
    msg: &T,
    timeout: std::time::Duration,
) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
    T: Serialize,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    tokio::time::timeout(timeout, sink.send(Message::Text(text.into())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

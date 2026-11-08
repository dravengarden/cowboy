//! The viewer's WebSocket (`GET .../browser/stream`): a product route with
//! the owner, Origin and account checks, continuing authorization exactly
//! like `/ws`, paired with the Machine's half of the stream by a one-time
//! token. Binary frames are relayed verbatim both ways and never inspected;
//! text frames carry only the viewer's lease renewals and resize requests.

use super::super::super::*;
use super::coordinator::Pairing;
use crate::machine_protocol::machine_browser::{
    Command, InstanceRef, LEASE_EXPIRY_MS, MAX_FRAME_BYTES, State as BrowserState, StateEvent,
    Viewport,
};
use crate::provider_account::ProviderAccountId;
use serde_json::json;

/// Subprotocol the viewer must offer; the device proof travels beside it.
pub(in crate::server) const SUBPROTOCOL: &str = "cowboy-browser-v1";
const ATTACH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
const FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(25);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct ViewerQuery {
    #[serde(default)]
    takeover: bool,
    width: u32,
    height: u32,
    scale: u8,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ViewerControl {
    Lease,
    Resize { width: u32, height: u32 },
}

fn offers_protocol(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .any(|value| {
            value
                .to_str()
                .is_ok_and(|value| value.split(',').any(|token| token.trim() == SUBPROTOCOL))
        })
}

fn refuse(code: &'static str) -> Response {
    let status = match code {
        "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        "account_unavailable" => StatusCode::NOT_FOUND,
        "invalid_viewport" => StatusCode::BAD_REQUEST,
        _ => StatusCode::CONFLICT,
    };
    (status, Json(json!({"error":{"code":code}}))).into_response()
}

#[allow(clippy::too_many_lines)] // One admission sequence, in the order the design fixes.
pub(in crate::server) async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Query(query): Query<ViewerQuery>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if auth
        .device_identity
        .as_ref()
        .is_some_and(crate::client_auth::DeviceAccessIdentity::is_automation)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let principal = match super::super::super::client_ws::authorize_ws_upgrade(
        &headers,
        peer,
        &state.public_origins,
        Some(auth.principal.clone()),
    ) {
        Ok(principal) => principal,
        Err(status) => return status.into_response(),
    };
    if !offers_protocol(&headers) {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }
    let owner = principal.user_id.clone();
    if let Err(code) = super::admit(&state, &owner, &account).await {
        return refuse(code);
    }
    let viewport = Viewport {
        width: query.width,
        height: query.height,
        scale: query.scale,
    };
    if viewport.validate().is_err() {
        return refuse("invalid_viewport");
    }
    let Some(observed) = state.browsers.state(account.as_str()) else {
        return refuse("browser_not_running");
    };
    if !matches!(
        observed.event.state,
        BrowserState::Detached | BrowserState::Attached
    ) {
        return refuse("browser_not_running");
    }
    if observed.event.state == BrowserState::Attached && !query.takeover {
        return refuse("browser_attached");
    }
    let Some((connection, _)) = state
        .machine_control
        .connection_protocol(&observed.machine_id)
    else {
        return refuse("machine_unavailable");
    };
    let Ok(token) = super::super::super::machine_connection::random_machine_token() else {
        return refuse("unavailable");
    };
    let attachment_id = format!(
        "att-{}",
        super::super::super::machine_connection::random_machine_token()
            .map(|value| value[..22].to_owned())
            .unwrap_or_else(|_| super::now_ms().to_string())
    );
    let (deliver, delivered) = tokio::sync::oneshot::channel();
    // The Machine opens its half as soon as it attaches, so the token must
    // be known before the command leaves.
    state.browsers.register_pairing(
        token.clone(),
        Pairing {
            created: std::time::Instant::now(),
            connection: connection.clone(),
            deliver,
        },
    );
    let instance = observed.instance();
    let attached = state
        .machine_control
        .browser_command(
            &connection,
            super::request_id("browser-attach"),
            Command::Attach {
                instance: instance.clone(),
                attachment_id: attachment_id.clone(),
                stream_token: token.clone(),
                viewport,
            },
            ATTACH_TIMEOUT,
        )
        .await;
    let attached = match attached {
        Ok(event)
            if event.state == BrowserState::Attached
                && event.attachment_id.as_deref() == Some(attachment_id.as_str()) =>
        {
            event
        }
        Ok(event) => {
            state.browsers.forget_pairing(&token);
            tracing::info!(state = ?event.state, reason = ?event.reason, "browser attach refused");
            return refuse("browser_attach_refused");
        }
        Err(error) => {
            state.browsers.forget_pairing(&token);
            tracing::warn!(%error, "browser attach failed");
            return refuse("browser_attach_failed");
        }
    };
    let via_cookie = state.product_auth_enabled
        && crate::product_auth::user_cookie_token(&headers).is_some()
        && crate::product_auth::bearer_token(&headers).is_none();
    let session = ViewerSession {
        account,
        instance,
        attachment_id,
        connection,
        principal,
        via_cookie,
        cookie_token: crate::product_auth::user_cookie_token(&headers),
        bearer: crate::product_auth::bearer_token(&headers),
        device_identity: auth.device_identity,
        initial: attached,
    };
    ws.protocols([SUBPROTOCOL])
        .on_upgrade(move |socket| serve(socket, state, session, delivered))
}

struct ViewerSession {
    account: ProviderAccountId,
    instance: InstanceRef,
    attachment_id: String,
    connection: crate::machine_control::ConnectionToken,
    principal: ProductPrincipal,
    via_cookie: bool,
    cookie_token: Option<String>,
    bearer: Option<String>,
    device_identity: Option<crate::client_auth::DeviceAccessIdentity>,
    initial: StateEvent,
}

async fn serve(
    socket: WebSocket,
    state: Arc<AppState>,
    session: ViewerSession,
    delivered: tokio::sync::oneshot::Receiver<WebSocket>,
) {
    let mut cancelled = state.browsers.register_viewer(
        session.account.as_str(),
        &session.attachment_id,
        session.instance.incarnation,
    );
    let (mut viewer_sink, mut viewer_stream) = socket.split();
    let end = relay(
        &state,
        &session,
        &mut viewer_sink,
        &mut viewer_stream,
        delivered,
        &mut cancelled,
    )
    .await;
    tracing::info!(end, "browser viewer stream ended");
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        viewer_sink.send(Message::Close(None)),
    )
    .await;
    state
        .browsers
        .unregister_viewer(session.account.as_str(), &session.attachment_id);
    // Tell the Machine this attachment is gone; a takeover's replacement
    // carries another attachment id and is untouched.
    let _ = state
        .machine_control
        .browser_command(
            &session.connection,
            super::request_id("browser-detach"),
            Command::Detach {
                instance: session.instance.clone(),
                attachment_id: session.attachment_id.clone(),
            },
            std::time::Duration::from_secs(5),
        )
        .await;
}

async fn relay<S, R>(
    state: &Arc<AppState>,
    session: &ViewerSession,
    viewer_sink: &mut S,
    viewer_stream: &mut R,
    delivered: tokio::sync::oneshot::Receiver<WebSocket>,
    cancelled: &mut tokio::sync::watch::Receiver<bool>,
) -> &'static str
where
    S: SinkExt<Message> + Unpin,
    R: StreamExt<Item = Result<Message, axum::Error>> + Unpin,
{
    let machine = tokio::select! {
        delivered = tokio::time::timeout(super::coordinator::Coordinator::pairing_timeout(), delivered) => delivered,
        _ = cancelled.changed() => return "cancelled_before_pairing",
    };
    let machine = match machine {
        Ok(Ok(machine)) => machine,
        Ok(Err(_)) => return "pairing_dropped",
        Err(_) => return "pairing_timeout",
    };
    let (mut machine_sink, mut machine_stream) = machine.split();
    if send_json(
        viewer_sink,
        &json!({"type":"state","state":session.initial}),
    )
    .await
    .is_err()
    {
        return "viewer_closed";
    }
    let initial_deadline_ms = match session.cookie_token.as_deref() {
        Some(token) => match renew_auth(state, session, viewer_sink, token).await {
            Ok(next_due_at_ms) => next_due_at_ms,
            Err(end) => return end,
        },
        None => i64::MAX,
    };
    relay_frames(
        state,
        session,
        (viewer_sink, viewer_stream),
        (&mut machine_sink, &mut machine_stream),
        cancelled,
        initial_deadline_ms,
    )
    .await
}

/// One select loop owns both hops and every authorization deadline.
async fn relay_frames<S, R, MS, MR>(
    state: &Arc<AppState>,
    session: &ViewerSession,
    (viewer_sink, viewer_stream): (&mut S, &mut R),
    (machine_sink, machine_stream): (&mut MS, &mut MR),
    cancelled: &mut tokio::sync::watch::Receiver<bool>,
    initial_deadline_ms: i64,
) -> &'static str
where
    S: SinkExt<Message> + Unpin,
    R: StreamExt<Item = Result<Message, axum::Error>> + Unpin,
    MS: SinkExt<Message> + Unpin,
    MR: StreamExt<Item = Result<Message, axum::Error>> + Unpin,
{
    let auth_deadline =
        tokio::time::sleep_until(product_auth_deadline_instant(initial_deadline_ms));
    tokio::pin!(auth_deadline);
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.tick().await;
    loop {
        tokio::select! {
            _ = cancelled.changed() => return "replaced",
            message = viewer_stream.next() => {
                match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if bytes.len() > MAX_FRAME_BYTES {
                            return "viewer_frame_too_large";
                        }
                        if tokio::time::timeout(FRAME_TIMEOUT, machine_sink.send(Message::Binary(bytes))).await.is_err() {
                            return "machine_stalled";
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Err(end) = viewer_control(state, session, viewer_sink, &text).await {
                            return end;
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | None => return "viewer_closed",
                    Some(Err(_)) => return "viewer_error",
                }
            }
            message = machine_stream.next() => {
                match message {
                    Some(Ok(Message::Binary(bytes))) => {
                        if bytes.len() > MAX_FRAME_BYTES {
                            return "machine_frame_too_large";
                        }
                        if tokio::time::timeout(FRAME_TIMEOUT, viewer_sink.send(Message::Binary(bytes))).await.is_err() {
                            return "viewer_stalled";
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Text(_))) => return "machine_protocol_error",
                    Some(Ok(Message::Close(_))) | None => return "machine_closed",
                    Some(Err(_)) => return "machine_error",
                }
            }
            () = &mut auth_deadline, if session.via_cookie => {
                let Some(token) = session.cookie_token.as_deref() else {
                    let _ = viewer_sink.send(ws_close_auth_required()).await;
                    return "auth_required";
                };
                match renew_auth(state, session, viewer_sink, token).await {
                    Ok(next_due_at_ms) => {
                        auth_deadline.as_mut().reset(product_auth_deadline_instant(next_due_at_ms));
                    }
                    Err(end) => return end,
                }
            }
            _ = heartbeat.tick() => {
                if !principal_still_valid(
                    state,
                    session.via_cookie,
                    session.cookie_token.as_deref(),
                    session.bearer.as_deref(),
                    session.device_identity.as_ref(),
                    &session.principal,
                )
                .await
                {
                    let _ = viewer_sink.send(ws_close_auth_required()).await;
                    return "auth_required";
                }
                if !state.machine_control.is_current(&session.connection) {
                    return "machine_connection_changed";
                }
                if tokio::time::timeout(FRAME_TIMEOUT, viewer_sink.send(Message::Ping(Vec::new().into()))).await.is_err() {
                    return "viewer_stalled";
                }
            }
        }
    }
}

/// One text frame from the viewer: a lease renewal or a resize request,
/// accepted only while the principal is still valid. `Err` ends the stream.
async fn viewer_control<S>(
    state: &Arc<AppState>,
    session: &ViewerSession,
    viewer_sink: &mut S,
    text: &str,
) -> Result<(), &'static str>
where
    S: SinkExt<Message> + Unpin,
{
    if !principal_still_valid(
        state,
        session.via_cookie,
        session.cookie_token.as_deref(),
        session.bearer.as_deref(),
        session.device_identity.as_ref(),
        &session.principal,
    )
    .await
    {
        let _ = viewer_sink.send(ws_close_auth_required()).await;
        return Err("auth_required");
    }
    let Ok(control) = serde_json::from_str::<ViewerControl>(text) else {
        return Err("viewer_protocol_error");
    };
    match control {
        ViewerControl::Lease => {
            let expires_at_ms = super::now_ms().saturating_add(LEASE_EXPIRY_MS);
            if state
                .machine_control
                .browser_lease(
                    &session.connection,
                    session.instance.clone(),
                    session.attachment_id.clone(),
                    expires_at_ms,
                )
                .is_err()
            {
                return Err("machine_connection_changed");
            }
        }
        ViewerControl::Resize { width, height } => {
            let result = state
                .machine_control
                .browser_command(
                    &session.connection,
                    super::request_id("browser-resize"),
                    Command::Resize {
                        instance: session.instance.clone(),
                        attachment_id: session.attachment_id.clone(),
                        width,
                        height,
                    },
                    std::time::Duration::from_secs(15),
                )
                .await;
            let reply = match result {
                Ok(event) => json!({"type":"state","state":event}),
                Err(error) => {
                    tracing::info!(%error, "browser resize failed");
                    json!({"type":"error","code":"resize_failed"})
                }
            };
            if send_json(viewer_sink, &reply).await.is_err() {
                return Err("viewer_closed");
            }
        }
    }
    Ok(())
}

/// Replays the product session snapshot to the viewer and closes the stream
/// unless it still belongs to the attached principal; `Ok` carries when the
/// next check is due.
async fn renew_auth<S>(
    state: &Arc<AppState>,
    session: &ViewerSession,
    viewer_sink: &mut S,
    token: &str,
) -> Result<i64, &'static str>
where
    S: SinkExt<Message> + Unpin,
{
    let Some(snapshot) = product_auth_session_message(state, token).await else {
        let _ = viewer_sink.send(ws_close_auth_required()).await;
        return Err("auth_required");
    };
    if send_json(viewer_sink, &snapshot.message).await.is_err() {
        return Err("viewer_closed");
    }
    if !snapshot.valid || snapshot.user_id != session.principal.user_id {
        let _ = viewer_sink.send(ws_close_auth_required()).await;
        return Err("auth_required");
    }
    Ok(snapshot.next_due_at_ms)
}

//! The per-client fan-out task: broadcast events and direct product-auth
//! updates to one WebSocket, app-level heartbeats, product-auth revalidation,
//! and active-client capacity lease heartbeats.

use super::*;

/// Identity and capacity lease state the fan-out task revalidates while the
/// client is connected.
pub(super) struct ClientFanout {
    pub(super) state: Arc<AppState>,
    pub(super) principal: ProductPrincipal,
    pub(super) via_cookie: bool,
    pub(super) cookie: Option<String>,
    pub(super) bearer: Option<String>,
    pub(super) device: Option<crate::client_auth::DeviceAccessIdentity>,
    pub(super) initial_auth_deadline_ms: Option<i64>,
    pub(super) capacity_store: Option<Store>,
    pub(super) capacity_runtime: Arc<ActiveCapacityRuntime>,
    pub(super) capacity_client_id: String,
    pub(super) capacity_fencing_token: Option<i64>,
    pub(super) capacity_lease_ms: i64,
    pub(super) capacity_heartbeat_ms: i64,
    pub(super) capacity_channel_limit: u32,
}

// Fan-out task: broadcast events -> this socket, plus a periodic app-level
// heartbeat (Outbound::Ping) so a client can detect a HALF-OPEN socket that
// never fires `onclose` (see Outbound::Ping). Per-client interval - a failed
// heartbeat send reaps a dead client here too.
pub(super) async fn run_client_fanout(
    fanout: ClientFanout,
    mut sink: futures::stream::SplitSink<WebSocket, Message>,
    mut rx: tokio::sync::broadcast::Receiver<Arc<FanoutFrame>>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    mut direct_rx: tokio::sync::mpsc::UnboundedReceiver<Option<ProductAuthSessionSnapshot>>,
    mut probe_rx: tokio::sync::mpsc::Receiver<u64>,
) {
    let ClientFanout {
        state: fanout_state,
        principal: fanout_principal,
        via_cookie,
        cookie: fanout_cookie,
        bearer: fanout_bearer,
        device: fanout_device,
        initial_auth_deadline_ms,
        capacity_store: fanout_capacity_store,
        capacity_runtime: fanout_capacity_runtime,
        capacity_client_id: fanout_capacity_client_id,
        capacity_fencing_token: fanout_capacity_fencing_token,
        capacity_lease_ms,
        capacity_heartbeat_ms,
        capacity_channel_limit,
    } = fanout;
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    let mut capacity_heartbeat = tokio::time::interval(std::time::Duration::from_millis(
        u64::try_from(capacity_heartbeat_ms.max(1_000)).unwrap_or(1_000),
    ));
    let auth_deadline = tokio::time::sleep_until(product_auth_deadline_instant(
        initial_auth_deadline_ms.unwrap_or(i64::MAX),
    ));
    tokio::pin!(auth_deadline);
    // The first tick fires immediately; consume it so the first ping waits a
    // full interval (the connect snapshot is fresh traffic already).
    heartbeat.tick().await;
    capacity_heartbeat.tick().await;
    loop {
        tokio::select! {
            Some(nonce) = probe_rx.recv() => {
                if send_json(&mut sink, &Outbound::ConnectionProbe { nonce }).await.is_err() {
                    break;
                }
            }
            Some(update) = direct_rx.recv() => {
                let Some(snapshot) = update else {
                    let _ = sink.send(ws_close_auth_required()).await;
                    break;
                };
                if send_json(&mut sink, &snapshot.message).await.is_err() {
                        break;
                }
                if !snapshot.valid || snapshot.user_id != fanout_principal.user_id {
                    let _ = sink.send(ws_close_auth_required()).await;
                    break;
                }
                auth_deadline
                    .as_mut()
                    .reset(product_auth_deadline_instant(snapshot.next_due_at_ms));
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            _ = async {
                tokio::select! {
                    _ = capacity_heartbeat.tick() => {}
                    () = fanout_capacity_runtime.changed.notified() => {}
                }
            }, if fanout_capacity_fencing_token.is_some() => {
                let Some(store) = fanout_capacity_store.as_ref() else {
                    let _ = sink.send(ws_close_capacity_required()).await;
                    break;
                };
                let fencing_token = fanout_capacity_fencing_token.unwrap_or_default();
                match store
                    .heartbeat_active_client(
                        &fanout_capacity_client_id,
                        fencing_token,
                        auth_now_ms(),
                        capacity_lease_ms,
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        let message = active_capacity_message(
                            "lost",
                            None,
                            None,
                            Some(capacity_heartbeat_ms),
                            None,
                            capacity_channel_limit,
                        );
                        let _ = send_json(&mut sink, &message).await;
                        let _ = sink.send(ws_close_capacity_required()).await;
                        fanout_capacity_runtime.notify_changed();
                        break;
                    }
                    Err(error) => {
                        // A transient database outage must not silently let
                        // a client mutate beyond its fenced lease. Close and
                        // let the normal reconnect path re-admit it.
                        tracing::warn!(%error, client_id = fanout_capacity_client_id, "heartbeating active-client lease");
                        let _ = sink.send(ws_close_capacity_required()).await;
                        break;
                    }
                }
            }
            msg = rx.recv() => match msg {
                Ok(msg) => {
                    let result = if fanout_sends_shared_frame(&fanout_principal, msg.outbound()) {
                        send_frame(&mut sink, msg.as_ref()).await
                    } else {
                        let visible = visible_session_ids(&fanout_state.hub, &fanout_principal);
                        let Some(projected) = project_outbound(
                            &fanout_state.hub,
                            &fanout_principal,
                            &visible,
                            msg.outbound().clone(),
                        ) else {
                            continue;
                        };
                        send_json(&mut sink, &projected).await
                    };
                    if result.is_err() {
                        break;
                    }
                }
                // Lagged: a slow client (mobile/5G, or backgrounded) fell
                // >1024 events behind, so the broadcast DROPPED events for it.
                // Its timeline is now permanently inconsistent — e.g. it missed
                // the tool_call_update that completed a tool, so the UI shows a
                // stuck "pending" tool / "working" spinner on an idle session
                // (the observed bug). Continuing would keep serving newer events
                // over that hole forever. Instead CLOSE the socket: the client
                // reconnects and rebuilds a consistent state from a fresh
                // snapshot (the connect path re-sends sessions + snapshots).
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "WS client lagged; closing to force a resync");
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = &mut auth_deadline, if via_cookie => {
                let Some(token) = fanout_cookie.as_deref() else {
                    let _ = sink.send(ws_close_auth_required()).await;
                    break;
                };
                let Some(snapshot) = product_auth_session_message(&fanout_state, token).await else {
                    let _ = sink.send(ws_close_auth_required()).await;
                    break;
                };
                if send_json(&mut sink, &snapshot.message).await.is_err() {
                    break;
                }
                if !snapshot.valid || snapshot.user_id != fanout_principal.user_id {
                    let _ = sink.send(ws_close_auth_required()).await;
                    break;
                }
                auth_deadline
                    .as_mut()
                    .reset(product_auth_deadline_instant(snapshot.next_due_at_ms));
            }
            _ = heartbeat.tick() => {
                let valid = if via_cookie {
                    match fanout_cookie.as_deref() {
                        Some(token) => match product_auth_session_message(&fanout_state, token).await {
                            Some(snapshot) => {
                                if send_json(&mut sink, &snapshot.message).await.is_err() {
                                    break;
                                }
                                auth_deadline
                                    .as_mut()
                                    .reset(product_auth_deadline_instant(snapshot.next_due_at_ms));
                                snapshot.valid && snapshot.user_id == fanout_principal.user_id
                            }
                            None => false,
                        },
                        None => false,
                    }
                } else {
                    principal_still_valid(
                        &fanout_state,
                        false,
                        None,
                        fanout_bearer.as_deref(),
                        fanout_device.as_ref(),
                        &fanout_principal,
                    )
                    .await
                };
                if !valid {
                    tracing::info!(reason = "disabled", "ws_rejected");
                    let _ = sink.send(ws_close_auth_required()).await;
                    break;
                }
                if send_json(&mut sink, &Outbound::Ping).await.is_err() {
                    break;
                }
            }
        }
    }
}

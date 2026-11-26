//! The per-client fan-out task: broadcast events and direct product-auth
//! updates to one WebSocket, app-level heartbeats, product-auth revalidation,
//! and active-client capacity lease heartbeats.

use super::*;

/// Ordered per-connection session projection for legacy clients that do not
/// negotiate `session_deltas=1`. Folds sequenced session deltas AND version-
/// gated `"title"` / `"order"` `SyncPatch`es so synthesized `Outbound::Sessions`
/// frames stay ordered without reading live Hub state or regressing to stale
/// titles/ordering.
#[derive(Clone, Debug, Default)]
pub(crate) struct LegacySessionTracker {
    sessions: Vec<crate::core::SessionMeta>,
    title_version: u64,
    order_version: u64,
}

impl LegacySessionTracker {
    pub(crate) fn from_bootstrap(
        bootstrap: &[Outbound],
        supports_session_deltas: bool,
    ) -> Option<Self> {
        if supports_session_deltas {
            return None;
        }
        let sessions = bootstrap.iter().find_map(|message| match message {
            Outbound::Sessions { sessions } => Some(sessions.clone()),
            _ => None,
        })?;
        let mut tracker = Self {
            sessions,
            title_version: 0,
            order_version: 0,
        };
        for message in bootstrap {
            if let Outbound::SyncPatch {
                state,
                version,
                resync,
                value,
                ..
            } = message
            {
                tracker.apply_sync_patch(state, *version, *resync, value);
            }
        }
        Some(tracker)
    }

    fn apply_sync_patch(
        &mut self,
        state: &str,
        version: u64,
        resync: bool,
        value: &serde_json::Value,
    ) {
        match state {
            "title" => {
                if version < self.title_version || (version == self.title_version && !resync) {
                    return;
                }
                let Some(titles) = value.as_object() else {
                    return;
                };
                self.title_version = version;
                for meta in &mut self.sessions {
                    if let Some(title) = titles.get(&meta.id).and_then(serde_json::Value::as_str) {
                        meta.title = title.to_owned();
                    }
                }
            }
            "order" => {
                if version < self.order_version || (version == self.order_version && !resync) {
                    return;
                }
                let Some(entries) = value.as_array() else {
                    return;
                };
                self.order_version = version;
                let submitted: Vec<String> = entries
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect();
                let current_ids: Vec<String> =
                    self.sessions.iter().map(|meta| meta.id.clone()).collect();
                let merged = crate::core::merge_session_order(&current_ids, &submitted);
                let mut by_id: HashMap<String, crate::core::SessionMeta> = self
                    .sessions
                    .drain(..)
                    .map(|meta| (meta.id.clone(), meta))
                    .collect();
                for id in merged {
                    if let Some(meta) = by_id.remove(&id) {
                        self.sessions.push(meta);
                    }
                }
            }
            _ => {}
        }
    }

    fn apply_sessions(
        &mut self,
        principal: &ProductPrincipal,
        sessions: &[crate::core::SessionMeta],
    ) -> Vec<crate::core::SessionMeta> {
        let filtered: Vec<_> = sessions
            .iter()
            .filter(|meta| principal.can_see(meta.owner_user_id.as_deref()))
            .cloned()
            .collect();
        self.sessions = filtered.clone();
        filtered
    }

    fn upsert(&mut self, session: crate::core::SessionMeta) -> Vec<crate::core::SessionMeta> {
        if let Some(existing) = self.sessions.iter_mut().find(|meta| meta.id == session.id) {
            *existing = session;
        } else {
            self.sessions.push(session);
        }
        self.sessions.clone()
    }

    fn remove(&mut self, session_id: &str) -> Vec<crate::core::SessionMeta> {
        self.sessions.retain(|meta| meta.id != session_id);
        self.sessions.clone()
    }
}

/// Identity and capacity lease state the fan-out task revalidates while the
/// client is connected.
pub struct ClientFanout {
    pub state: Arc<AppState>,
    pub principal: ProductPrincipal,
    pub supports_session_deltas: bool,
    pub bootstrap_sessions_seq: u64,
    pub legacy_sessions: Option<LegacySessionTracker>,
    pub via_cookie: bool,
    pub cookie: Option<String>,
    pub bearer: Option<String>,
    pub device: Option<crate::client_auth::DeviceAccessIdentity>,
    pub initial_auth_deadline_ms: Option<i64>,
    pub capacity_store: Option<Store>,
    pub capacity_runtime: Arc<ActiveCapacityRuntime>,
    pub capacity_client_id: String,
    pub capacity_fencing_token: Option<i64>,
    pub capacity_lease_ms: i64,
    pub capacity_heartbeat_ms: i64,
    pub capacity_channel_limit: u32,
}

// Fan-out task: broadcast events -> this socket, plus a periodic app-level
// heartbeat (Outbound::Ping) so a client can detect a HALF-OPEN socket that
// never fires `onclose` (see Outbound::Ping). Per-client interval - a failed
// heartbeat send reaps a dead client here too.
pub async fn run_client_fanout(
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
        supports_session_deltas,
        bootstrap_sessions_seq,
        mut legacy_sessions,
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
                    if forward_fanout_frame(
                        &mut sink,
                        &fanout_state.hub,
                        &fanout_principal,
                        supports_session_deltas,
                        bootstrap_sessions_seq,
                        &mut legacy_sessions,
                        msg.as_ref(),
                    )
                    .await
                    .is_err()
                    {
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

pub(crate) enum FanoutDelivery<'a> {
    Skip,
    Shared(&'a FanoutFrame),
    Owned(Box<Outbound>),
}

/// Resolve how one broadcast `FanoutFrame` reaches a connected WebSocket:
/// skips frames already covered by the connect bootstrap watermark, enforces
/// per-session owner visibility (including on `SessionRemoved`), reuses the
/// shared pre-serialized delta for opted-in clients, and applies deltas plus
/// title/order `SyncPatch`es in sequence onto a per-connection tracker seeded
/// from bootstrap for legacy clients (never querying live Hub state out of order).
pub(crate) fn resolve_fanout_delivery<'a>(
    hub: &Hub,
    principal: &ProductPrincipal,
    supports_session_deltas: bool,
    bootstrap_sessions_seq: u64,
    legacy_sessions: &mut Option<LegacySessionTracker>,
    frame: &'a FanoutFrame,
) -> FanoutDelivery<'a> {
    if frame
        .sessions_seq()
        .is_some_and(|seq| seq <= bootstrap_sessions_seq)
    {
        return FanoutDelivery::Skip;
    }
    match frame.outbound() {
        Outbound::SyncPatch {
            state,
            version,
            resync,
            value,
            ..
        } => {
            if let Some(tracker) = legacy_sessions.as_mut() {
                tracker.apply_sync_patch(state, *version, *resync, value);
            }
        }
        Outbound::Sessions { sessions } => {
            if let Some(tracker) = legacy_sessions.as_mut() {
                let filtered = tracker.apply_sessions(principal, sessions);
                return if fanout_sends_shared_frame(principal, frame.outbound()) {
                    FanoutDelivery::Shared(frame)
                } else {
                    FanoutDelivery::Owned(Box::new(Outbound::Sessions { sessions: filtered }))
                };
            }
        }
        Outbound::SessionUpdated { session } => {
            if !principal.can_see(session.owner_user_id.as_deref()) {
                return FanoutDelivery::Skip;
            }
            if supports_session_deltas {
                // Single-session frames carry no other user's metadata; once
                // visibility passes, reuse the cached serialized JSON directly.
                return FanoutDelivery::Shared(frame);
            }
            let sessions = legacy_sessions
                .get_or_insert_with(LegacySessionTracker::default)
                .upsert((**session).clone());
            return FanoutDelivery::Owned(Box::new(Outbound::Sessions { sessions }));
        }
        Outbound::SessionRemoved { session_id } => {
            let visible = frame.session_owner().map_or_else(
                || principal.sees_every_session(),
                |owner| principal.can_see(owner),
            );
            if !visible {
                return FanoutDelivery::Skip;
            }
            if supports_session_deltas {
                return FanoutDelivery::Shared(frame);
            }
            let sessions = legacy_sessions
                .get_or_insert_with(LegacySessionTracker::default)
                .remove(session_id);
            return FanoutDelivery::Owned(Box::new(Outbound::Sessions { sessions }));
        }
        _ => {}
    }
    if fanout_sends_shared_frame(principal, frame.outbound()) {
        FanoutDelivery::Shared(frame)
    } else {
        let visible = visible_session_ids(hub, principal);
        project_outbound(hub, principal, &visible, frame.outbound().clone())
            .map_or(FanoutDelivery::Skip, |outbound| {
                FanoutDelivery::Owned(Box::new(outbound))
            })
    }
}

#[allow(clippy::result_unit_err)]
async fn forward_fanout_frame(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    hub: &Hub,
    principal: &ProductPrincipal,
    supports_session_deltas: bool,
    bootstrap_sessions_seq: u64,
    legacy_sessions: &mut Option<LegacySessionTracker>,
    frame: &FanoutFrame,
) -> Result<(), ()> {
    match resolve_fanout_delivery(
        hub,
        principal,
        supports_session_deltas,
        bootstrap_sessions_seq,
        legacy_sessions,
        frame,
    ) {
        FanoutDelivery::Skip => Ok(()),
        FanoutDelivery::Shared(frame) => send_frame(sink, frame).await,
        FanoutDelivery::Owned(projected) => send_json(sink, projected.as_ref()).await,
    }
}

//! Stdio ACP ↔ cowboy-daemon HTTP+WS bridge (design §13a).
//!
//! Spawned by an ACP client (e.g. Zed's `agent_servers["cowboy"]` entry) as a
//! child process; runs in the foreground over stdio. Translates between the
//! client's stdio ACP protocol and a running cowboy **daemon's** HTTP + WS
//! surface. **Stateless**: every session, event, transcript, and permission
//! lives in the daemon — this process is purely a protocol adapter.
//!
//! Architectural rationale (design §13a, post-pivot 2026-05-28):
//!
//! - cowboy daemon is a long-running systemd service binding `:3333`. It owns
//!   the [`Hub`], `Supervisor`, `SQLite`, and is the **single source of
//!   truth**.
//! - All non-Zed surfaces (Web UI on phone / iPad / browser) connect to it
//!   over WebSocket as today — **unchanged**.
//! - The bridge gives Zed a stdio ACP face onto that same daemon. A session
//!   created here is the daemon's session; events flow from daemon → bridge →
//!   Zed, and from daemon → all other WS clients in parallel; permission
//!   "first answer wins" naturally falls out of the supervisor's existing
//!   `oneshot` pattern.
//!
//! ACP plumbing (crate 0.14): cowboy is the [`Agent`] role here. `connect_with`
//! gives the WS supervisor a [`ConnectionTo<Client>`] for pushing
//! `session/update` notifications and `session/request_permission` requests at
//! Zed. Unhandled client methods (`authenticate` / `load_session` / set mode /
//! any extension) fall through to the crate's default handler, which answers
//! `method_not_found` — matching the explicit refusals the previous trait impl
//! returned. (The old speculative `_session/delete` sniffing is dropped until a
//! real schema exists; ACP has no agent-initiated session-list push, so
//! phone-initiated sessions remain invisible to Zed — a deferred follow-up.)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;

use agent_client_protocol::schema::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PermissionOption, PromptCapabilities, PromptRequest,
    PromptResponse, ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest, SessionId,
    SessionNotification, SessionUpdate, StopReason, ToolCallUpdate,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error, Stdio};
use anyhow::{Context as _, Result};
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tracing_subscriber::EnvFilter;

use crate::cli::AcpBridgeArgs;
use crate::core::{Event, Inbound, Outbound};

/// State shared across the connection's handler closures, the WS-recv task, and
/// the fan-out helpers. All are `Send` (the crate dispatches on a `Send`
/// executor), so `Arc` + `Mutex`, not `Rc`/`RefCell`.
struct Bridge {
    /// Default provider for `session/new`. Could be overridden per-session in
    /// the future via `_meta` from the client; v0 is one provider per bridge
    /// process (= one `agent_servers` entry in Zed config).
    provider: String,
    api_url: String,
    /// Sender to the daemon WS (Inbound commands). The recv task owns the
    /// stream half.
    ws_tx: mpsc::UnboundedSender<Inbound>,
    /// Sessions this bridge created via `session/new`. Hub events for any
    /// session NOT in here are dropped (they belong to the phone Web UI / a
    /// different bridge / etc.).
    known_sessions: Mutex<HashSet<String>>,
    /// One sender per in-flight `prompt`, resolved when the daemon emits
    /// `Event::TurnEnd` for that session.
    pending_prompts: Mutex<HashMap<String, oneshot::Sender<StopReason>>>,
    /// One sender per outstanding outbound `session/request_permission` we
    /// fired at the client. Signalled when the daemon broadcasts a
    /// `PermissionResolved` event (= some other client answered first) so we
    /// can abandon the RPC without double-answering.
    pending_perms: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

/// Translate one daemon-side Hub `Event` into ACP outbound calls. Mirrors the
/// table in `tasks/active/cowboy-zed-acp/design.md §4.6`.
fn handle_envelope(
    cx: &ConnectionTo<Client>,
    state: &Arc<Bridge>,
    session_id: String,
    event: Event,
) {
    match event {
        Event::Update { update } => forward_update(cx, &session_id, update),
        Event::PermissionRequest {
            request_id,
            tool_call,
            options,
        } => dispatch_permission_request(cx, state, session_id, request_id, tool_call, options),
        Event::PermissionResolved { request_id, .. } => {
            if let Some(tx) = state.pending_perms.lock().unwrap().remove(&request_id) {
                let _ = tx.send(());
            }
        }
        Event::TurnEnd { stop_reason } => {
            if let Some(tx) = state.pending_prompts.lock().unwrap().remove(&session_id) {
                let _ = tx.send(parse_stop_reason(&stop_reason));
            }
        }
        Event::Lifecycle { .. } => {
            // No ACP equivalent — daemon's WS clients see lifecycle; Zed
            // infers from the prompt RPC's success/failure.
        }
    }
}

fn parse_stop_reason(s: &str) -> StopReason {
    if let Some(rest) = s.strip_prefix("error:") {
        tracing::warn!(
            detail = rest.trim(),
            "upstream prompt errored; reporting Cancelled"
        );
        return StopReason::Cancelled;
    }
    match s {
        "EndTurn" => StopReason::EndTurn,
        "MaxTokens" => StopReason::MaxTokens,
        "MaxTurnRequests" => StopReason::MaxTurnRequests,
        "Refusal" => StopReason::Refusal,
        "Cancelled" => StopReason::Cancelled,
        other => {
            tracing::warn!(stop = other, "unknown stop_reason; defaulting to EndTurn");
            StopReason::EndTurn
        }
    }
}

fn forward_update(cx: &ConnectionTo<Client>, session_id: &str, update: serde_json::Value) {
    let update = match serde_json::from_value::<SessionUpdate>(update) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(error = %e, "skip non-renderable SessionUpdate variant");
            return;
        }
    };
    if let Err(e) =
        cx.send_notification(SessionNotification::new(SessionId::new(session_id), update))
    {
        tracing::warn!(error = ?e, "session_notification failed");
    }
}

fn dispatch_permission_request(
    cx: &ConnectionTo<Client>,
    state: &Arc<Bridge>,
    session_id: String,
    request_id: String,
    tool_call: serde_json::Value,
    options: serde_json::Value,
) {
    let Ok(tool_call) = serde_json::from_value::<ToolCallUpdate>(tool_call) else {
        tracing::warn!(request_id, "PermissionRequest tool_call deserialize failed");
        return;
    };
    let Ok(options) = serde_json::from_value::<Vec<PermissionOption>>(options) else {
        tracing::warn!(request_id, "PermissionRequest options deserialize failed");
        return;
    };
    let (short_tx, short_rx) = oneshot::channel::<()>();
    state
        .pending_perms
        .lock()
        .unwrap()
        .insert(request_id.clone(), short_tx);
    let state = state.clone();
    let cx = cx.clone();
    let _ = cx.clone().spawn(async move {
        let req =
            RequestPermissionRequest::new(SessionId::new(session_id.as_str()), tool_call, options);
        let (chosen, resolved_elsewhere) = tokio::select! {
            result = cx.send_request(req).block_task() => match result {
                Ok(resp) => match resp.outcome {
                    RequestPermissionOutcome::Selected(sel) => {
                        (Some(sel.option_id.0.to_string()), false)
                    }
                    // `Cancelled` and any future variant: no selection made.
                    _ => (None, false),
                },
                Err(e) => {
                    tracing::warn!(error = ?e, "request_permission rpc failed");
                    (None, false)
                }
            },
            _ = short_rx => (None, true),
        };
        state.pending_perms.lock().unwrap().remove(&request_id);
        if resolved_elsewhere {
            // Another WS client (phone) already answered; daemon will route
            // its choice. Sending our None would double-cancel.
            return Ok(());
        }
        if let Err(e) = state.ws_tx.send(Inbound::Permission {
            session_id,
            request_id,
            option_id: chosen,
        }) {
            tracing::warn!(error = %e, "permission: daemon ws closed");
        }
        Ok(())
    });
}

/// Minimal HTTP/1.1 POST that talks to the daemon's `POST /api/sessions`.
/// Avoids pulling reqwest just for one local request; the daemon is loopback,
/// no TLS, no chunked encoding, no redirects.
async fn http_post_new_session(api_url: &str, provider: &str, cwd: &str) -> Result<String> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let (host, port) = parse_http_host(api_url)?;
    let body = serde_json::to_vec(&serde_json::json!({
        "provider": provider,
        "cwd": cwd,
        "origin": "zed",
    }))?;
    let header = format!(
        "POST /api/sessions HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .with_context(|| format!("connect to daemon {host}:{port}"))?;
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    let mut response = Vec::with_capacity(512);
    stream.read_to_end(&mut response).await?;

    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("no header/body split in daemon response")?;
    let status_line_end = response
        .windows(2)
        .position(|w| w == b"\r\n")
        .context("no status line in daemon response")?;
    let status_line = std::str::from_utf8(&response[..status_line_end])?;
    if !status_line.contains(" 201 ") && !status_line.contains(" 200 ") {
        let body = String::from_utf8_lossy(&response[split + 4..]);
        anyhow::bail!("daemon /api/sessions {status_line} body={body}");
    }
    let body_bytes = &response[split + 4..];
    let parsed: serde_json::Value = serde_json::from_slice(body_bytes)
        .with_context(|| format!("decoding daemon response: {body_bytes:?}"))?;
    parsed
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .context("daemon response missing session_id")
}

fn parse_http_host(api_url: &str) -> Result<(String, u16)> {
    let stripped = api_url
        .strip_prefix("http://")
        .context("api_url must start with http://")?;
    let host_port = stripped.split('/').next().unwrap_or(stripped);
    let mut parts = host_port.split(':');
    let host = parts.next().context("api_url has no host")?.to_string();
    let port: u16 = parts
        .next()
        .map_or(Ok(80), str::parse)
        .context("api_url port not a u16")?;
    Ok((host, port))
}

/// Bridge entrypoint.
///
/// # Errors
/// If stdio setup or the daemon WS connect fails, or if either I/O loop ends
/// with an error.
#[allow(clippy::too_many_lines)] // one cohesive builder + handler registration
pub async fn run(args: AcpBridgeArgs) -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    tracing::info!(
        provider = %args.provider,
        daemon_url = %args.daemon_url,
        api_url = %args.api_url,
        "cowboy acp-bridge starting",
    );

    // Fail fast on an unknown provider id. Each Zed `agent_servers` entry binds
    // one provider via `--provider`. A typo would otherwise start cleanly and
    // only error on the first `session/new`. Bailing here makes Zed show the
    // launch failure immediately, with the valid ids in stderr.
    if crate::provider::lookup(&args.provider).is_none() {
        let mut known: Vec<&str> = crate::provider::builtin().into_keys().collect();
        known.sort_unstable();
        anyhow::bail!(
            "unknown provider {:?}; known providers: {}",
            args.provider,
            known.join(", "),
        );
    }

    let daemon_url = args.daemon_url.clone();

    // Connect once up front; if the daemon is down at startup, fail fast so Zed
    // surfaces it in the Agent Panel. After this first success the WS
    // supervisor reconnects automatically across daemon restarts.
    let (ws, _resp) = tokio_tungstenite::connect_async(&daemon_url)
        .await
        .with_context(|| format!("connect to daemon ws {daemon_url}"))?;
    tracing::info!("daemon ws connected");

    let (ws_tx, ws_tx_rx) = mpsc::unbounded_channel::<Inbound>();

    let state = Arc::new(Bridge {
        provider: args.provider,
        api_url: args.api_url,
        ws_tx,
        known_sessions: Mutex::new(HashSet::new()),
        pending_prompts: Mutex::new(HashMap::new()),
        pending_perms: Mutex::new(HashMap::new()),
    });

    let new_state = state.clone();
    let prompt_state = state.clone();
    let cancel_state = state.clone();
    let main_state = state.clone();

    // The ACP connection runs over stdio. `connect_with` hands the WS
    // supervisor a `ConnectionTo<Client>`; it returns (with a connection-closed
    // error) when Zed shuts the child's stdio, which we treat as a clean exit.
    let result = Agent
        .builder()
        .name("cowboy")
        .on_receive_request(
            async |_req: InitializeRequest,
                   responder,
                   _cx: ConnectionTo<Client>|
                   -> Result<(), Error> {
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1).agent_capabilities(
                        AgentCapabilities::new()
                            .load_session(false)
                            .prompt_capabilities(
                                // Zed's composer blocks image paste unless we
                                // advertise this; claude-agent-acp accepts images,
                                // so it's safe end-to-end.
                                PromptCapabilities::new().image(true),
                            ),
                    ),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest,
                        responder,
                        cx: ConnectionTo<Client>|
                        -> Result<(), Error> {
                let state = new_state.clone();
                // The HTTP POST is synchronous from the daemon's side — it
                // returns the assigned id before we tell Zed (race-free vs a
                // fire-and-forget WS command). Defer off the dispatch loop.
                cx.spawn(async move {
                    let cwd = req.cwd.to_string_lossy().into_owned();
                    match http_post_new_session(&state.api_url, &state.provider, &cwd).await {
                        Ok(session_id) => {
                            state
                                .known_sessions
                                .lock()
                                .unwrap()
                                .insert(session_id.clone());
                            responder.respond(NewSessionResponse::new(SessionId::new(
                                session_id.as_str(),
                            )))?;
                        }
                        Err(e) => {
                            responder.respond_with_error(
                                Error::internal_error()
                                    .data(serde_json::Value::String(e.to_string())),
                            )?;
                        }
                    }
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: PromptRequest,
                        responder,
                        cx: ConnectionTo<Client>|
                        -> Result<(), Error> {
                let state = prompt_state.clone();
                let session_id = req.session_id.0.to_string();
                if !state.known_sessions.lock().unwrap().contains(&session_id) {
                    return responder.respond_with_error(Error::invalid_params().data(
                        serde_json::Value::String(format!(
                            "session {session_id} not owned by this bridge"
                        )),
                    ));
                }
                // Pass the entire content array through to the daemon as
                // ACP-shaped JSON; the daemon forwards it verbatim to the
                // upstream agent, so pasted images make it through.
                let content: Vec<serde_json::Value> = req
                    .prompt
                    .iter()
                    .filter_map(|b| match serde_json::to_value(b) {
                        Ok(v) => Some(v),
                        Err(e) => {
                            tracing::warn!(error = %e, "serializing prompt block; dropping");
                            None
                        }
                    })
                    .collect();
                let (tx, rx) = oneshot::channel::<StopReason>();
                state
                    .pending_prompts
                    .lock()
                    .unwrap()
                    .insert(session_id.clone(), tx);
                if state
                    .ws_tx
                    .send(Inbound::Prompt {
                        session_id: session_id.clone(),
                        text: String::new(),
                        content,
                    })
                    .is_err()
                {
                    state.pending_prompts.lock().unwrap().remove(&session_id);
                    return responder.respond_with_error(
                        Error::internal_error()
                            .data(serde_json::Value::String("daemon ws closed".to_owned())),
                    );
                }
                // Defer the response until the daemon emits TurnEnd; awaiting
                // here would block every other incoming message (e.g. cancel).
                cx.spawn(async move {
                    let stop_reason = rx.await.unwrap_or(StopReason::Cancelled);
                    responder.respond(PromptResponse::new(stop_reason))?;
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |args: CancelNotification, _cx: ConnectionTo<Client>| -> Result<(), Error> {
                let session_id = args.session_id.0.to_string();
                if cancel_state
                    .ws_tx
                    .send(Inbound::Cancel { session_id })
                    .is_err()
                {
                    tracing::warn!("cancel: daemon ws closed");
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(Stdio::new(), async move |cx: ConnectionTo<Client>| {
            // WS supervisor: owns the daemon connection for the whole bridge
            // lifetime, reconnecting across daemon restarts, and pushes daemon
            // events at Zed via `cx`. It also owns `ws_tx_rx`, so the handlers'
            // `ws_tx.send` never sees a dropped receiver during a brief
            // disconnect. When stdio closes the connection collapses and this
            // spawned task is dropped with it.
            cx.clone()
                .spawn(ws_loop(main_state, cx.clone(), daemon_url, ws_tx_rx, ws))?;
            std::future::pending::<Result<(), Error>>().await
        })
        .await;

    match result {
        Ok(()) => Ok(()),
        // Zed closing the child's stdio collapses the connection; that is the
        // normal shutdown path, not an error.
        Err(e) => {
            tracing::info!(reason = %e, "acp connection closed; bridge exiting");
            Ok(())
        }
    }
}

/// The daemon WS connection type (what `connect_async` yields for `ws://`).
type DaemonWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Maintain the daemon WS for the bridge's lifetime, reconnecting when the
/// daemon restarts. The Zed-facing ACP stdio connection stays up across daemon
/// reconnects; only this WS is re-established. Returns when `ws_tx_rx` closes
/// (the bridge is shutting down).
async fn ws_loop(
    state: Arc<Bridge>,
    cx: ConnectionTo<Client>,
    daemon_url: String,
    mut ws_tx_rx: mpsc::UnboundedReceiver<Inbound>,
    initial: DaemonWs,
) -> Result<(), Error> {
    let mut conn_ws = initial;
    loop {
        let (mut sink, mut stream) = conn_ws.split();
        // Pump until the connection drops (or the bridge shuts down).
        loop {
            tokio::select! {
                maybe = ws_tx_rx.recv() => match maybe {
                    None => return Ok(()), // ws_tx dropped → bridge shutting down
                    Some(inbound) => {
                        let text = match serde_json::to_string(&inbound) {
                            Ok(t) => t,
                            Err(e) => {
                                tracing::warn!(error = %e, "serializing Inbound");
                                continue;
                            }
                        };
                        if let Err(e) = sink.send(Message::Text(text.into())).await {
                            tracing::warn!(error = %e, "ws send failed; reconnecting");
                            break;
                        }
                    }
                },
                msg = stream.next() => match msg {
                    Some(Ok(Message::Text(text))) => dispatch_outbound(&cx, &state, &text),
                    Some(Ok(_)) => {} // non-text frame (ping/binary): ignore
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "ws recv error; reconnecting");
                        break;
                    }
                    None => {
                        tracing::warn!("daemon ws closed; reconnecting");
                        break;
                    }
                },
            }
        }
        // Disconnected. End any in-flight turns so Zed stops waiting for a
        // `TurnEnd` that died with the old connection (the user resends; the
        // resend flushes once we reconnect), then re-establish the WS.
        fail_pending_on_disconnect(&state);
        conn_ws = reconnect(&daemon_url).await;
    }
}

/// Reconnect to the daemon with capped exponential backoff. Loops until it
/// succeeds — the daemon is a local systemd unit, so "down" is virtually
/// always "restarting, back in a moment", not "gone forever".
async fn reconnect(daemon_url: &str) -> DaemonWs {
    let mut delay = std::time::Duration::from_millis(250);
    let cap = std::time::Duration::from_secs(5);
    loop {
        match tokio_tungstenite::connect_async(daemon_url).await {
            Ok((ws, _resp)) => {
                tracing::info!("daemon ws reconnected");
                return ws;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                    "daemon ws reconnect failed; retrying",
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(cap);
            }
        }
    }
}

/// Dispatch one decoded daemon `Outbound`. Extracted so the connection loop
/// stays readable and the recv logic is identical across reconnects.
fn dispatch_outbound(cx: &ConnectionTo<Client>, state: &Arc<Bridge>, text: &str) {
    let outbound: Outbound = match serde_json::from_str(text) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = %e, "decoding daemon Outbound");
            return;
        }
    };
    match outbound {
        Outbound::Event { envelope } => {
            let session_id = envelope.session_id.clone();
            if !state.known_sessions.lock().unwrap().contains(&session_id) {
                return;
            }
            handle_envelope(cx, state, session_id, envelope.event);
        }
        // A daemon-reported error for a session: if a prompt is in flight, the
        // daemon won't emit a `TurnEnd`, so the pending-prompt oneshot would
        // never resolve and Zed would spin forever. Resolve it here with
        // `Cancelled`. Only fires when a prompt is actually pending.
        Outbound::Error {
            session_id: Some(sid),
            message,
        } => {
            if let Some(tx) = state.pending_prompts.lock().unwrap().remove(&sid) {
                tracing::warn!(
                    session = %sid,
                    error = %message,
                    "daemon error for in-flight prompt; ending the turn",
                );
                let _ = tx.send(StopReason::Cancelled);
            }
        }
        // Sessions / Snapshot / ConfigOptions / session-less Error are Web-UI
        // bookkeeping; Zed has its own model, so drop them.
        _ => {}
    }
}

/// End every in-flight turn on a daemon disconnect so Zed stops waiting for a
/// `TurnEnd` that died with the old connection. No-op when nothing is pending.
fn fail_pending_on_disconnect(state: &Arc<Bridge>) {
    let mut pending = state.pending_prompts.lock().unwrap();
    if pending.is_empty() {
        return;
    }
    tracing::warn!(
        count = pending.len(),
        "daemon disconnected; ending in-flight turns (resend to continue)",
    );
    for (_sid, tx) in pending.drain() {
        let _ = tx.send(StopReason::Cancelled);
    }
}

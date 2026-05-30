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
//! Synchronization caveats (v0, see `tasks/active/cowboy-zed-acp/`):
//!
//! - Only sessions created **via this bridge** are forwarded to Zed.
//!   Phone-initiated sessions are not visible to Zed — ACP has no
//!   agent-initiated session-list push. Cross-direction visibility is a
//!   deferred follow-up.
//! - Session deletion sync (`unstable_deleteSession`, daemon-initiated
//!   notifications) is also deferred — the daemon's WS surface doesn't yet
//!   surface delete events, and ACP has no message for it either.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use agent_client_protocol::{
    Agent, AgentCapabilities, AgentSideConnection, AuthenticateRequest, AuthenticateResponse,
    CancelNotification, Client as _, Error, ExtNotification, ExtRequest, ExtResponse,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse,
    McpCapabilities, NewSessionRequest, NewSessionResponse, PermissionOption, PromptCapabilities,
    PromptRequest, PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, SessionId,
    SessionNotification, SessionUpdate, SetSessionModeRequest, SetSessionModeResponse, StopReason,
    ToolCallUpdate, V1,
};
use anyhow::{Context as _, Result};
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing_subscriber::EnvFilter;

use crate::cli::AcpBridgeArgs;
use crate::core::{Event, Inbound, Outbound};

/// State shared across the [`Agent`] trait impl, the WS-recv task, and the
/// fan-out helpers. All inhabit one `LocalSet` — `Rc<RefCell<_>>` is sound.
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
    known_sessions: RefCell<HashSet<String>>,
    /// One sender per in-flight `Agent::prompt`, resolved when the daemon
    /// emits `Event::TurnEnd` for that session.
    pending_prompts: RefCell<HashMap<String, oneshot::Sender<StopReason>>>,
    /// One sender per outstanding outbound `session/request_permission` we
    /// fired at the client. Signalled when the daemon broadcasts a
    /// `PermissionResolved` event (= some other client answered first) so we
    /// can abandon the RPC without double-answering.
    pending_perms: RefCell<HashMap<String, oneshot::Sender<()>>>,
}

/// ACP `Agent` trait impl. Every method translates to a daemon HTTP / WS
/// operation. No upstream agent process is spawned here — that's the daemon's
/// job.
struct AcpServer {
    state: Rc<Bridge>,
}

#[async_trait::async_trait(?Send)]
impl Agent for AcpServer {
    async fn initialize(&self, _req: InitializeRequest) -> Result<InitializeResponse, Error> {
        Ok(InitializeResponse {
            protocol_version: V1,
            agent_capabilities: AgentCapabilities {
                load_session: false,
                prompt_capabilities: PromptCapabilities {
                    // Zed's composer will block image paste unless we
                    // advertise this. claude-agent-acp itself accepts images
                    // (its own initialize response carries image=true), so
                    // it's safe to enable end-to-end.
                    image: true,
                    audio: false,
                    embedded_context: false,
                    meta: None,
                },
                mcp_capabilities: McpCapabilities::default(),
                meta: None,
            },
            auth_methods: vec![],
            meta: None,
        })
    }

    async fn authenticate(&self, _req: AuthenticateRequest) -> Result<AuthenticateResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn new_session(&self, req: NewSessionRequest) -> Result<NewSessionResponse, Error> {
        let cwd = req.cwd.to_string_lossy().into_owned();
        // The HTTP POST is synchronous — the daemon returns the assigned id
        // before we tell Zed. WS Inbound::NewSession would be fire-and-forget
        // and force us to diff session lists; the HTTP path is race-free.
        let session_id = http_post_new_session(&self.state.api_url, &self.state.provider, &cwd)
            .await
            .map_err(|e| Error::internal_error().with_data(e.to_string()))?;
        self.state
            .known_sessions
            .borrow_mut()
            .insert(session_id.clone());
        Ok(NewSessionResponse {
            session_id: SessionId(Arc::from(session_id.as_str())),
            modes: None,
            meta: None,
        })
    }

    async fn prompt(&self, req: PromptRequest) -> Result<PromptResponse, Error> {
        let session_id = req.session_id.0.to_string();
        if !self.state.known_sessions.borrow().contains(&session_id) {
            return Err(Error::invalid_params()
                .with_data(format!("session {session_id} not owned by this bridge")));
        }
        // Pass the entire content array through to the daemon as ACP-shaped
        // JSON; the daemon deserializes back to `ContentBlock` and forwards
        // verbatim to the upstream agent. We no longer collapse to text — so
        // pasted images (Zed → bridge → daemon → claude-agent-acp) actually
        // make it through. Anything the upstream doesn't accept will be
        // dropped or errored there.
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
        let (tx, rx) = oneshot::channel();
        self.state
            .pending_prompts
            .borrow_mut()
            .insert(session_id.clone(), tx);
        self.state
            .ws_tx
            .send(Inbound::Prompt {
                session_id: session_id.clone(),
                text: String::new(),
                content,
            })
            .map_err(|_| Error::internal_error().with_data("daemon ws closed"))?;
        let stop_reason = rx.await.unwrap_or(StopReason::Cancelled);
        Ok(PromptResponse {
            stop_reason,
            meta: None,
        })
    }

    async fn cancel(&self, args: CancelNotification) -> Result<(), Error> {
        let session_id = args.session_id.0.to_string();
        if let Err(e) = self.state.ws_tx.send(Inbound::Cancel { session_id }) {
            tracing::warn!(error = %e, "cancel: daemon ws closed");
        }
        Ok(())
    }

    async fn load_session(&self, _req: LoadSessionRequest) -> Result<LoadSessionResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn set_session_mode(
        &self,
        _req: SetSessionModeRequest,
    ) -> Result<SetSessionModeResponse, Error> {
        Err(Error::method_not_found())
    }

    async fn ext_method(&self, req: ExtRequest) -> Result<ExtResponse, Error> {
        // Log every extension call so we learn what Zed actually sends.
        // Particularly interested in any future `_session/delete` or
        // `unstable_deleteSession` — when we see one, wire it through to
        // `Inbound::DeleteSession` on the daemon. v0 just refuses (no schema
        // agreed yet); the side effect we DO do is forward to the daemon so
        // its session list converges.
        let method = req.method.to_string();
        let params_str = req.params.get();
        tracing::info!(method = %method, params = params_str, "ext_method seen");
        if matches_delete_method(&method) {
            if let Some(session_id) = serde_json::from_str::<serde_json::Value>(params_str)
                .ok()
                .and_then(|v| {
                    v.get("sessionId")
                        .and_then(|s| s.as_str())
                        .map(str::to_owned)
                })
            {
                tracing::info!(
                    session_id,
                    "ext_method looks like a session delete; forwarding to daemon"
                );
                let _ = self.state.ws_tx.send(Inbound::DeleteSession { session_id });
            }
        }
        Err(Error::method_not_found())
    }

    async fn ext_notification(&self, args: ExtNotification) -> Result<(), Error> {
        let method = args.method.to_string();
        let params = args.params.get();
        tracing::info!(method = %method, params = params, "ext_notification seen");
        Ok(())
    }
}

/// Heuristic for "is this ACP extension method a session-delete?" We don't
/// know the exact name Zed will use (the only public hint is the SDK's
/// `unstable_deleteSession`), so we match common variants.
fn matches_delete_method(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("delete") && lower.contains("session")
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

/// Translate one daemon-side Hub `Event` into ACP outbound calls. Mirrors the
/// table in `tasks/active/cowboy-zed-acp/design.md §4.6`.
fn handle_envelope(
    conn: &Rc<AgentSideConnection>,
    state: &Rc<Bridge>,
    session_id: String,
    event: Event,
) {
    match event {
        Event::Update { update } => forward_update(conn, &session_id, update),
        Event::PermissionRequest {
            request_id,
            tool_call,
            options,
        } => dispatch_permission_request(conn, state, session_id, request_id, tool_call, options),
        Event::PermissionResolved { request_id, .. } => {
            if let Some(tx) = state.pending_perms.borrow_mut().remove(&request_id) {
                let _ = tx.send(());
            }
        }
        Event::TurnEnd { stop_reason } => {
            if let Some(tx) = state.pending_prompts.borrow_mut().remove(&session_id) {
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

fn forward_update(conn: &Rc<AgentSideConnection>, session_id: &str, update: serde_json::Value) {
    let update = match serde_json::from_value::<SessionUpdate>(update) {
        Ok(u) => u,
        Err(e) => {
            tracing::debug!(error = %e, "skip non-renderable SessionUpdate variant");
            return;
        }
    };
    let conn = conn.clone();
    let session_id = SessionId(Arc::from(session_id));
    tokio::task::spawn_local(async move {
        if let Err(e) = conn
            .session_notification(SessionNotification {
                session_id,
                update,
                meta: None,
            })
            .await
        {
            tracing::warn!(error = ?e, "session_notification failed");
        }
    });
}

fn dispatch_permission_request(
    conn: &Rc<AgentSideConnection>,
    state: &Rc<Bridge>,
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
        .borrow_mut()
        .insert(request_id.clone(), short_tx);
    let conn = conn.clone();
    let state = state.clone();
    tokio::task::spawn_local(async move {
        let req = RequestPermissionRequest {
            session_id: SessionId(Arc::from(session_id.as_str())),
            tool_call,
            options,
            meta: None,
        };
        let (chosen, resolved_elsewhere) = tokio::select! {
            result = conn.request_permission(req) => match result {
                Ok(resp) => match resp.outcome {
                    RequestPermissionOutcome::Selected { option_id } => {
                        (Some(option_id.0.to_string()), false)
                    }
                    RequestPermissionOutcome::Cancelled => (None, false),
                },
                Err(e) => {
                    tracing::warn!(error = ?e, "request_permission rpc failed");
                    (None, false)
                }
            },
            _ = short_rx => (None, true),
        };
        state.pending_perms.borrow_mut().remove(&request_id);
        if resolved_elsewhere {
            // Another WS client (phone) already answered; daemon will route
            // its choice. Sending our None would double-cancel.
            return;
        }
        if let Err(e) = state.ws_tx.send(Inbound::Permission {
            session_id,
            request_id,
            option_id: chosen,
        }) {
            tracing::warn!(error = %e, "permission: daemon ws closed");
        }
    });
}

/// Bridge entrypoint.
///
/// # Errors
/// If stdio setup or the daemon WS connect fails, or if either I/O loop ends
/// with an error.
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

    // Connect to the daemon WS first; if the daemon isn't up, fail fast
    // (Zed will surface the error to the Agent Panel).
    let (ws, _resp) = tokio_tungstenite::connect_async(&args.daemon_url)
        .await
        .with_context(|| format!("connect to daemon ws {}", args.daemon_url))?;
    tracing::info!("daemon ws connected");
    let (mut ws_sink, mut ws_stream) = ws.split();

    let (ws_tx, mut ws_tx_rx) = mpsc::unbounded_channel::<Inbound>();

    let state = Rc::new(Bridge {
        provider: args.provider,
        api_url: args.api_url,
        ws_tx,
        known_sessions: RefCell::new(HashSet::new()),
        pending_prompts: RefCell::new(HashMap::new()),
        pending_perms: RefCell::new(HashMap::new()),
    });

    let server = AcpServer {
        state: state.clone(),
    };
    let stdin = tokio::io::stdin().compat();
    let stdout = tokio::io::stdout().compat_write();
    let (conn, acp_io) = AgentSideConnection::new(server, stdout, stdin, |fut| {
        tokio::task::spawn_local(fut);
    });
    let conn = Rc::new(conn);

    // WS-send task: drain the bridge's Inbound channel onto the daemon WS.
    let ws_send_task = tokio::task::spawn_local(async move {
        while let Some(inbound) = ws_tx_rx.recv().await {
            let text = match serde_json::to_string(&inbound) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, "serializing Inbound");
                    continue;
                }
            };
            if let Err(e) = ws_sink.send(Message::Text(text.into())).await {
                tracing::warn!(error = %e, "ws send failed");
                break;
            }
        }
    });

    // WS-recv task: each daemon Outbound is dispatched.
    let conn_for_recv = conn.clone();
    let state_for_recv = state.clone();
    let ws_recv_task = tokio::task::spawn_local(async move {
        while let Some(msg) = ws_stream.next().await {
            let Ok(Message::Text(text)) = msg else {
                continue;
            };
            let outbound: Outbound = match serde_json::from_str(&text) {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!(error = %e, "decoding daemon Outbound");
                    continue;
                }
            };
            let Outbound::Event { envelope } = outbound else {
                // Sessions / Snapshot / Error are bookkeeping for Web UI;
                // Zed has its own session-list model, so we drop them.
                continue;
            };
            let session_id = envelope.session_id.clone();
            if !state_for_recv.known_sessions.borrow().contains(&session_id) {
                continue;
            }
            handle_envelope(&conn_for_recv, &state_for_recv, session_id, envelope.event);
        }
    });

    // ACP IO loop. When stdio closes (Zed shuts the child down), the bridge
    // exits and the WS tasks are aborted by drop.
    let r = acp_io.await.context("acp io");
    ws_send_task.abort();
    ws_recv_task.abort();
    r?;
    Ok(())
}

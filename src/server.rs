//! HTTP / WebSocket server (design §5).
//!
//! Every frontend is an **equal subscriber** to one WebSocket stream. On
//! connect a client receives the session list plus a full snapshot of each
//! session's event log, then a live tail of all events — so "new session shows
//! everywhere" and "permission resolves everywhere" are just broadcasts. The
//! same socket carries inbound commands.
//!
//! v1 has **no auth** and binds `0.0.0.0` by deliberate choice (LAN-only use);
//! design §9 auth/pairing is a follow-up.

use std::sync::Arc;

use anyhow::Context as _;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::Router;
use futures::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use serde::Deserialize;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::acp::AgentCommand;
use crate::cli::ServeArgs;
use crate::core::{Hub, Outbound};
use crate::supervisor::Supervisor;

struct AppState {
    hub: Hub,
    supervisor: Supervisor,
}

/// A command sent by a client over the WebSocket.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Inbound {
    /// Start a new agent session.
    NewSession {
        provider: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Send a user turn to a session.
    Prompt { session_id: String, text: String },
    /// Cancel a session's current turn.
    Cancel { session_id: String },
    /// Answer a pending permission request.
    Permission {
        session_id: String,
        request_id: String,
        #[serde(default)]
        option_id: Option<String>,
    },
}

/// Start the HTTP/WebSocket server and the agent supervisor.
pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    init_tracing();

    let hub = Hub::new();
    let supervisor = Supervisor::new(hub.clone(), args.workspace_root.clone());
    let state = Arc::new(AppState { hub, supervisor });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/version.json", get(version_json))
        .route("/ws", any(ws_upgrade))
        // Everything else: the embedded SPA, with index.html fallback for
        // client-side routes.
        .fallback(static_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;

    tracing::info!(
        addr = %args.bind,
        workspace = %args.workspace_root.display(),
        data_dir = %args.data_dir.display(),
        "cowboy serving",
    );

    axum::serve(listener, app).await.context("axum serve")?;
    Ok(())
}

pub(crate) fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

async fn healthz() -> &'static str {
    "ok"
}

/// The deployed build id the atlantis portal polls to raise an update banner
/// over a kept-alive iframe running a stale bundle (atlantis README → "Update
/// notifications"). The flake injects the app's commit SHA at build time; a
/// plain `cargo build` falls back to the crate version.
async fn version_json() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "version": option_env!("ATLANTIS_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")),
    }))
}

/// The built web UI (Vite output), embedded at compile time. The flake builds
/// `web/dist` with bun before the cargo build so this folder exists.
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// Serve an embedded asset by path, falling back to `index.html` so the SPA
/// owns client-side routing. Missing `index.html` (UI not built) → 404.
async fn static_handler(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };

    // Serve the asset if it exists; otherwise fall back to index.html so the
    // SPA handles the route. The mime is keyed off the *served* name so the
    // fallback is delivered as text/html, not octet-stream.
    let (name, file) = match Assets::get(requested) {
        Some(f) => (requested, Some(f)),
        None => ("index.html", Assets::get("index.html")),
    };
    match file {
        Some(content) => {
            let mime = mime_guess::from_path(name).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref())],
                Body::from(content.data),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
    }
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: Arc<AppState>) {
    let (mut sink, mut stream) = socket.split();

    // Subscribe BEFORE snapshotting so no event slips through the gap; the
    // client dedups by (session_id, seq), so a brief overlap is harmless.
    let mut rx = state.hub.subscribe();

    if send_json(
        &mut sink,
        &Outbound::Sessions {
            sessions: state.hub.session_list(),
        },
    )
    .await
    .is_err()
    {
        return;
    }
    for meta in state.hub.session_list() {
        if let Some(events) = state.hub.snapshot(&meta.id) {
            if send_json(
                &mut sink,
                &Outbound::Snapshot {
                    session_id: meta.id,
                    events,
                },
            )
            .await
            .is_err()
            {
                return;
            }
        }
    }

    // Fan-out task: broadcast events → this socket.
    let mut fanout = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if send_json(&mut sink, &msg).await.is_err() {
                        break;
                    }
                }
                // Lagged: the client missed events; it can reconnect for a fresh
                // snapshot. Keep going rather than dropping the socket.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Inbound command loop.
    loop {
        tokio::select! {
            _ = &mut fanout => break,
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => handle_command(&state, &text),
                    // Other frame types (ping/pong/binary) are ignored.
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                    // Close, transport error, or stream end: tear down.
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                }
            }
        }
    }
    fanout.abort();
}

fn handle_command(state: &AppState, text: &str) {
    let cmd: Inbound = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "bad inbound command");
            return;
        }
    };
    let result = match cmd {
        Inbound::NewSession { provider, cwd } => {
            state.supervisor.new_session(&provider, cwd).map(|_| ())
        }
        Inbound::Prompt { session_id, text } => state
            .supervisor
            .send(&session_id, AgentCommand::Prompt(text)),
        Inbound::Cancel { session_id } => state.supervisor.send(&session_id, AgentCommand::Cancel),
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
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "command failed");
    }
}

async fn send_json<S>(sink: &mut S, msg: &Outbound) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

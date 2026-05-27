use anyhow::Context;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{Html, IntoResponse};
use axum::routing::{any, get};
use axum::Router;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::cli::ServeArgs;

/// Start the HTTP/WebSocket server. This is the v0 spine: it serves a
/// placeholder UI, a health check, and an echo WebSocket. The echo handler is
/// the seam where the normalized `SessionEvent`/`Command` bus fan-out (design
/// §5) will plug in.
pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    init_tracing();

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/ws", any(ws_upgrade))
        .layer(TraceLayer::new_for_http());

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

async fn index() -> impl IntoResponse {
    Html(PLACEHOLDER)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn ws_upgrade(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(ws_echo)
}

// v0 placeholder: echo text frames. Replaced by the session bus fan-out.
async fn ws_echo(mut socket: WebSocket) {
    while let Some(Ok(msg)) = socket.recv().await {
        if let Message::Text(text) = msg {
            if socket.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    }
}

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>cowboy</title></head>
<body style="font-family:system-ui;max-width:40rem;margin:4rem auto;padding:0 1rem">
<h1>🤠 cowboy</h1>
<p>Daemon is up. The web UI is not built yet — this is the v0 placeholder.</p>
<p><code>GET /healthz</code> for health, <code>/ws</code> for the (echo) WebSocket.</p>
</body>
</html>
"#;

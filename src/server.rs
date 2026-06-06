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

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context as _;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::Router;
use futures::{SinkExt, StreamExt};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use agent_client_protocol::schema::ContentBlock;

use crate::acp::AgentCommand;
use crate::cli::ServeArgs;
use crate::core::{
    DispatchReq, Hub, Inbound, Outbound, RestoredSession, SessionOrigin, Status, StoreWrite,
};
use crate::store::Store;
use crate::supervisor::Supervisor;
use tokio::sync::mpsc;

struct AppState {
    hub: Hub,
    supervisor: Arc<Supervisor>,
}

/// Start the HTTP/WebSocket server and the agent supervisor.
pub async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    init_tracing();

    // Phase 2: when --postgres-url is supplied, hook in the persistent store.
    // Migrations run on every start (sqlx tracks applied versions, so it's
    // idempotent); the in-memory Hub is then warmed from the DB before WS
    // clients can connect. Without --postgres-url the daemon falls back to
    // pure in-memory mode — same behaviour as before, useful for dev or for
    // running on a host that doesn't have the cowboy-private postgres yet.
    let (hub, store) = if let Some(url) = args.postgres_url.as_deref() {
        let store = Store::connect(url).await.context("connecting postgres")?;
        store.migrate().await.context("running migrations")?;
        let (tx, rx) = mpsc::unbounded_channel::<StoreWrite>();
        let hub = Hub::with_store(Some(tx));
        // Warm restore — sessions + events come back exactly as the daemon
        // left them, so on a fresh process every WS client's first snapshot
        // is correct.
        let loaded = store.load_all().await.context("loading persisted state")?;
        let restored: Vec<_> = loaded
            .into_iter()
            .map(|ls| RestoredSession {
                meta: ls.meta,
                log: ls.events,
                next_seq: ls.next_seq,
                queue: ls.queue,
                drafts: ls.drafts,
            })
            .collect();
        let restored_count = restored.len();
        hub.restore(restored);
        tracing::info!(
            postgres = url,
            restored = restored_count,
            "persistence wired",
        );
        // Background DB writer: dequeues StoreWrite intents and applies them.
        // Errors are logged but don't bring the daemon down — the in-memory
        // state remains authoritative for the current process.
        tokio::spawn(run_store_writer(store.clone(), rx));
        (hub, Some(store))
    } else {
        tracing::info!("no --postgres-url: running in-memory only");
        (Hub::new(), None)
    };
    drop(store); // We only kept it to thread the type; the writer holds it.

    let supervisor = Arc::new(Supervisor::new(hub.clone(), args.workspace_root.clone()));

    // Background dispatcher: the Hub owns each session's send-queue but can't
    // call the Supervisor (which holds the Hub) — that cycle is why the queue
    // used to live client-side. The Hub now makes the drain decision under its
    // lock and hands each ready prompt over this channel; we send it to the
    // agent here, off the lock. Wired before any client connects.
    let (dispatch_tx, dispatch_rx) = mpsc::unbounded_channel::<DispatchReq>();
    hub.set_dispatch_tx(dispatch_tx);
    tokio::spawn(run_dispatcher(
        hub.clone(),
        Arc::clone(&supervisor),
        dispatch_rx,
    ));

    tracing::info!(
        workspace = %args.workspace_root.display(),
        data_dir = %args.data_dir.display(),
        "cowboy serving",
    );

    serve_axum(args.bind, hub, supervisor).await
}

/// Drain the write-behind channel into postgres. One row per intent, no
/// batching for v0 — append-event is the hot one and a single INSERT per
/// envelope is fine at the volumes we'll see (a streaming claude turn is
/// ~50-200 events; postgres can handle that easily over a local socket).
async fn run_store_writer(store: Store, mut rx: mpsc::UnboundedReceiver<StoreWrite>) {
    while let Some(write) = rx.recv().await {
        let result = match &write {
            StoreWrite::InsertSession(meta) => store.insert_session(meta).await,
            StoreWrite::AppendEvent(env) => store.append_event(env).await,
            StoreWrite::UpdateStatus { session_id, status } => {
                store.update_status(session_id, *status).await
            }
            StoreWrite::UpdateTitle { session_id, title } => {
                store.update_title(session_id, title).await
            }
            StoreWrite::SetAgentSessionId {
                session_id,
                agent_session_id,
            } => {
                store
                    .update_agent_session_id(session_id, agent_session_id)
                    .await
            }
            StoreWrite::DeleteSession(id) => store.delete_session(id).await,
            StoreWrite::UpdatePending {
                session_id,
                queue,
                drafts,
            } => store.update_pending(session_id, queue, drafts).await,
            StoreWrite::UpdateSessionOrder { order } => {
                store.update_session_order(order).await
            }
        };
        if let Err(e) = result {
            tracing::warn!(error = %e, "store writer failed an intent (intent dropped)");
        }
    }
    tracing::info!("store writer shutting down (channel closed)");
}

/// Drain the Hub→dispatcher channel: each [`DispatchReq`] is a queued prompt the
/// Hub decided is ready to send. We forward it to the session's agent. On
/// success, derive the auto-title from the first prompt (a no-op after the
/// first); on failure, clear the in-flight guard (so the queue can keep
/// draining) and surface the error to every client.
async fn run_dispatcher(
    hub: Hub,
    supervisor: Arc<Supervisor>,
    mut rx: mpsc::UnboundedReceiver<DispatchReq>,
) {
    while let Some(req) = rx.recv().await {
        let DispatchReq {
            session_id,
            text,
            content,
        } = req;
        let Some(blocks) = build_prompt_blocks(&text, &content) else {
            tracing::warn!(session = %session_id, "queued prompt had no content; dropping");
            hub.clear_in_flight(&session_id);
            continue;
        };
        let title = first_prompt_title(&text, &content);
        match supervisor.send(&session_id, AgentCommand::Prompt(blocks)) {
            Ok(()) => {
                if let Some(t) = title {
                    hub.auto_title(&session_id, t);
                }
            }
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "queued dispatch failed");
                hub.clear_in_flight(&session_id);
                hub.broadcast_error(Some(session_id), format!("send failed: {e}"));
            }
        }
    }
    tracing::info!("dispatcher shutting down (channel closed)");
}

/// Build the ACP prompt blocks for a queued message: parse the stored content
/// blocks, or fall back to a single text block. Mirrors the `Inbound::Prompt`
/// handler's logic. Returns `None` for a genuinely empty prompt.
fn build_prompt_blocks(text: &str, content: &[serde_json::Value]) -> Option<Vec<ContentBlock>> {
    if content.is_empty() {
        if text.is_empty() {
            return None;
        }
        return Some(vec![ContentBlock::from(text.to_owned())]);
    }
    let blocks: Vec<ContentBlock> = content
        .iter()
        .filter_map(|v| match serde_json::from_value::<ContentBlock>(v.clone()) {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::warn!(error = %e, "skipping unparseable queued content block");
                None
            }
        })
        .collect();
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

async fn serve_axum(
    bind: std::net::SocketAddr,
    hub: Hub,
    supervisor: Arc<Supervisor>,
) -> anyhow::Result<()> {
    let state = Arc::new(AppState { hub, supervisor });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/version", get(version))
        .route("/api/sessions", post(api_new_session))
        .route("/api/sessions/{id}/files", get(api_search_files))
        .route("/ws", any(ws_upgrade))
        // Everything else: the embedded SPA, with index.html fallback for
        // client-side routes.
        .fallback(static_handler)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(addr = %bind, "WS/HTTP listening");

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

/// Response body for `GET /version`.
#[derive(Debug, Serialize)]
struct VersionResponse {
    version: String,
}

/// A build identifier the SPA polls to detect a redeploy. We reuse the embedded
/// `index.html`'s compile-time SHA256: it references the content-hashed JS/CSS
/// bundles, so any change to the shipped UI changes this hash, while an
/// unchanged build keeps it stable. The frontend captures it on first load and
/// re-checks after each WS reconnect — a mismatch means the daemon was
/// redeployed under a now-stale tab, which surfaces the "new version" banner.
async fn version() -> Response {
    match Assets::get("index.html") {
        Some(f) => Json(VersionResponse {
            version: content_hash_hex(&f.metadata.sha256_hash()),
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
    }
}

/// Render rust-embed's compile-time SHA256 as a 32-hex-char string (first 16
/// bytes — ample to avoid collisions across a handful of static files). Used
/// both for the static-asset `ETag` and the `/version` build id so the two
/// never drift.
fn content_hash_hex(hash: &[u8; 32]) -> String {
    format!(
        "{:016x}{:016x}",
        u64::from_be_bytes(hash[0..8].try_into().expect("8 bytes")),
        u64::from_be_bytes(hash[8..16].try_into().expect("8 bytes")),
    )
}

/// Request body for `POST /api/sessions`.
///
/// WS `Inbound::NewSession` is fire-and-forget without a `sessionId` reply, so
/// external drivers (e.g. the `acp-bridge` translating ACP `session/new`)
/// would have to diff `Outbound::Sessions` broadcasts to learn their id —
/// racey. This endpoint exists so a single synchronous HTTP request returns
/// the assigned id directly. Web UI clients can keep using the WS path;
/// this is purely additive.
#[derive(Debug, Deserialize)]
struct NewSessionRequest {
    provider: String,
    #[serde(default)]
    cwd: Option<String>,
    /// Which surface opened the session — defaults to `Api` for direct
    /// `curl`/test callers. `acp-bridge` sends `Zed`. The Web UI uses the WS
    /// `Inbound::NewSession` path (which always tags `Web`), not this
    /// endpoint, so `Web` shouldn't normally arrive here.
    #[serde(default)]
    origin: SessionOrigin,
}

/// Response body for `POST /api/sessions`.
#[derive(Debug, Serialize)]
struct NewSessionResponse {
    session_id: String,
}

async fn api_new_session(
    State(state): State<Arc<AppState>>,
    Json(req): Json<NewSessionRequest>,
) -> Response {
    match state
        .supervisor
        .new_session(&req.provider, req.cwd, req.origin)
    {
        Ok(session_id) => {
            (StatusCode::CREATED, Json(NewSessionResponse { session_id })).into_response()
        }
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}

/// Query string for `GET /api/sessions/{id}/files` — the composer's `@` picker.
#[derive(Debug, Deserialize)]
struct FileSearchQuery {
    /// Fuzzy query; empty returns the "most useful" files (shallow + recent).
    #[serde(default)]
    q: String,
    #[serde(default = "default_file_limit")]
    limit: usize,
}

fn default_file_limit() -> usize {
    20
}

#[derive(Debug, Serialize)]
struct FileSearchResponse {
    files: Vec<String>,
}

/// Rank files under a session's working directory for the `@` reference picker.
///
/// The cwd comes from the session itself (never from the client) so a browser
/// can't walk arbitrary paths. The walk + fuzzy match is blocking, so it runs
/// on a blocking thread; a missing session is `404`, an empty tree is `200`
/// with `[]`.
async fn api_search_files(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Query(query): Query<FileSearchQuery>,
) -> Response {
    let Some(cwd) = state
        .hub
        .session_list()
        .into_iter()
        .find(|m| m.id == session_id)
        .map(|m| m.cwd)
    else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    let limit = query.limit.clamp(1, 100);
    let files = tokio::task::spawn_blocking(move || {
        crate::files::search(std::path::Path::new(&cwd), &query.q, limit)
    })
    .await
    .unwrap_or_default();
    Json(FileSearchResponse { files }).into_response()
}

/// The built web UI (Vite output), embedded at compile time. The flake builds
/// `web/dist` with deno before the cargo build so this folder exists.
#[derive(RustEmbed)]
#[folder = "web/dist"]
struct Assets;

/// Serve an embedded asset by path, falling back to `index.html` so the SPA
/// owns client-side routing. Missing `index.html` (UI not built) → 404.
///
/// Caching: rust-embed computes a per-file SHA256 at compile time, which we use
/// as a content `ETag` (stable across rebuilds when the bytes are unchanged). The
/// cache policy is split by whether the filename is content-addressed:
///   - `/assets/*` — Vite emits content-hashed names, so the bytes behind a name
///     never change → `immutable` with a one-year max-age, never revalidated.
///   - everything else (index.html, sw.js, manifest, favicon, icons) —
///     `no-cache`: the browser may store it but MUST revalidate via the `ETag` on
///     every use, so a redeploy is picked up immediately while unchanged files
///     cost only a 304. This is what stops a redeployed favicon/icon from being
///     pinned to a stale copy in the browser's HTTP cache.
async fn static_handler(uri: Uri, headers: HeaderMap) -> Response {
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
    let Some(content) = file else {
        return (StatusCode::NOT_FOUND, "UI not built").into_response();
    };

    // Content ETag from rust-embed's compile-time SHA256 (same hash the
    // `/version` build id uses, so the two stay in lockstep).
    let etag = format!("\"{}\"", content_hash_hex(&content.metadata.sha256_hash()));

    // Conditional request: the browser echoes our ETag in If-None-Match; if it
    // still matches, skip the body. `contains` (not strict equality) tolerates a
    // comma-list or a `W/` weak prefix some clients send.
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if inm.contains(etag.as_str()) {
            return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag.as_str())]).into_response();
        }
    }

    let cache_control = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache_control),
            (header::ETAG, etag.as_str()),
        ],
        Body::from(content.data),
    )
        .into_response()
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
                    session_id: meta.id.clone(),
                    events,
                },
            )
            .await
            .is_err()
            {
                return;
            }
        }
        // Replay the last-seen agent config options so the composer's
        // mode / model / effort dropdowns hydrate on first paint instead of
        // waiting for the next `config_option_update` to fire upstream.
        if let Some(options) = state.hub.config_options(&meta.id) {
            if send_json(
                &mut sink,
                &Outbound::ConfigOptions {
                    session_id: meta.id.clone(),
                    options,
                },
            )
            .await
            .is_err()
            {
                return;
            }
        }
        // Replay the server-authoritative queue + drafts so a freshly-opened
        // terminal renders the same staged messages as every other one.
        if let Some((queue, drafts)) = state.hub.pending(&meta.id) {
            if send_json(
                &mut sink,
                &Outbound::Queues {
                    session_id: meta.id,
                    queue,
                    drafts,
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

    // Edit-holds this connection set (session_id → held queued-message id). The
    // editing hold is GLOBAL server state, so a client that disconnects mid-edit
    // would otherwise leave the head pinned and stall the queue forever. We
    // track what this socket held and release it on teardown.
    let mut held: HashMap<String, String> = HashMap::new();

    // Inbound command loop.
    loop {
        tokio::select! {
            _ = &mut fanout => break,
            msg = stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => handle_command(&state, &text, &mut held),
                    // Other frame types (ping/pong/binary) are ignored.
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Binary(_))) => {}
                    // Close, transport error, or stream end: tear down.
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                }
            }
        }
    }
    // Release any edit-holds this connection still owns so the queue can drain.
    for session_id in held.keys() {
        state.hub.set_queue_editing(session_id, None);
    }
    fanout.abort();
}

/// Derive a short session title from the first prompt: the first non-empty
/// line, whitespace-collapsed and truncated. Prefers the legacy `text` field;
/// falls back to the first text block in `content` (attachment prompts carry
/// their text there). Returns None for an attachment-only / empty prompt.
fn first_prompt_title(text: &str, content: &[serde_json::Value]) -> Option<String> {
    // Cap length on a char boundary so a long first line stays a label, not a
    // paragraph.
    const MAX: usize = 60;
    let raw = if text.trim().is_empty() {
        content.iter().find_map(|v| {
            if v.get("type").and_then(serde_json::Value::as_str) == Some("text") {
                v.get("text")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            } else {
                None
            }
        })?
    } else {
        text.to_owned()
    };
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    // Append an ellipsis when the first line is cut to MAX.
    if collapsed.chars().count() > MAX {
        let head: String = collapsed.chars().take(MAX).collect();
        Some(format!("{head}…"))
    } else {
        Some(collapsed)
    }
}

#[allow(clippy::too_many_lines)] // one cohesive command-dispatch match
fn handle_command(state: &AppState, text: &str, held: &mut HashMap<String, String>) {
    let cmd: Inbound = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "bad inbound command");
            state
                .hub
                .broadcast_error(None, format!("bad inbound command: {e}"));
            return;
        }
    };
    // Capture session_id ahead of the match for error attribution. Most
    // commands carry one; NewSession doesn't (the session id is assigned
    // by the daemon after success).
    let session_id_for_err: Option<String> = match &cmd {
        Inbound::Prompt { session_id, .. }
        | Inbound::Cancel { session_id }
        | Inbound::Permission { session_id, .. }
        | Inbound::DeleteSession { session_id }
        | Inbound::RenameSession { session_id, .. }
        | Inbound::SetConfigOption { session_id, .. }
        | Inbound::OpenSession { session_id }
        | Inbound::Submit { session_id, .. }
        | Inbound::RemoveQueued { session_id, .. }
        | Inbound::EditQueued { session_id, .. }
        | Inbound::ClearQueue { session_id }
        | Inbound::RequestSendQueued { session_id, .. }
        | Inbound::ForcePushQueued { session_id, .. }
        | Inbound::QueuedToDraft { session_id, .. }
        | Inbound::SetQueueEditing { session_id, .. }
        | Inbound::AddDraft { session_id, .. }
        | Inbound::EditDraft { session_id, .. }
        | Inbound::RemoveDraft { session_id, .. }
        | Inbound::ClearDrafts { session_id }
        | Inbound::ActivateDraft { session_id, .. }
        | Inbound::ActivateAllDrafts { session_id }
        | Inbound::ReorderQueue { session_id, .. }
        | Inbound::ReorderDrafts { session_id, .. } => Some(session_id.clone()),
        Inbound::NewSession { .. } | Inbound::ReorderSessions { .. } => None,
    };
    let result = match cmd {
        Inbound::NewSession { provider, cwd } => state
            .supervisor
            .new_session(&provider, cwd, SessionOrigin::Web)
            .map(|_| ()),
        Inbound::Prompt {
            session_id,
            text,
            content,
        } => {
            // Derive an auto-title from the first prompt before text/content are
            // consumed below. auto_title no-ops unless the title is still the
            // cwd default, so this only "takes" on a session's first prompt and
            // never overrides a manual rename.
            let auto = first_prompt_title(&text, &content);
            let blocks: Vec<ContentBlock> = if content.is_empty() {
                if text.is_empty() {
                    tracing::warn!("Prompt with neither text nor content; dropping");
                    state.hub.broadcast_error(
                        Some(session_id),
                        "empty prompt: no text or content blocks".to_owned(),
                    );
                    return;
                }
                vec![ContentBlock::from(text)]
            } else {
                content
                    .into_iter()
                    .filter_map(|v| match serde_json::from_value::<ContentBlock>(v) {
                        Ok(b) => Some(b),
                        Err(e) => {
                            tracing::warn!(error = %e, "skipping unparseable Prompt content block");
                            None
                        }
                    })
                    .collect()
            };
            let result = state
                .supervisor
                .send(&session_id, AgentCommand::Prompt(blocks));
            if result.is_ok() {
                if let Some(title) = auto {
                    state.hub.auto_title(&session_id, title);
                }
            }
            result
        }
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
        Inbound::DeleteSession { session_id } => {
            // Order: tear down agent thread first (so it doesn't push more
            // events into a soon-to-be-gone Hub session), then drop Hub state
            // + broadcast updated list.
            state.supervisor.delete_session(&session_id);
            state.hub.delete_session(&session_id);
            Ok(())
        }
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
        Inbound::SetConfigOption {
            session_id,
            config_id,
            value,
        } => state.supervisor.send(
            &session_id,
            AgentCommand::SetConfigOption { config_id, value },
        ),
        // Revive on open (design §7): warm the agent when the client selects
        // the session, not only on the first prompt. No-op if already alive.
        Inbound::OpenSession { session_id } => {
            // A client opens the focus it restored from localStorage on reload.
            // If that session is gone (deleted while the client was away), this
            // is NOT an error condition: the client already pops a one-shot
            // *warning* snackbar and falls back to another session. Log a
            // server-side warning and swallow the error so no error toast is
            // broadcast (which would otherwise read as a hard failure).
            match state.supervisor.ensure_alive(&session_id) {
                Ok(_) => Ok(()),
                Err(e) => {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %e,
                        "open of unknown/gone session ignored — client will fall back",
                    );
                    Ok(())
                }
            }
        }

        // --- Server-authoritative queue + drafts ------------------------------
        // These mutate Hub state, which broadcasts the new queue/drafts to every
        // terminal. They never fail in a way worth a toast, so all return Ok.
        Inbound::Submit {
            session_id,
            text,
            content,
        } => {
            state.hub.submit(&session_id, text, content);
            Ok(())
        }
        Inbound::RemoveQueued { session_id, id } => {
            state.hub.remove_queued(&session_id, &id);
            Ok(())
        }
        Inbound::EditQueued {
            session_id,
            id,
            text,
            content,
        } => {
            state.hub.edit_queued(&session_id, &id, text, content);
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
                state.supervisor.send(&session_id, AgentCommand::Cancel)
            } else {
                Ok(())
            }
        }
        Inbound::QueuedToDraft { session_id, id } => {
            state.hub.queued_to_draft(&session_id, &id);
            Ok(())
        }
        Inbound::SetQueueEditing { session_id, id } => {
            // Track the hold per-connection so a mid-edit disconnect releases it
            // (the hold is global server state — see handle_ws teardown).
            match &id {
                Some(mid) => {
                    held.insert(session_id.clone(), mid.clone());
                }
                None => {
                    held.remove(&session_id);
                }
            }
            state.hub.set_queue_editing(&session_id, id);
            Ok(())
        }
        Inbound::AddDraft {
            session_id,
            text,
            content,
        } => {
            state.hub.add_draft(&session_id, text, content);
            Ok(())
        }
        Inbound::EditDraft {
            session_id,
            id,
            text,
            content,
        } => {
            state.hub.edit_draft(&session_id, &id, text, content);
            Ok(())
        }
        Inbound::RemoveDraft { session_id, id } => {
            state.hub.remove_draft(&session_id, &id);
            Ok(())
        }
        Inbound::ClearDrafts { session_id } => {
            state.hub.clear_drafts(&session_id);
            Ok(())
        }
        Inbound::ActivateDraft { session_id, id } => {
            state.hub.activate_draft(&session_id, &id);
            Ok(())
        }
        Inbound::ActivateAllDrafts { session_id } => {
            state.hub.activate_all_drafts(&session_id);
            Ok(())
        }
        Inbound::ReorderSessions { order } => {
            state.hub.reorder_sessions(&order);
            Ok(())
        }
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
    }
}

async fn send_json<S>(sink: &mut S, msg: &Outbound) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sink.send(Message::Text(text.into())).await.map_err(|_| ())
}

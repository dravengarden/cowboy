//! Loopback-only experiment server. /fixture/* is a trusted test harness,
//! not pairing or authentication suitable for a product.
use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::{get, post},
};
use cowboy_browser_wireguard_probe::{Peer, public_key, secret};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tower_http::services::ServeDir;

struct Fixture {
    secret: [u8; 32],
    peer: Option<Peer>,
    revoked_packets: u64,
    last_reply: Option<Vec<u8>>,
    injected: Vec<Vec<u8>>,
    run: PathBuf,
}
type Shared = Arc<Mutex<Fixture>>;

#[derive(Deserialize)]
struct Registration {
    public_key: Option<[u8; 32]>,
}

async fn config(State(state): State<Shared>) -> Json<Value> {
    Json(json!({"server_public": public_key(&state.lock().unwrap().secret)}))
}
async fn register(State(state): State<Shared>, Json(body): Json<Registration>) -> Json<Value> {
    let mut state = state.lock().unwrap();
    state.peer = body
        .public_key
        .map(|key| Peer::new(state.secret, key, true));
    state.last_reply = None;
    state.injected.clear();
    Json(json!({"registered": state.peer.is_some()}))
}
async fn stats(State(state): State<Shared>) -> Json<Value> {
    let state = state.lock().unwrap();
    Json(
        json!({"peer":state.peer.as_ref().map(|peer| &peer.counters), "revoked_packets":state.revoked_packets}),
    )
}
async fn inject(State(state): State<Shared>) -> Json<Value> {
    let mut state = state.lock().unwrap();
    if let Some(packet) = state.last_reply.clone() {
        state.injected.push(packet);
    }
    Json(json!({"queued":state.injected.len()}))
}
async fn push(State(state): State<Shared>, Json(payload): Json<Vec<u8>>) -> Json<Value> {
    assert!(payload.len() <= 1200);
    if let Some(peer) = &mut state.lock().unwrap().peer {
        peer.send(&payload, false);
    }
    Json(json!({"queued":true}))
}
async fn report(State(state): State<Shared>, Json(report): Json<Value>) -> Json<Value> {
    let path = state.lock().unwrap().run.join("report.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    Json(json!({"ok":true}))
}
async fn progress(State(state): State<Shared>, Json(progress): Json<Value>) -> Json<Value> {
    let path = state.lock().unwrap().run.join("progress.json");
    std::fs::write(path, serde_json::to_vec(&progress).unwrap()).unwrap();
    Json(json!({"ok":true}))
}
async fn upgrade(ws: WebSocketUpgrade, State(state): State<Shared>) -> impl IntoResponse {
    ws.max_message_size(1344)
        .max_frame_size(1344)
        .on_upgrade(move |socket| carrier(socket, state))
}

fn drain(state: &mut Fixture) -> Vec<Vec<u8>> {
    let mut out = std::mem::take(&mut state.injected);
    if let Some(peer) = &mut state.peer {
        while let Some(payload) = peer.pop_payload() {
            peer.send(&payload, false);
        }
        while let Some(packet) = peer.pop_network() {
            // Every fixture data payload is nonempty. A keepalive is 32 bytes.
            if packet.first() == Some(&4) && packet.len() > 32 {
                state.last_reply = Some(packet.clone());
            }
            out.push(packet);
        }
    }
    out
}

async fn carrier(mut socket: WebSocket, state: Shared) {
    let mut timer = tokio::time::interval(std::time::Duration::from_millis(50));
    loop {
        let out = tokio::select! {
            message = socket.recv() => {
                let Some(Ok(message)) = message else { break };
                match message {
                    Message::Binary(packet) => {
                        let mut state = state.lock().unwrap();
                        if let Some(peer) = &mut state.peer { peer.receive(&packet); }
                        else { state.revoked_packets += 1; }
                        drain(&mut state)
                    }
                    Message::Close(_) => break,
                    _ => Vec::new(),
                }
            }
            _ = timer.tick() => {
                let mut state = state.lock().unwrap();
                if let Some(peer) = &mut state.peer { peer.tick(); }
                drain(&mut state)
            }
        };
        for packet in out {
            if socket.send(Message::Binary(packet.into())).await.is_err() {
                return;
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(
        args.len(),
        2,
        "assets directory and disposable run directory required"
    );
    let run = PathBuf::from(&args[1]);
    let state = Arc::new(Mutex::new(Fixture {
        secret: secret(),
        peer: None,
        revoked_packets: 0,
        last_reply: None,
        injected: Vec::new(),
        run: run.clone(),
    }));
    let app = Router::new()
        .route("/fixture/config", get(config))
        .route("/fixture/peer", post(register))
        .route("/fixture/stats", get(stats))
        .route("/fixture/inject-last", post(inject))
        .route("/fixture/push", post(push))
        .route("/fixture/report", post(report))
        .route("/fixture/progress", post(progress))
        .route("/wireguard", get(upgrade))
        .fallback_service(ServeDir::new(PathBuf::from(&args[0])))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::fs::write(
        run.join("backend"),
        listener.local_addr().unwrap().to_string(),
    )
    .unwrap();
    axum::serve(listener, app).await.unwrap();
}

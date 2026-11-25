//! Independent authenticated HTTPS post-edit diagnostic transport for
//! `machine_cli` (Decision 2A / R4).
//!
//! Zero diagnostic bytes enter the primary Machine WebSocket (`machine_transport`).
//! Every request is executed out-of-band over HTTPS with the per-connection
//! `x-cowboy-machine-post-edit-token`, strictly bounded concurrency (`Semaphore(4)`),
//! no HTTP redirects (`Policy::none()`, R3-M16), 8 KiB request / 16 KiB reply
//! body caps, and immediate cancellation when the primary Machine WebSocket ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use tokio::sync::{Semaphore, mpsc, watch};

use crate::post_edit_inspect::{
    MAX_POST_EDIT_REPLY_BYTES, MAX_POST_EDIT_REQUEST_BYTES, POST_EDIT_INSPECT_HEADER,
    POST_EDIT_INSPECT_PATH, POST_EDIT_INSPECT_PROTOCOL, POST_EDIT_INSPECT_TOKEN_HEADER,
    PostEditInspectReply, PostEditInspectRequest, clamp_remaining_ms,
};
use crate::runtime_wire::Frame;

pub(super) const DIAG_RUNTIME_REPLY_CAPACITY: usize = 8;

pub(super) fn attach_upgrade_header(
    request: &mut tokio_tungstenite::tungstenite::http::Request<()>,
) -> anyhow::Result<()> {
    request.headers_mut().insert(
        POST_EDIT_INSPECT_HEADER,
        POST_EDIT_INSPECT_PROTOCOL.parse()?,
    );
    Ok(())
}

pub(super) fn negotiate_client(
    ws_endpoint: &url::Url,
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
) -> Option<PostEditHttpClient> {
    let token = response
        .headers()
        .get(POST_EDIT_INSPECT_HEADER)
        .is_some_and(|value| value == POST_EDIT_INSPECT_PROTOCOL)
        .then(|| {
            response
                .headers()
                .get(POST_EDIT_INSPECT_TOKEN_HEADER)
                .and_then(|value| value.to_str().ok())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .flatten()?;
    PostEditHttpClient::from_ws_endpoint(ws_endpoint, token).ok()
}

pub(super) fn intercept_local_frame(
    frame: Frame,
    client: Option<&PostEditHttpClient>,
    reply_tx: &mpsc::Sender<Frame>,
) -> Option<Frame> {
    match frame {
        Frame::PostEditInspectRequest { request } => {
            if let Some(client) = client {
                client.dispatch(*request, reply_tx.clone());
            } else {
                send_refusal(reply_tx, &request, "unsupported");
            }
            None
        }
        Frame::PostEditInspectReply { .. } => {
            tracing::warn!("dropping wrong-direction PostEditInspectReply from local broker");
            None
        }
        other => Some(other),
    }
}
const MAX_CONCURRENT_POST_EDIT_HTTP: usize = 4;

pub(super) struct PostEditHttpClient {
    http: reqwest::Client,
    endpoint_url: url::Url,
    token: String,
    slots: Arc<Semaphore>,
    cancel_tx: watch::Sender<bool>,
}

impl Drop for PostEditHttpClient {
    fn drop(&mut self) {
        let _ = self.cancel_tx.send(true);
    }
}

impl PostEditHttpClient {
    /// Build an independent HTTPS client strictly targeting the authenticated
    /// primary WebSocket's origin (`{scheme}://{host}[:port]/api/machine/post-edit-inspect`)
    /// with redirects disabled (`R3-M16`).
    pub(super) fn from_ws_endpoint(ws_endpoint: &url::Url, token: String) -> anyhow::Result<Self> {
        let mut endpoint_url = ws_endpoint.clone();
        let http_scheme = match ws_endpoint.scheme() {
            "ws" => "http",
            "wss" => "https",
            other => anyhow::bail!("unsupported Machine endpoint scheme for post-edit: {other}"),
        };
        endpoint_url
            .set_scheme(http_scheme)
            .map_err(|()| anyhow::anyhow!("failed setting post-edit HTTP scheme"))?;
        endpoint_url.set_path(POST_EDIT_INSPECT_PATH);
        endpoint_url.set_query(None);
        endpoint_url.set_fragment(None);

        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let (cancel_tx, _) = watch::channel(false);
        Ok(Self {
            http,
            endpoint_url,
            token,
            slots: Arc::new(Semaphore::new(MAX_CONCURRENT_POST_EDIT_HTTP)),
            cancel_tx,
        })
    }

    pub(super) fn dispatch(&self, request: PostEditInspectRequest, reply_tx: mpsc::Sender<Frame>) {
        if !request.validate_structure() {
            send_refusal(&reply_tx, &request, "invalid_request");
            return;
        }
        let Some(clamped_ms) = clamp_remaining_ms(request.remaining_ms) else {
            send_refusal(&reply_tx, &request, "timeout");
            return;
        };
        let deadline = Instant::now() + Duration::from_millis(clamped_ms);
        let Ok(permit) = Arc::clone(&self.slots).try_acquire_owned() else {
            send_refusal(&reply_tx, &request, "capacity");
            return;
        };
        let http = self.http.clone();
        let url = self.endpoint_url.clone();
        let token = self.token.clone();
        let mut cancel_rx = self.cancel_tx.subscribe();
        tokio::spawn(async move {
            let _permit = permit;
            let outcome = tokio::select! {
                biased;
                _ = cancel_rx.wait_for(|cancelled| *cancelled) => return,
                reply = execute_http_at_deadline(http, url, token, request, deadline) => reply,
            };
            let frame = Frame::PostEditInspectReply {
                reply: Box::new(outcome),
            };
            if serde_json::to_vec(&frame)
                .is_ok_and(|bytes| bytes.len() <= MAX_POST_EDIT_REPLY_BYTES)
            {
                let _ = reply_tx.try_send(frame);
            }
        });
    }
}

async fn execute_http_at_deadline(
    http: reqwest::Client,
    url: url::Url,
    token: String,
    mut request: PostEditInspectRequest,
    deadline: Instant,
) -> PostEditInspectReply {
    let rem = u64::try_from(
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis(),
    )
    .unwrap_or(u64::MAX);
    let Some(send_ms) = clamp_remaining_ms(rem) else {
        return refusal_for(&request, "timeout");
    };
    request.remaining_ms = send_ms;
    let Ok(body) = serde_json::to_vec(&request) else {
        return refusal_for(&request, "invalid_request");
    };
    if body.len() > MAX_POST_EDIT_REQUEST_BYTES {
        return refusal_for(&request, "invalid_request");
    }
    execute_http(http, url, token, body, &request, deadline).await
}

async fn execute_http(
    http: reqwest::Client,
    url: url::Url,
    token: String,
    body: Vec<u8>,
    request: &PostEditInspectRequest,
    deadline: Instant,
) -> PostEditInspectReply {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return refusal_for(request, "timeout");
    }
    let send_fut = http
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(POST_EDIT_INSPECT_TOKEN_HEADER, token)
        .body(body)
        .send();
    let response = match tokio::time::timeout(remaining, send_fut).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => return refusal_for(request, "unavailable"),
        Err(_) => return refusal_for(request, "timeout"),
    };
    let status = response.status();
    if status.is_redirection() {
        // R3-M16: redirects are never followed and treated as unavailable.
        return refusal_for(request, "unavailable");
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return refusal_for(request, "restricted");
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return refusal_for(request, "capacity");
    }
    if !status.is_success() {
        return refusal_for(request, "unavailable");
    }

    let mut bytes = Vec::with_capacity(2048);
    let mut stream = response.bytes_stream();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return refusal_for(request, "timeout");
        }
        match tokio::time::timeout(left, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                if bytes.len().saturating_add(chunk.len()) > MAX_POST_EDIT_REPLY_BYTES {
                    return refusal_for(request, "unavailable");
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(Some(Err(_))) => return refusal_for(request, "unavailable"),
            Ok(None) => break,
            Err(_) => return refusal_for(request, "timeout"),
        }
    }

    let Ok(reply) = serde_json::from_slice::<PostEditInspectReply>(&bytes) else {
        return refusal_for(request, "unavailable");
    };
    if !reply.validate()
        || reply.session_id != request.session_id
        || reply.worker_epoch != request.worker_epoch
        || reply.request_id != request.request_id
        || reply.scope != request.scope
    {
        return refusal_for(request, "unavailable");
    }
    reply
}

fn refusal_for(request: &PostEditInspectRequest, reason: &'static str) -> PostEditInspectReply {
    PostEditInspectReply::refused(
        request.session_id.clone(),
        request.worker_epoch.clone(),
        request.request_id.clone(),
        request.scope.clone(),
        reason,
    )
}

pub(super) fn send_refusal(
    reply_tx: &mpsc::Sender<Frame>,
    request: &PostEditInspectRequest,
    reason: &'static str,
) {
    let _ = reply_tx.try_send(Frame::PostEditInspectReply {
        reply: Box::new(refusal_for(request, reason)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_edit_inspect::PostEditInspectOutcome;
    use crate::post_edit_inspect::PostEditScope;
    use axum::{Router, extract::Request, http::StatusCode, response::IntoResponse, routing::post};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn sample_req(id: &str) -> PostEditInspectRequest {
        PostEditInspectRequest {
            session_id: "sess-1".into(),
            worker_epoch: "epoch-1".into(),
            request_id: id.into(),
            turn_id: "turn-1".into(),
            scope: PostEditScope {
                binding_id: "bind-1".into(),
                revision: 1,
                incarnation: "inc-1".into(),
            },
            path: "src/lib.rs".into(),
            expected_lf_sha256: "a".repeat(64),
            remaining_ms: 4_000,
            canary_inspect_id: None,
        }
    }

    #[tokio::test]
    async fn authenticates_independent_http_endpoint_and_blocks_redirects() {
        let redirect_hits = Arc::new(AtomicUsize::new(0));
        let redirect_hits_srv = Arc::clone(&redirect_hits);
        let app = Router::new()
            .route(
                POST_EDIT_INSPECT_PATH,
                post(|req: Request| async move {
                    let token = req
                        .headers()
                        .get(POST_EDIT_INSPECT_TOKEN_HEADER)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    if token != "secret-token-1" {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    let body = axum::body::to_bytes(req.into_body(), MAX_POST_EDIT_REQUEST_BYTES)
                        .await
                        .unwrap();
                    let parsed: PostEditInspectRequest = serde_json::from_slice(&body).unwrap();
                    if parsed.request_id == "redirect-test" {
                        return (
                            StatusCode::TEMPORARY_REDIRECT,
                            [(axum::http::header::LOCATION, "/leaked")],
                        )
                            .into_response();
                    }
                    if parsed.request_id == "oversized-test" {
                        return (StatusCode::OK, "x".repeat(MAX_POST_EDIT_REPLY_BYTES + 64))
                            .into_response();
                    }
                    let reply = PostEditInspectReply {
                        session_id: parsed.session_id,
                        worker_epoch: parsed.worker_epoch,
                        request_id: parsed.request_id,
                        scope: parsed.scope,
                        outcome: PostEditInspectOutcome::Projected {
                            payload: serde_json::json!({"path": parsed.path, "deliver": true}),
                            matches_expected: true,
                        },
                    };
                    (StatusCode::OK, serde_json::to_vec(&reply).unwrap()).into_response()
                }),
            )
            .route(
                "/leaked",
                post(move || {
                    let hits = Arc::clone(&redirect_hits_srv);
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let ws_url = url::Url::parse(&format!("ws://{addr}/api/machine/connect")).unwrap();
        let client =
            PostEditHttpClient::from_ws_endpoint(&ws_url, "secret-token-1".into()).unwrap();
        let (tx, mut rx) = mpsc::channel(DIAG_RUNTIME_REPLY_CAPACITY);

        // 1. Happy path returns projected payload.
        client.dispatch(sample_req("ok-1"), tx.clone());
        let Frame::PostEditInspectReply { reply } = rx.recv().await.unwrap() else {
            panic!("expected reply");
        };
        assert_eq!(reply.request_id, "ok-1");
        assert!(matches!(
            reply.outcome,
            PostEditInspectOutcome::Projected {
                matches_expected: true,
                ..
            }
        ));

        // 2. Redirect is never followed (R3-M16) and never leaks token to /leaked.
        client.dispatch(sample_req("redirect-test"), tx.clone());
        let Frame::PostEditInspectReply { reply } = rx.recv().await.unwrap() else {
            panic!("expected reply");
        };
        assert_eq!(
            reply.outcome,
            PostEditInspectOutcome::Refused {
                reason: "unavailable".into()
            }
        );
        assert_eq!(redirect_hits.load(Ordering::SeqCst), 0);

        // 3. Oversized response (> 16 KiB) is refused cleanly.
        client.dispatch(sample_req("oversized-test"), tx.clone());
        let Frame::PostEditInspectReply { reply } = rx.recv().await.unwrap() else {
            panic!("expected reply");
        };
        assert_eq!(
            reply.outcome,
            PostEditInspectOutcome::Refused {
                reason: "unavailable".into()
            }
        );

        server.abort();
    }
}

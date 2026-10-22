//! A repeated `Idempotency-Key` on a session-creation route returns the first
//! successful response instead of creating a second session.
//!
//! Creation allocates a fresh session id per request, so a client that submits
//! twice (a double activation, or a retry after a lost response) would
//! otherwise own two sessions. Only success is retained: a failed attempt
//! leaves the key free so the same request can be retried.

use super::AuthenticatedProductRequest;
use axum::{
    body::Bytes,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;

const KEY_HEADER: &str = "idempotency-key";
const MAX_KEY_BYTES: usize = 128;
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Long enough to cover a retry after a lost response; a client mints a new
/// key for every distinct creation.
const RETENTION: Duration = Duration::from_secs(600);

#[derive(Clone)]
struct Created {
    status: StatusCode,
    content_type: Option<HeaderValue>,
    body: Bytes,
}

impl Created {
    fn response(&self) -> Response {
        let mut response = (self.status, self.body.clone()).into_response();
        match &self.content_type {
            Some(value) => response
                .headers_mut()
                .insert(header::CONTENT_TYPE, value.clone()),
            None => response.headers_mut().remove(header::CONTENT_TYPE),
        };
        response
    }
}

struct Entry {
    claimed: Instant,
    created: Arc<OnceCell<Created>>,
}

#[derive(Default)]
pub(super) struct Replays(parking_lot::Mutex<HashMap<String, Entry>>);

impl Replays {
    fn claim(&self, scope: String, now: Instant) -> Arc<OnceCell<Created>> {
        let mut entries = self.0.lock();
        entries.retain(|_, entry| now.duration_since(entry.claimed) < RETENTION);
        Arc::clone(
            &entries
                .entry(scope)
                .or_insert_with(|| Entry {
                    claimed: now,
                    created: Arc::default(),
                })
                .created,
        )
    }

    /// Concurrent callers of one scope wait for the first; later ones replay it.
    async fn create_once(&self, scope: String, create: impl Future<Output = Response>) -> Response {
        let created = self.claim(scope, Instant::now());
        match created
            .get_or_try_init(|| async { capture(create.await).await })
            .await
        {
            Ok(created) => created.response(),
            Err(response) => *response,
        }
    }
}

async fn capture(response: Response) -> Result<Created, Box<Response>> {
    if !response.status().is_success() {
        return Err(Box::new(response));
    }
    let (parts, body) = response.into_parts();
    match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(body) => Ok(Created {
            status: parts.status,
            content_type: parts.headers.get(header::CONTENT_TYPE).cloned(),
            body,
        }),
        Err(_) => Err(Box::new(StatusCode::INTERNAL_SERVER_ERROR.into_response())),
    }
}

pub(super) async fn replay(
    State(replays): State<Arc<Replays>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(key) = request.headers().get(KEY_HEADER) else {
        return next.run(request).await;
    };
    let key = match key.to_str() {
        Ok(key) if !key.is_empty() && key.len() <= MAX_KEY_BYTES => key,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "Idempotency-Key must be 1 to 128 visible ASCII characters",
            )
                .into_response();
        }
    };
    // One user's key never replays another user's session, and a key reused
    // across the two creation routes names two different requests.
    let owner = request
        .extensions()
        .get::<AuthenticatedProductRequest>()
        .map_or("", |authenticated| authenticated.principal.user_id.as_str());
    let scope = format!("{owner}\n{}\n{key}", request.uri().path());
    replays.create_once(scope, next.run(request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn body(response: Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), MAX_BODY_BYTES)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    async fn created(calls: &AtomicUsize) -> Response {
        let id = calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        (
            StatusCode::CREATED,
            Json(serde_json::json!({ "session_id": format!("sess-{id}") })),
        )
            .into_response()
    }

    #[tokio::test]
    async fn concurrent_and_later_repeats_return_the_first_session() {
        let replays = Replays::default();
        let calls = AtomicUsize::new(0);
        let (first, second) = tokio::join!(
            replays.create_once("user\n/api/sessions\nkey".into(), created(&calls)),
            replays.create_once("user\n/api/sessions\nkey".into(), created(&calls)),
        );
        let third = replays
            .create_once("user\n/api/sessions\nkey".into(), created(&calls))
            .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(third.status(), StatusCode::CREATED);
        assert_eq!(
            third.headers().get(header::CONTENT_TYPE),
            first.headers().get(header::CONTENT_TYPE)
        );
        let expected = r#"{"session_id":"sess-0"}"#;
        assert_eq!(body(first).await, expected);
        assert_eq!(body(second).await, expected);
        assert_eq!(body(third).await, expected);
    }

    #[tokio::test]
    async fn another_scope_creates_its_own_session() {
        let replays = Replays::default();
        let calls = AtomicUsize::new(0);
        for scope in [
            "user\n/api/sessions\na",
            "user\n/api/sessions\nb",
            "other\n/api/sessions\na",
        ] {
            replays.create_once(scope.into(), created(&calls)).await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_failed_creation_can_be_retried_with_the_same_key() {
        let replays = Replays::default();
        let calls = AtomicUsize::new(0);
        let refused = replays
            .create_once("user\n/api/sessions\nkey".into(), async {
                (StatusCode::CONFLICT, "at capacity").into_response()
            })
            .await;
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        assert_eq!(body(refused).await, "at capacity");
        let retried = replays
            .create_once("user\n/api/sessions\nkey".into(), created(&calls))
            .await;
        assert_eq!(body(retried).await, r#"{"session_id":"sess-0"}"#);
    }

    #[test]
    fn expired_claims_are_released() {
        let replays = Replays::default();
        let start = Instant::now();
        let first = replays.claim("scope".into(), start);
        let kept = replays.claim("scope".into(), start + RETENTION - Duration::from_secs(1));
        assert!(Arc::ptr_eq(&first, &kept));
        let fresh = replays.claim("scope".into(), start + RETENTION);
        assert!(!Arc::ptr_eq(&first, &fresh));
    }
}

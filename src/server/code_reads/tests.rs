use std::cell::Cell;

use axum::body::to_bytes;
use axum::http::HeaderMap;
use tokio::sync::oneshot;

use super::*;
use crate::core::{Hub, SessionOrigin, Status};

fn create(hub: &Hub) {
    hub.create_local_session(
        "session".into(),
        "codex".into(),
        "/work/a".into(),
        "title".into(),
        SessionOrigin::default(),
        false,
    );
}

async fn assert_stale(response: Response) {
    assert_eq!(response.status(), StatusCode::GONE);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(!response.headers().contains_key(header::ETAG));
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "code context changed"
    );
}

#[tokio::test]
async fn stale_observation_never_invokes_the_read_closure() {
    let hub = Hub::new();
    create(&hub);
    let scope = hub.session_code_scope("session").unwrap();
    hub.update_session_cwd("session", "/work/b".into()).unwrap();
    let invoked = Cell::new(false);
    let checks = Cell::new(0);
    let response = guarded_response(
        || async {
            checks.set(checks.get() + 1);
            hub.code_scope_is_current(&scope)
        },
        || {
            // Even synchronous setup, before constructing the future, is gated.
            invoked.set(true);
            async { "must not run".into_response() }
        },
    )
    .await;
    assert!(!invoked.get());
    assert_eq!(checks.get(), 1);
    assert_stale(response).await;
}

#[derive(Clone, Copy)]
enum Change {
    Retarget,
    CwdAba,
    Delete,
    Recreate,
}

impl Change {
    fn apply(self, hub: &Hub) {
        match self {
            Self::Retarget => {
                hub.update_session_cwd("session", "/work/b".into()).unwrap();
            }
            Self::CwdAba => {
                hub.update_session_cwd("session", "/work/b".into()).unwrap();
                hub.update_session_cwd("session", "/work/a".into()).unwrap();
            }
            Self::Delete => assert!(hub.delete_session("session")),
            Self::Recreate => {
                assert!(hub.delete_session("session"));
                create(hub);
            }
        }
    }
}

#[tokio::test]
async fn changes_while_reading_discard_success_conditional_and_error_responses() {
    for change in [
        Change::Retarget,
        Change::CwdAba,
        Change::Delete,
        Change::Recreate,
    ] {
        for status in [
            StatusCode::OK,
            StatusCode::NOT_MODIFIED,
            StatusCode::BAD_GATEWAY,
        ] {
            let hub = Hub::new();
            create(&hub);
            let scope = hub.session_code_scope("session").unwrap();
            let (started, waiting) = oneshot::channel();
            let (release, released) = oneshot::channel();
            let checks = Cell::new(0);
            let read = guarded_response(
                || async {
                    checks.set(checks.get() + 1);
                    hub.code_scope_is_current(&scope)
                },
                || async {
                    started.send(()).unwrap();
                    released.await.unwrap();
                    (
                        status,
                        [
                            (header::ETAG, "\"old-snapshot\""),
                            (header::CACHE_CONTROL, "private"),
                        ],
                        vec![0x89, b'P', b'N', b'G', 0, 1, 2],
                    )
                        .into_response()
                },
            );
            let retarget = async {
                waiting.await.unwrap();
                change.apply(&hub);
                release.send(()).unwrap();
            };
            let (response, ()) = tokio::join!(read, retarget);
            assert_eq!(checks.get(), 2);
            assert_stale(response).await;
        }
    }
}

#[tokio::test]
async fn stable_cache_hits_keep_exact_headers_bytes_and_conditional_status() {
    for conditional in [false, true] {
        let hub = Hub::new();
        create(&hub);
        let scope = hub.session_code_scope("session").unwrap();
        let mut headers = HeaderMap::new();
        if conditional {
            headers.insert(header::IF_NONE_MATCH, "\"revision\"".parse().unwrap());
        }
        let response = guarded_response(
            || async { hub.code_scope_is_current(&scope) },
            || async {
                hub.rename_session("session", "renamed".into());
                hub.set_status("session", Status::Running, None);
                hub.update_session_cwd("session", "/work/a".into()).unwrap();
                super::super::file_tree_http_response(&headers, "revision", b"cached tree".to_vec())
            },
        )
        .await;
        assert_eq!(
            response.status(),
            if conditional {
                StatusCode::NOT_MODIFIED
            } else {
                StatusCode::OK
            }
        );
        assert_eq!(response.headers()[header::ETAG], "\"revision\"");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, max-age=15, stale-while-revalidate=120"
        );
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            body.as_ref(),
            if conditional {
                b"".as_slice()
            } else {
                b"cached tree".as_slice()
            }
        );
    }
}

#[tokio::test]
async fn an_immediate_conditional_cache_return_is_still_rechecked() {
    let hub = Hub::new();
    create(&hub);
    let scope = hub.session_code_scope("session").unwrap();
    let response = guarded_response(
        || async { hub.code_scope_is_current(&scope) },
        || async {
            Change::Recreate.apply(&hub);
            let mut headers = HeaderMap::new();
            headers.insert(header::IF_NONE_MATCH, "\"revision\"".parse().unwrap());
            super::super::file_tree_http_response(&headers, "revision", b"cached tree".to_vec())
        },
    )
    .await;
    assert_stale(response).await;
}

#[tokio::test]
async fn stable_context_preserves_read_errors_instead_of_masking_them() {
    let hub = Hub::new();
    create(&hub);
    let scope = hub.session_code_scope("session").unwrap();
    let response = guarded_response(
        || async { hub.code_scope_is_current(&scope) },
        || async { (StatusCode::BAD_GATEWAY, "remote file unavailable").into_response() },
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "remote file unavailable"
    );
}

#[tokio::test]
async fn asynchronous_recheck_finishes_before_releasing_the_response() {
    let checks = Cell::new(0);
    let read_finished = Cell::new(false);
    let response = guarded_response(
        || async {
            tokio::task::yield_now().await;
            checks.set(checks.get() + 1);
            !read_finished.get()
        },
        || async {
            read_finished.set(true);
            "unreleased".into_response()
        },
    )
    .await;
    assert_eq!(checks.get(), 2);
    assert_stale(response).await;
}

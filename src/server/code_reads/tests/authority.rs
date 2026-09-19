use super::*;
use crate::admin::AdminRole;
use crate::server::product_continuation::tests::Harness;

#[tokio::test]
async fn revoked_reads_discard_success_conditional_and_error_bodies() {
    for kind in ["cookie", "token", "disabled", "visibility"] {
        for status in [
            StatusCode::OK,
            StatusCode::NOT_MODIFIED,
            StatusCode::BAD_GATEWAY,
        ] {
            let h = Harness::new().await;
            if kind == "visibility" {
                h.role(AdminRole::Owner);
            }
            let headers = if kind == "token" {
                h.token().await
            } else {
                h.cookie().await
            };
            let product = h.capture(&headers).await;
            let owner = if kind == "visibility" {
                "other-user"
            } else {
                &h.user.id
            };
            let (started, waiting) = oneshot::channel();
            let (release, released) = oneshot::channel();
            let read = guarded_response(
                || authorized(h.auth(), &product, Some(owner)),
                || async {
                    started.send(()).unwrap();
                    released.await.unwrap();
                    (
                        status,
                        [
                            (header::ETAG, "\"private-revision\""),
                            (header::CACHE_CONTROL, "private"),
                        ],
                        "private bytes",
                    )
                        .into_response()
                },
            );
            let change = async {
                waiting.await.unwrap();
                h.revoke(kind).await;
                release.send(()).unwrap();
            };
            let (response, ()) = tokio::join!(read, change);
            assert_eq!(
                response.status(),
                if kind == "visibility" {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(!response.headers().contains_key(header::ETAG));
            assert_ne!(
                to_bytes(response.into_body(), 1024).await.unwrap(),
                "private bytes"
            );
        }
    }
}

#[tokio::test]
async fn revocation_before_read_never_invokes_even_synchronous_setup() {
    let h = Harness::new().await;
    let product = h.capture(&h.cookie().await).await;
    h.revoke("cookie").await;
    let invoked = Cell::new(false);
    let response = guarded_response(
        || authorized(h.auth(), &product, Some(&h.user.id)),
        || {
            invoked.set(true);
            async { "private".into_response() }
        },
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(!invoked.get());
}

#[tokio::test]
async fn viewer_own_and_shared_reads_keep_success_and_conditional_responses() {
    let h = Harness::new().await;
    let product = h.capture(&h.cookie().await).await;
    for owner in [None, Some(h.user.id.as_str())] {
        for status in [StatusCode::OK, StatusCode::NOT_MODIFIED] {
            let response = guarded_response(
                || authorized(h.auth(), &product, owner),
                || async { (status, [(header::ETAG, "\"current\"")], "current").into_response() },
            )
            .await;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::ETAG], "\"current\"");
        }
    }
}

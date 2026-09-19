use super::*;
use crate::admin::AdminRole;
use crate::server::product_continuation::tests::Harness;
use std::time::Duration;

fn fixture(h: &Harness, open: bool) -> (Fixture, String) {
    let mut f = Fixture::new();
    f.context.hub = h.hub.clone();
    f.context.product_auth_enabled = true;
    f.context.store = Some(h.store.clone());
    create_owned(&f.context.hub, "machine", &h.user.id);
    let owners = &f.context.code_buffers;
    let id = owners
        .insert(
            owners.reserve().unwrap(),
            Binding {
                user: h.user.id.clone(),
                scope: f.context.hub.session_code_scope("session").unwrap(),
                connection: f.connection.clone(),
                native: serde_json::from_value(native(1)).unwrap(),
            },
        )
        .unwrap()
        .resource_id;
    if open {
        let Admission::Run(job) = owners.admit(&h.user.id, &id, Action::Open).unwrap() else {
            panic!("fixture open required")
        };
        job.begin().unwrap();
        job.finish(remote::LeaseState::Open).unwrap();
    }
    (f, id)
}

async fn verified(h: &Harness, headers: &HeaderMap) -> AuthenticatedProductRequest {
    resolve_product_api_request_principal(
        h.auth(),
        &Method::GET,
        &"/api/code/buffers/fixture".parse().unwrap(),
        headers,
    )
    .await
    .unwrap()
    .unwrap()
}

async fn denied(response: Response) {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(!response.headers().contains_key(header::ETAG));
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap(),
        "owned buffer unavailable",
    );
}

fn saved(f: &Fixture, user: &str, id: &str, action: Action, state: remote::LeaseState) {
    let Admission::Saved(snapshot) = f.context.code_buffers.admit(user, id, action).unwrap() else {
        panic!("must retain a saved outcome without dispatch")
    };
    assert_eq!(snapshot.resource_id, id);
    assert_eq!(snapshot.state, state);
    assert!(!snapshot.pending);
}

#[tokio::test]
async fn an_expired_original_deadline_never_begins_or_consumes_an_open_attempt() {
    let mut f = Fixture::new();
    let prepared = f.prepare(1).await;
    let authority = Arc::new(approval(&f.context, &authenticated(), &HeaderMap::new()).unwrap());
    let Admission::Run(job) = f
        .context
        .code_buffers
        .admit("local", &prepared.resource_id, Action::Open)
        .unwrap()
    else {
        panic!("expected original admission")
    };
    assert_eq!(
        run_job(
            f.context.clone(),
            authority,
            *job,
            tokio::time::Instant::now()
        )
        .await
        .unwrap_err(),
        StatusCode::GATEWAY_TIMEOUT,
    );
    assert!(f.commands.try_recv().is_err());
    // No effect began. The inert original resource still permits a separately
    // authenticated Open, rather than fabricating Unknown or Released.
    assert!(matches!(
        f.context
            .code_buffers
            .admit("local", &prepared.resource_id, Action::Open)
            .unwrap(),
        Admission::Run(_),
    ));
}

#[tokio::test]
async fn original_authority_is_rechecked_after_effects_without_discarding_the_outcome() {
    for action in [Action::Open, Action::Query, Action::Release] {
        for change in ["cookie", "token", "disabled", "visibility"] {
            let h = Harness::new().await;
            h.role(AdminRole::Operator);
            let (mut f, id) = fixture(&h, action != Action::Open);
            let headers = if change == "token" {
                h.token().await
            } else {
                h.cookie().await
            };
            let authenticated = verified(&h, &headers).await;
            let task = tokio::spawn(operate(
                f.context.clone(),
                id.clone(),
                authenticated,
                headers,
                action,
            ));
            let command = tokio::time::timeout(Duration::from_secs(2), f.commands.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                payload(&command)["type"],
                match action {
                    Action::Open => "openBufferLease",
                    Action::Query => "queryBufferLease",
                    Action::Release => "releaseBufferLease",
                }
            );
            h.revoke(change).await;
            let state = if action == Action::Release {
                "released"
            } else {
                "open"
            };
            reply(&f.context, &f.connection, command, native_reply(1, state));
            denied(task.await.unwrap()).await;
            saved(
                &f,
                &h.user.id,
                &id,
                if action == Action::Release {
                    Action::Query
                } else {
                    Action::Open
                },
                if action == Action::Release {
                    remote::LeaseState::Released
                } else {
                    remote::LeaseState::Open
                },
            );
            assert!(
                f.commands.try_recv().is_err(),
                "refusal cannot replay or compensate"
            );
        }
    }
}

#[tokio::test]
async fn revoked_authority_masks_failed_dispatch_without_rearming_unknown_effects() {
    for action in [Action::Open, Action::Query, Action::Release] {
        let h = Harness::new().await;
        h.role(AdminRole::Operator);
        let (mut f, id) = fixture(&h, action != Action::Open);
        let headers = h.cookie().await;
        let task = tokio::spawn(operate(
            f.context.clone(),
            id.clone(),
            verified(&h, &headers).await,
            headers,
            action,
        ));
        let command = tokio::time::timeout(Duration::from_secs(2), f.commands.recv())
            .await
            .unwrap()
            .unwrap();
        h.revoke("cookie").await;
        reply(&f.context, &f.connection, command, Value::Null);
        denied(task.await.unwrap()).await;
        saved(
            &f,
            &h.user.id,
            &id,
            if action == Action::Release {
                Action::Release
            } else {
                Action::Open
            },
            if action == Action::Query {
                remote::LeaseState::Open
            } else {
                remote::LeaseState::Unknown
            },
        );
        assert!(f.commands.try_recv().is_err());
    }
}

#[tokio::test]
async fn saved_open_and_terminal_receipts_require_current_original_authority() {
    for action in [Action::Open, Action::Query, Action::Release] {
        for change in ["cookie", "token", "disabled", "visibility"] {
            let h = Harness::new().await;
            h.role(AdminRole::Operator);
            let (mut f, id) = fixture(&h, true);
            if action != Action::Open {
                let Admission::Run(job) = f
                    .context
                    .code_buffers
                    .admit(&h.user.id, &id, Action::Release)
                    .unwrap()
                else {
                    panic!("fixture release required")
                };
                job.begin().unwrap();
                job.finish(remote::LeaseState::Released).unwrap();
            }
            let headers = if change == "token" {
                h.token().await
            } else {
                h.cookie().await
            };
            let authenticated = verified(&h, &headers).await;
            h.revoke(change).await;
            denied(operate(f.context.clone(), id, authenticated, headers, action).await).await;
            assert!(f.commands.try_recv().is_err());
        }
    }
}

use super::*;
use crate::admin::{AdminRole, hex_sha256};
use crate::store::{ProductApiToken, ProductUser};

#[tokio::test]
async fn expired_read_deadline_does_not_probe_and_releases_only_its_borrow() {
    let (mut f, id) = opened().await;
    let authority = Arc::new(approval(&f.context, &authenticated(), &HeaderMap::new()).unwrap());
    let job = f.context.code_buffers.admit_read("local", &id).unwrap();
    assert!(matches!(
        run_read(
            f.context.clone(),
            authority,
            job,
            Request::Language {},
            tokio::time::Instant::now()
        )
        .await,
        Err(StatusCode::GATEWAY_TIMEOUT)
    ));
    assert!(f.commands.try_recv().is_err());
    assert!(f.context.code_buffers.admit_read("local", &id).is_ok());
}

#[tokio::test]
async fn cancelled_read_drains_by_the_original_deadline_across_both_native_waits() {
    let (mut f, id) = opened().await;
    tokio::time::pause();
    let task = start(&f, &id, Request::Language {});
    let support = command(&mut f).await;
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    reply(
        &f.context,
        &f.connection,
        support,
        json!({"type":"bufferLeaseReadSupport","api_version":1}),
    );
    let reading = command(&mut f).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(f.context.code_buffers.admit_read("local", &id).is_err());
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    assert!(
        f.context.code_buffers.admit_read("local", &id).is_ok(),
        "a dropped observer must not renew the owned read's total budget"
    );
    reply(&f.context, &f.connection, reading, result("language"));
    let Admission::Saved(snapshot) = f
        .context
        .code_buffers
        .admit("local", &id, Action::Open)
        .unwrap()
    else {
        panic!("read timeout must not reopen or close the original native owner")
    };
    assert_eq!(snapshot.state, remote::LeaseState::Open);
    assert!(f.commands.try_recv().is_err());
}

#[tokio::test]
async fn original_credential_and_role_are_rechecked_after_each_remote_boundary() {
    for stage in ["probe", "read", "probe_error", "read_error"] {
        for change in ["revoked", "role", "role_aba"] {
            let mut fixture = Fixture::new();
            let root = tempfile::tempdir().unwrap();
            let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
                .await
                .unwrap();
            store.migrate().await.unwrap();
            let user = ProductUser {
                id: "c".repeat(32),
                username: "fixture".into(),
                password_algo: "argon2id".into(),
                password_hash: "unused-hermetic-fixture".into(),
                created_at_ms: auth_now_ms(),
                updated_at_ms: auth_now_ms(),
                disabled_at_ms: None,
            };
            store.insert_user(&user).await.unwrap();
            fixture.context.hub.delete_session("session");
            crate::server::code_buffers::tests::create_owned(
                &fixture.context.hub,
                "machine",
                &user.id,
            );
            let owners = &fixture.context.code_buffers;
            let id = owners
                .insert(
                    owners.reserve().unwrap(),
                    Binding {
                        user: user.id.clone(),
                        scope: fixture.context.hub.session_code_scope("session").unwrap(),
                        connection: fixture.connection.clone(),
                        native: serde_json::from_value(native(1)).unwrap(),
                    },
                )
                .unwrap()
                .resource_id;
            let Admission::Run(open) = owners.admit(&user.id, &id, Action::Open).unwrap() else {
                panic!("expected fixture open admission");
            };
            open.begin().unwrap();
            open.finish(remote::LeaseState::Open).unwrap();
            let token = ProductApiToken {
                id: "a".repeat(32),
                user_id: user.id.clone(),
                name: "fixture".into(),
                token_prefix: "cow_test".into(),
                token_hash: hex_sha256(b"cow_test-buffer-read"),
                created_at_ms: auth_now_ms(),
                expires_at_ms: Some(auth_now_ms() + 300_000),
                last_used_at_ms: None,
                revoked_at_ms: None,
            };
            store.insert_user_api_token(&token).await.unwrap();
            fixture.context.product_auth_enabled = true;
            fixture.context.store = Some(store.clone());
            fixture.context.hub.set_setting(
                crate::admin::PERMISSIONS_SETTING.into(),
                json!({"default_role":AdminRole::Operator,"grants":[]}),
            );
            let mut headers = HeaderMap::new();
            headers.insert(
                header::AUTHORIZATION,
                "Bearer cow_test-buffer-read".parse().unwrap(),
            );
            let authenticated = resolve_product_api_request_principal(
                fixture.context.auth(),
                &Method::POST,
                &format!("/api/code/buffers/{id}/read").parse().unwrap(),
                &headers,
            )
            .await
            .unwrap()
            .unwrap();
            let task = tokio::spawn(read(
                State(fixture.context.clone()),
                Path(id),
                Extension(authenticated),
                headers,
                Json(Request::Language {}),
            ));
            if stage.starts_with("read") {
                probe(&mut fixture).await;
            }
            let command = command(&mut fixture).await;
            if change == "revoked" {
                store
                    .revoke_user_api_token_for_user(&user.id, &token.id)
                    .await
                    .unwrap();
            } else {
                fixture.context.hub.set_setting(
                    crate::admin::PERMISSIONS_SETTING.into(),
                    json!({"default_role":AdminRole::Viewer,"grants":[]}),
                );
                if change == "role_aba" {
                    fixture.context.hub.set_setting(
                        crate::admin::PERMISSIONS_SETTING.into(),
                        json!({"default_role":AdminRole::Operator,"grants":[]}),
                    );
                }
            }
            let value = if stage.ends_with("_error") {
                Value::Null
            } else if stage == "read" {
                result("language")
            } else {
                json!({"type":"bufferLeaseReadSupport","api_version":1})
            };
            reply(&fixture.context, &fixture.connection, command, value);
            assert_eq!(task.await.unwrap().status(), StatusCode::UNAUTHORIZED);
            assert!(fixture.commands.try_recv().is_err());
        }
    }
}

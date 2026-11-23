use super::*;
use crate::admin::AdminRole;

pub use super::test_fixtures::Harness;

#[tokio::test]
async fn permission_aba_cannot_revive_an_original_product_continuation() {
    let h = Harness::new().await;
    h.role(AdminRole::Owner);
    let headers = h.cookie().await;
    let captured = h.capture(&headers).await;
    h.role(AdminRole::Viewer);
    h.role(AdminRole::Owner);
    // No intermediate poll: the core's actual permission mutation must end
    // the original lifetime, not just compare equal before/after role values.
    assert!(captured.current(h.auth()).await.is_none());
    assert!(h.capture(&headers).await.current(h.auth()).await.is_some());
}

#[tokio::test]
async fn queued_authentication_cannot_capture_a_replacement_permission_lifetime() {
    for token in [false, true] {
        let h = Harness::new().await;
        h.role(AdminRole::Operator);
        let headers = if token {
            h.token().await
        } else {
            h.cookie().await
        };
        let verified = resolve_product_api_request_principal(
            h.auth(),
            &Method::GET,
            &"/api/plugins".parse().unwrap(),
            &headers,
        )
        .await
        .unwrap()
        .unwrap();
        h.revoke("permission_aba").await;
        assert!(ProductContinuation::capture(h.auth(), Some(&verified), &headers).is_err());
        assert!(h.capture(&headers).await.current(h.auth()).await.is_some());
    }
}

#[tokio::test]
async fn permission_observations_cannot_be_substituted_from_another_core() {
    let h = Harness::new().await;
    let headers = h.cookie().await;
    let captured = h.capture(&headers).await;
    let other = Hub::new();
    assert!(
        captured
            .current(ProductRequestAuth {
                hub: &other,
                ..h.auth()
            })
            .await
            .is_none()
    );
    assert!(captured.current(h.auth()).await.is_some());
}

#[tokio::test]
async fn original_cookie_and_token_never_fall_back_or_change_user() {
    for kind in ["cookie", "token", "disabled"] {
        let h = Harness::new().await;
        let mut headers = h.cookie().await;
        if kind == "token" {
            headers.extend(h.token().await);
        }
        let captured = h.capture(&headers).await;
        assert_eq!(captured.current(h.auth()).await.unwrap().user_id, h.user.id);
        h.revoke(kind).await;
        assert!(captured.current(h.auth()).await.is_none(), "{kind}");
        assert!(
            captured
                .current(ProductRequestAuth {
                    product_auth_enabled: false,
                    ..h.auth()
                })
                .await
                .is_none()
        );
        assert!(ProductContinuation::capture(h.auth(), None, &headers).is_err());
    }
}

#[tokio::test]
async fn current_roles_preserve_viewer_reads_without_granting_mutations() {
    let h = Harness::new().await;
    let headers = h.cookie().await;
    let captured = h.capture(&headers).await;
    let viewer = captured.current(h.auth()).await.unwrap();
    assert!(viewer.can_see(Some(&h.user.id)) && viewer.can_see(None));
    assert!(!viewer.can_see(Some("other-user")) && !viewer.can_mutate(Some(&h.user.id)));
    h.role(AdminRole::Owner);
    assert!(captured.current(h.auth()).await.is_none());
    let owner = h.capture(&headers).await;
    assert!(
        owner
            .current(h.auth())
            .await
            .unwrap()
            .can_see(Some("other-user"))
    );
    h.role(AdminRole::Viewer);
    assert!(owner.current(h.auth()).await.is_none());
    assert!(captured.current(h.auth()).await.is_none());
    let fresh = h.capture(&headers).await;
    assert!(
        !fresh
            .current(h.auth())
            .await
            .unwrap()
            .can_see(Some("other-user"))
    );
}

#[tokio::test]
async fn device_and_automation_reads_do_not_replay_the_original_proof() {
    for automation in [false, true] {
        let mut h = Harness::new().await;
        h.authentication.automation.enabled = automation;
        if automation {
            h.role(AdminRole::Operator);
        }
        let key = crate::client_auth::new_signing_key().unwrap();
        let public = crate::client_auth::public_key_to_base64(&key);
        let device = "d".repeat(32);
        if !automation {
            crate::server::persist_fixture_device(&h.store, &h.user.id, &device, &public).await;
        }
        let (token, _) = if automation {
            h.devices.issue_automation(
                &device,
                &h.user.id,
                &public,
                vec!["api:read".into()],
                auth_now_ms(),
                60_000,
            )
        } else {
            h.devices.issue(&device, &h.user.id, &public, auth_now_ms())
        }
        .unwrap();
        let path = "/api/code/sessions/fixture/file?path=read.txt";
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        for (name, value) in crate::client_auth::signed_proof_headers(
            &key,
            &device,
            &token,
            "GET",
            path,
            auth_now_ms(),
        )
        .unwrap()
        {
            headers.insert(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        let verified = resolve_product_api_request_principal(
            h.auth(),
            &Method::GET,
            &path.parse().unwrap(),
            &headers,
        )
        .await
        .unwrap()
        .unwrap();
        let captured = ProductContinuation::capture(h.auth(), Some(&verified), &headers).unwrap();
        if automation {
            assert!(
                crate::server::operator_approval::OperatorApproval::capture_product(
                    h.auth(),
                    "service",
                    Some(&verified),
                    &headers
                )
                .is_err()
            );
        }
        for _ in 0..2 {
            assert!(captured.current(h.auth()).await.is_some());
        }
        assert!(
            h.devices
                .authenticate(&headers, &Method::GET, path, auth_now_ms())
                .is_err()
        );
        if automation {
            h.authentication.automation.enabled = false;
            assert!(captured.current(h.auth()).await.is_none());
            h.authentication.automation.enabled = true;
        } else {
            // A different process can revoke durable authority without clearing
            // this process's still-live token map.
            h.store
                .revoke_user_device(&device, auth_now_ms())
                .await
                .unwrap();
            assert!(h.devices.token_still_valid(
                &token,
                &verified.device_identity.clone().unwrap(),
                auth_now_ms()
            ));
            assert!(captured.current(h.auth()).await.is_none());
        }
        h.devices.revoke_access_token(&token);
        assert!(captured.current(h.auth()).await.is_none());
    }
}

#[tokio::test]
async fn only_verified_explicit_local_mode_can_capture_local_continuation() {
    let h = Harness::new().await;
    let local = ProductRequestAuth {
        product_auth_enabled: false,
        ..h.auth()
    };
    let verified = resolve_product_api_request_principal(
        local,
        &Method::GET,
        &"/api/code/sessions/s/file".parse().unwrap(),
        &HeaderMap::new(),
    )
    .await
    .unwrap()
    .unwrap();
    let captured = ProductContinuation::capture(local, Some(&verified), &HeaderMap::new()).unwrap();
    assert_eq!(
        captured.current(local).await,
        Some(crate::product_auth::local_product_principal())
    );
    assert!(captured.current(h.auth()).await.is_none());
    assert!(ProductContinuation::capture(local, None, &HeaderMap::new()).is_err());
}

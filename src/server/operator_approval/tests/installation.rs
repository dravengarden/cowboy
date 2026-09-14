use super::*;

fn desired() -> crate::machine_protocol::DesiredPlugin {
    // This fixture tests confirmation binding, not package/signature admission.
    // It deliberately needs no Machine-host module in the Controller test slice.
    serde_json::from_value(serde_json::json!({
        "release": {
            "release_schema": 1, "plugin_id": "victoria", "plugin_version": "1.1.0",
            "plugin_kind": "telemetry_backend", "package_digest": "sha256:fixture-package",
            "artifact_digest": "sha256:fixture-release", "artifact_url": "https://example.invalid/plugin",
            "publisher": "fixture", "contract_fingerprint": "sha256:fixture-contract",
            "component_release": "2.9.0", "host_bundle_digest": null, "signature": "fixture",
            "supported_platforms": [], "runtime_artifacts": []
        },
        "package_base64": "e30=", "publisher_public_key": "fixture", "host_bundle_base64": null
    })).unwrap()
}

#[tokio::test]
async fn installation_binds_complete_release_target_original_credential_and_budget() {
    for change in [
        "none",
        "service",
        "machine",
        "version",
        "digest",
        "package",
        "host",
        "publisher",
        "signature",
        "url",
        "logout",
        "role",
        "disabled",
        "queued",
    ] {
        let h = Harness::new().await;
        let desired = desired();
        let (headers, verified) = h.cookie().await;
        let mut approval =
            OperatorApproval::capture(h.context(), "service-test", Some(&verified), &headers)
                .unwrap();
        if change == "queued" {
            approval.received = TimeSample::for_test(
                std::time::Instant::now() - Duration::from_secs(301),
                auth_now_ms(),
            );
        }
        let authority = approval
            .bind_installation("machine-test", &desired)
            .unwrap();
        let mut changed = desired.clone();
        match change {
            "version" => changed.release.plugin_version = "9.0.0".into(),
            "digest" => changed.release.artifact_digest.push('f'),
            "package" => changed.package_base64.push('a'),
            "host" => changed.host_bundle_base64 = Some("e30=".into()),
            "publisher" => changed.publisher_public_key.push('a'),
            "signature" => changed.release.signature.push('a'),
            "url" => changed.release.artifact_url.push_str("/other"),
            "logout" => {
                h.store
                    .revoke_user_session_for_user(
                        &h.user.id,
                        "session-approval",
                        "logout",
                        auth_now_ms(),
                    )
                    .await
                    .unwrap();
            }
            "role" => h.role(AdminRole::Viewer),
            "disabled" => {
                h.store
                    .set_user_disabled_at(&h.user.id, Some(auth_now_ms()))
                    .await
                    .unwrap();
            }
            _ => {}
        }
        assert_eq!(
            authority
                .check(
                    h.context(),
                    if change == "service" {
                        "foreign-service"
                    } else {
                        "service-test"
                    },
                    if change == "machine" {
                        "foreign-machine"
                    } else {
                        "machine-test"
                    },
                    &changed
                )
                .await,
            change == "none",
            "{change}"
        );
        h.role(AdminRole::Operator);
        h.store
            .set_user_disabled_at(&h.user.id, None)
            .await
            .unwrap();
        assert_eq!(
            authority
                .check(h.context(), "service-test", "machine-test", &desired)
                .await,
            change == "none",
            "repair cannot revive {change}"
        );
    }
}

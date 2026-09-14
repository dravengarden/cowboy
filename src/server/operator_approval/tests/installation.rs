use super::*;

#[tokio::test]
async fn installation_wire_deadline_keeps_time_spent_before_binding() {
    let h = Harness::new().await;
    let (headers, verified) = h.cookie().await;
    let mut approval =
        OperatorApproval::capture(h.context(), "service-test", Some(&verified), &headers).unwrap();
    let original_wall = auth_now_ms() - 240_000;
    approval.received = TimeSample::for_test(
        std::time::Instant::now() - Duration::from_secs(240),
        original_wall,
    );
    let authority = approval
        .bind_installation(
            "machine-test",
            &desired(),
            "original-deadline-fixture-0001".into(),
            crate::machine_protocol::plugin_install::InstallTarget::Vacant {},
        )
        .unwrap();
    assert_eq!(authority.intent().expires_at_ms, original_wall + 300_000);
    assert!(authority.within_budget());
    assert!(authority.intent().expires_at_ms <= auth_now_ms() + 60_000);
}

fn desired() -> crate::machine_protocol::DesiredPlugin {
    // This fixture tests confirmation binding, not package/signature admission.
    // It deliberately needs no Machine-host module in the Controller test slice.
    serde_json::from_value(serde_json::json!({
        "release": {
            "release_schema": 1, "plugin_id": "victoria", "plugin_version": "1.1.0",
            "plugin_kind": "telemetry_backend", "package_digest": "sha256:fixture-package",
            "artifact_digest": format!("sha256:{}", "a".repeat(64)), "artifact_url": "https://example.invalid/plugin",
            "publisher": "fixture", "contract_fingerprint": format!("sha256:{}", "b".repeat(64)),
            "component_release": "2.9.0", "host_bundle_digest": null, "signature": "fixture",
            "supported_platforms": [], "runtime_artifacts": []
        },
        "package_base64": "e30=", "publisher_public_key": "fixture", "host_bundle_base64": null
    })).unwrap()
}

#[tokio::test]
async fn installation_dispatch_cannot_substitute_a_step_or_revive_a_revoked_binding() {
    use crate::machine_protocol::plugin_install::InstallTarget;
    for change in ["target", "plan", "deadline", "operation"] {
        let h = Harness::new().await;
        let (headers, verified) = h.cookie().await;
        let authority =
            OperatorApproval::capture(h.context(), "service-test", Some(&verified), &headers)
                .unwrap()
                .bind_installation(
                    "machine-test",
                    &desired(),
                    "dispatch-install-fixture-0001".into(),
                    InstallTarget::Vacant {},
                )
                .unwrap();
        let original = authority.intent().machine_step().unwrap();
        assert!(authority.matches_live_step(&original));
        let mut changed = original.clone();
        match change {
            "target" => {
                changed.expected = InstallTarget::Removed {
                    revision: format!("installation-{}", "e".repeat(64))
                        .try_into()
                        .unwrap(),
                }
            }
            "plan" => changed.plan_digest = format!("sha256:{}", "e".repeat(64)),
            "deadline" => changed.expires_at_ms += 1,
            _ => changed.operation_id.push('x'),
        }
        assert!(!authority.matches_live_step(&changed), "{change}");
        assert!(
            !authority.matches_live_step(&original),
            "cannot revive {change}"
        );
    }
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
            .bind_installation(
                "machine-test",
                &desired,
                "installation-authority-fixture".into(),
                crate::machine_protocol::plugin_install::InstallTarget::Vacant {},
            )
            .unwrap();
        assert_eq!(
            authority.intent().request_id,
            "plugin-install-installation-authority-fixture"
        );
        assert_eq!(
            authority.intent().envelope_digest,
            format!(
                "sha256:{}",
                hex_sha256(&serde_json::to_vec(&desired).unwrap())
            )
        );
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

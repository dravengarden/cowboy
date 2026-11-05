
use super::{MachineControl, ProviderAuthExecutor, reconcile_provider_auth_executors};
use std::collections::HashMap;

#[test]
fn machine_auth_discovery_preserves_legacy_catalog_schema() {
    let identity = |machine_id| super::ProviderAuthenticationExecutorIdentity {
        machine_id,
        provider_id: "claude-code".into(),
        provider_version: "1.0.0".into(),
        generation_digest: "sha256:fixture".into(),
    };
    let legacy = serde_json::to_value(identity(None)).unwrap();
    assert!(legacy.get("machine_id").is_none());
    let scoped = std::collections::BTreeSet::from([
        identity(Some("machine-a".into())),
        identity(Some("machine-b".into())),
    ]);
    assert_eq!(scoped.len(), 2);
    assert!(!super::machine_auth_discovery_requested(
        &super::HeaderMap::new()
    ));
    for (value, expected) in [("1", true), ("0", false), ("2", false)] {
        let mut headers = super::HeaderMap::new();
        headers.insert("x-cowboy-auth-machine", value.parse().unwrap());
        assert_eq!(super::machine_auth_discovery_requested(&headers), expected);
    }
}

#[test]
fn legacy_auth_requests_reject_unsupported_scope_instead_of_using_default() {
    use serde_json::json;
    let start = json!({"method":"native", "provider_version":"1", "generation_digest":"sha256:x"});
    let commit = json!({"method":"api-key", "values":{}});
    let submit = json!({"code":"fixture"});
    assert!(serde_json::from_value::<super::ProviderAuthStartRequest>(start.clone()).is_ok());
    assert!(serde_json::from_value::<super::ProviderAuthCommitRequest>(commit.clone()).is_ok());
    assert!(serde_json::from_value::<super::ProviderAuthSubmitRequest>(submit.clone()).is_ok());
    for field in ["account_id", "subscription_id"] {
        let scoped = |mut value: serde_json::Value| {
            value[field] = json!("explicit-scope");
            value
        };
        assert!(
            serde_json::from_value::<super::ProviderAuthStartRequest>(scoped(start.clone()))
                .is_err()
        );
        assert!(
            serde_json::from_value::<super::ProviderAuthCommitRequest>(scoped(commit.clone()))
                .is_err()
        );
        assert!(
            serde_json::from_value::<super::ProviderAuthSubmitRequest>(scoped(submit.clone()))
                .is_err()
        );
    }
}

#[test]
fn login_resume_requires_the_same_method_and_signed_release() {
    let control = control();
    let active = executor(&control, "claude-code", "claude-account", 2_000);
    for changed in [
        None,
        Some("method"),
        Some("version"),
        Some("digest"),
        Some("machine"),
        Some("owner"),
    ] {
        let request = super::ProviderAuthStartRequest {
            machine_id: Some(
                if changed == Some("machine") {
                    "other-machine"
                } else {
                    &active.machine_id
                }
                .into(),
            ),
            method: if changed == Some("method") {
                "api-key"
            } else {
                &active.auth_method
            }
            .into(),
            provider_version: if changed == Some("version") {
                "2.0.0"
            } else {
                &active.provider_version
            }
            .into(),
            generation_digest: if changed == Some("digest") {
                "sha256:other"
            } else {
                &active.generation_digest
            }
            .into(),
        };
        let response = super::resumed_provider_authentication_response(
            "request".into(),
            &active,
            &request,
            if changed == Some("owner") {
                "other-user"
            } else {
                "owner"
            },
        );
        assert_eq!(
            response.status(),
            if changed.is_none() {
                axum::http::StatusCode::ACCEPTED
            } else {
                axum::http::StatusCode::CONFLICT
            }
        );
    }
}

#[test]
fn login_management_requires_both_owner_and_provider() {
    let control = control();
    let active = executor(&control, "claude-code", "claude-account", 2_000);
    assert!(active.owned_by("owner", "claude-code"));
    assert!(!active.owned_by("other-user", "claude-code"));
    assert!(!active.owned_by("owner", "codex"));
    assert!(!active.owned_by("", "claude-code"));
}

#[test]
fn rejected_and_cancelled_login_submissions_never_reach_the_machine() {
    use crate::machine_protocol::MachineCommand;
    use axum::http::StatusCode;
    let control = MachineControl::default();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    control.install(
        "machine".into(),
        "epoch".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    let active = executor(&control, "claude-code", "claude-account", i64::MAX);
    let executors = parking_lot::Mutex::new(HashMap::from([("request".into(), active)]));
    for code in [
        "secret\ncommand".into(),
        "secret\0".into(),
        "x".repeat(4097),
    ] {
        assert_eq!(
            super::submit_provider_auth_code(
                &executors,
                &control,
                "owner",
                "claude-code",
                "request".into(),
                code,
            )
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(commands.try_recv().is_err());
    }
    let submit = |owner: &str, provider: &str| {
        super::submit_provider_auth_code(
            &executors,
            &control,
            owner,
            provider,
            "request".into(),
            "private-code".into(),
        )
        .status()
    };
    assert_eq!(submit("other-user", "claude-code"), StatusCode::NOT_FOUND);
    assert_eq!(submit("owner", "codex"), StatusCode::NOT_FOUND);
    assert!(commands.try_recv().is_err());
    assert_eq!(submit("owner", "claude-code"), StatusCode::ACCEPTED);
    assert!(matches!(commands.try_recv().unwrap(),
            MachineCommand::SubmitLoginCode { request_id, code }
            if request_id == "request" && code == "private-code"));
    executors
        .lock()
        .get_mut("request")
        .unwrap()
        .promotion_started = true;
    assert_eq!(submit("owner", "claude-code"), StatusCode::NOT_FOUND);
    assert!(commands.try_recv().is_err());
    executors.lock().remove("request");
    assert_eq!(submit("owner", "claude-code"), StatusCode::NOT_FOUND);
    assert!(commands.try_recv().is_err());
    let mut expired = executor(&control, "claude-code", "claude-account", 0);
    executors.lock().insert("request".into(), expired.clone());
    assert_eq!(submit("owner", "claude-code"), StatusCode::NOT_FOUND);
    expired.expires_at_ms = i64::MAX;
    executors.lock().insert("request".into(), expired);
    let (replacement_tx, mut replacement_commands) = tokio::sync::mpsc::unbounded_channel();
    control.install(
        "machine".into(),
        "epoch".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        replacement_tx,
    );
    assert_eq!(submit("owner", "claude-code"), StatusCode::NOT_FOUND);
    assert!(commands.try_recv().is_err());
    assert!(replacement_commands.try_recv().is_err());
}

#[test]
fn shared_machine_diagnostics_exclude_native_login_details() {
    use crate::machine_protocol::MachineEvent;
    let challenge = MachineEvent::LoginChallenge {
        request_id: "provider-login-fixture".into(),
        provider: "claude-code".into(),
        verification_url: "https://example.invalid/login".into(),
        user_code: Some("private-code".into()),
        input_required: false,
        input_label: None,
        secret_input: false,
        expires_at_ms: 2_000,
    };
    assert!(!super::machine_diagnostic_event_visible(&challenge));
    for (request_id, visible) in [("provider-login-fixture", false), ("install-fixture", true)] {
        let event: MachineEvent = serde_json::from_value(serde_json::json!({
            "event":"command_result", "request_id":request_id,
            "accepted":false, "detail":"command output"
        }))
        .unwrap();
        assert_eq!(super::machine_diagnostic_event_visible(&event), visible);
    }
}

fn control() -> MachineControl {
    let control = MachineControl::default();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    control.install(
        "machine".into(),
        "fixture".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    control
}

fn executor(
    control: &MachineControl,
    provider_id: &str,
    method: &str,
    expires_at_ms: i64,
) -> ProviderAuthExecutor {
    ProviderAuthExecutor {
        native_login: None,
        named_login: None,
        cancel_requested: false,
        owner_user_id: "owner".into(),
        machine_id: "machine".to_owned(),
        account_id: Default::default(),
        connection: control.operation_connection("machine").unwrap(),
        provider_id: provider_id.to_owned(),
        provider_version: "1.0.0".to_owned(),
        generation_digest: "sha256:generation".to_owned(),
        auth_contract_fingerprint: "sha256:contract".to_owned(),
        auth_method: method.to_owned(),
        expected_generation: 0,
        promotion_started: false,
        expires_at_ms,
    }
}

#[test]
fn failed_credential_promotion_can_retry_but_success_commits_only_once() {
    let control = control();
    let mut active = executor(&control, "claude-code", "claude-account", 2_000);
    active.expected_generation = 7;
    let failed = active.promote_once::<(), _>(|generation| {
        assert_eq!(generation, 7);
        Err("rejected credential bundle")
    });
    assert_eq!(failed, Err("rejected credential bundle"));
    assert!(!active.promotion_started);
    let retry = active.promote_once::<_, &str>(|generation| {
        assert_eq!(generation, 7);
        Ok("committed")
    });
    assert_eq!(retry, Ok(Some("committed")));
    let duplicate = active.promote_once::<(), &str>(|_| panic!("duplicate credential commit"));
    assert_eq!(duplicate, Ok(None));
}

#[test]
fn page_reload_recovers_the_existing_provider_authentication() {
    let control = control();
    let mut executors = HashMap::from([
        (
            "claude-request".to_owned(),
            executor(&control, "claude-code", "claude-account", 2_000),
        ),
        (
            "codex-request".to_owned(),
            executor(&control, "codex", "chatgpt-account", 3_000),
        ),
    ]);

    let reconciliation =
        reconcile_provider_auth_executors(&mut executors, "claude-code", 1_000, &control);

    assert!(reconciliation.expired.is_empty());
    let (request_id, active) = reconciliation
        .active
        .as_ref()
        .expect("existing Claude authentication");
    assert_eq!(request_id, "claude-request");
    assert_eq!(active.auth_method, "claude-account");
    assert!(reconciliation.resumable(Some("claude-request")).is_some());
    assert!(reconciliation.resumable(None).is_none());
    assert!(reconciliation.resumable(Some("failed-request")).is_none());
    assert_eq!(executors.len(), 2);
}

#[test]
fn expired_authentication_is_removed_instead_of_resumed() {
    let control = control();
    let mut executors = HashMap::from([
        (
            "expired-claude".to_owned(),
            executor(&control, "claude-code", "claude-account", 999),
        ),
        (
            "current-codex".to_owned(),
            executor(&control, "codex", "chatgpt-account", 2_000),
        ),
    ]);

    let reconciliation =
        reconcile_provider_auth_executors(&mut executors, "claude-code", 1_000, &control);

    assert_eq!(
        reconciliation.expired,
        vec![("claude-code".to_owned(), "expired-claude".to_owned())]
    );
    assert!(reconciliation.active.is_none());
    assert_eq!(executors.len(), 1);
    assert!(executors.contains_key("current-codex"));
}

#[test]
fn connection_replacement_and_deadline_revoke_resume_and_candidate_admission() {
    let control = control();
    let old = executor(&control, "claude-code", "claude-account", 2_000);
    assert!(old.active(&control, 1_999));
    assert!(!old.active(&control, 2_000));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    // Even a reused epoch string is a distinct authenticated connection.
    control.install(
        "machine".into(),
        "fixture".into(),
        false,
        crate::machine_protocol::MACHINE_PROTOCOL_VERSION,
        tx,
    );
    assert!(!old.active(&control, 1_000));
    assert!(
        control
            .send_on_connection(
                &old.connection,
                crate::machine_protocol::MachineCommand::SubmitLoginCode {
                    request_id: "old".into(),
                    code: "fixture".into(),
                }
            )
            .is_err()
    );
    assert!(rx.try_recv().is_err());
    let mut executors = HashMap::from([("old".into(), old)]);
    let result = reconcile_provider_auth_executors(&mut executors, "claude-code", 1_000, &control);
    assert!(result.active.is_none());
    assert_eq!(result.expired, [("claude-code".into(), "old".into())]);
    assert!(executors.is_empty());
}

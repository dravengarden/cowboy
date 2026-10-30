use super::*;
use crate::machine_protocol::{MachineCommand, PROVIDER_ACCOUNT_PROTOCOL_VERSION};

#[tokio::test]
async fn native_usage_rejects_legacy_protocol_and_replaced_connections() {
    let control = MachineControl::default();
    let profile = serde_json::from_value(json!({
        "account_id":"11111111-1111-4111-8111-111111111111","provider_id":"claude-code",
        "request_id":"login","generation":2,"method":"native","provider_version":"1",
        "generation_digest":"digest","auth_contract_fingerprint":"contract",
        "identity":{"authority":"claude","subject":"otter"}
    }))
    .unwrap();
    for protocol in [
        PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        crate::machine_protocol::NATIVE_USAGE_PROTOCOL_VERSION,
    ] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = control.install("machine".into(), "epoch".into(), false, protocol, tx);
        let pending = control.begin_native_usage(&connection, &profile);
        if protocol == PROVIDER_ACCOUNT_PROTOCOL_VERSION {
            assert!(pending.is_err());
            assert!(rx.try_recv().is_err());
            continue;
        }
        let pending = pending.unwrap();
        let MachineCommand::CollectNativeUsage {
            request_id,
            profile: sent,
        } = rx.try_recv().unwrap()
        else {
            panic!("scoped query expected")
        };
        assert_eq!(*sent, profile);
        control.record_remote(
            &connection,
            crate::machine_protocol::MachineEvent::PluginHostResponse {
                request_id,
                accepted: true,
                started: true,
                payload: Some(json!({"status":"available"})),
                detail: None,
            },
        );
        let (tx, _replacement) = tokio::sync::mpsc::unbounded_channel();
        control.install("machine".into(), "new".into(), false, protocol, tx);
        assert!(pending.await.is_err());
    }
}

#[tokio::test]
async fn native_usage_reports_closed_reasons_without_forwarding_provider_output() {
    let control = MachineControl::default();
    let profile = serde_json::from_value(json!({
        "account_id":"11111111-1111-4111-8111-111111111111","provider_id":"claude-code",
        "request_id":"login","generation":2,"method":"native","provider_version":"1",
        "generation_digest":"digest","auth_contract_fingerprint":"contract",
        "identity":{"authority":"claude","subject":"otter"}
    }))
    .unwrap();
    for (detail, expected) in [
        (
            "native_usage_authentication_required",
            "native_usage_authentication_required",
        ),
        (
            "native_usage_profile_changed",
            "native_usage_profile_changed",
        ),
        (
            "native stderr containing a private path or credential",
            "native_usage_unavailable",
        ),
    ] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = control.install(
            "machine".into(),
            "epoch".into(),
            false,
            crate::machine_protocol::NATIVE_USAGE_PROTOCOL_VERSION,
            tx,
        );
        let pending = control.begin_native_usage(&connection, &profile).unwrap();
        let MachineCommand::CollectNativeUsage { request_id, .. } = rx.try_recv().unwrap() else {
            panic!("scoped query expected")
        };
        control.record_remote(
            &connection,
            crate::machine_protocol::MachineEvent::PluginHostResponse {
                request_id,
                accepted: false,
                started: false,
                payload: None,
                detail: Some(detail.into()),
            },
        );
        assert_eq!(pending.await.unwrap_err(), expected);
    }
}

#[tokio::test]
async fn binding_preparation_requires_exact_typed_current_connection_receipt() {
    use crate::machine_protocol::MachineEvent;
    use crate::provider_account::{
        NativeIdentity, NativeProfileReceipt, PreparedBinding, WorkerExitObservation,
        WorkerExitRequest,
    };
    for mode in ["match", "scope", "missing", "connection"] {
        let dir = tempfile::tempdir().unwrap();
        let root = ProviderAuthService::open(dir.path()).unwrap();
        let control = MachineControl::default();
        let (executor, mut commands) = fixture(&root, &control, "binding-machine", 7);
        let binding = PreparedBinding {
            operation: "migration".into(),
            stopped: WorkerExitObservation {
                request: WorkerExitRequest {
                    account_id: executor.account_id.clone(),
                    session_id: "session".into(),
                    stop_command_id: Some("stop".into()),
                },
                launch_id: uuid::Uuid::new_v4(),
            },
            target: NativeProfileReceipt {
                account_id: executor.account_id.clone(),
                provider_id: "claude-code".into(),
                request_id: "login".into(),
                generation: 2,
                method: "native".into(),
                provider_version: "fixture".into(),
                generation_digest: "fixture".into(),
                auth_contract_fingerprint: "fixture".into(),
                identity: NativeIdentity {
                    authority: "fixture".into(),
                    subject: "private".into(),
                    tenant: None,
                },
            },
        };
        let mut pending = Box::pin(control.prepare_native_binding(
            &executor.connection,
            "binding-machine",
            &binding,
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        let command = commands.try_recv().unwrap();
        assert_eq!(
            command.minimum_protocol(),
            PROVIDER_ACCOUNT_PROTOCOL_VERSION
        );
        let MachineCommand::PrepareNativeBinding {
            request_id,
            binding: mut observed,
        } = command
        else {
            panic!("prepare expected")
        };
        assert_eq!(*observed, binding);
        control.record_remote(
            &executor.connection,
            MachineEvent::CommandResult {
                request_id: request_id.clone(),
                accepted: true,
                detail: None,
            },
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        if mode == "scope" {
            observed.target.generation += 1;
        }
        let event = MachineEvent::NativeBindingPrepared {
            request_id,
            binding: (mode != "missing").then_some(observed),
        };
        assert!(!machine_diagnostic_event_visible(&event));
        control.record_remote(&executor.connection, event);
        if mode == "connection" {
            let _replacement = fixture(&root, &control, "binding-machine", 7);
        }
        assert_eq!(pending.await.is_ok(), mode == "match");
        assert!(control.events("binding-machine").is_empty());
    }
}

#[tokio::test]
async fn worker_exit_requires_typed_scoped_current_connection_evidence() {
    use crate::machine_protocol::MachineEvent;
    use crate::provider_account::{WorkerExitObservation, WorkerExitRequest};
    for mode in ["match", "scope", "launch", "missing", "connection"] {
        let dir = tempfile::tempdir().unwrap();
        let root = ProviderAuthService::open(dir.path()).unwrap();
        let control = MachineControl::default();
        let (executor, mut commands) = fixture(&root, &control, "exit-machine", 7);
        let request = WorkerExitRequest {
            stop_command_id: None,
            account_id: executor.account_id.clone(),
            session_id: "session".into(),
        };
        let mut pending =
            Box::pin(control.query_worker_exit(&executor.connection, "exit-machine", &request));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        let command = commands.try_recv().unwrap();
        assert_eq!(
            command.minimum_protocol(),
            PROVIDER_ACCOUNT_PROTOCOL_VERSION
        );
        let MachineCommand::QueryWorkerExit { request_id, worker } = command else {
            panic!("worker query")
        };
        assert_eq!(worker, request);
        control.record_remote(
            &executor.connection,
            MachineEvent::CommandResult {
                request_id: request_id.clone(),
                accepted: true,
                detail: None,
            },
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        let mut observation = WorkerExitObservation {
            request: worker,
            launch_id: uuid::Uuid::new_v4(),
        };
        if mode == "scope" {
            observation.request.session_id = "other".into();
        }
        if mode == "launch" {
            observation.launch_id = uuid::Uuid::nil();
        }
        let event = MachineEvent::WorkerExitObserved {
            request_id,
            observation: (mode != "missing").then_some(observation),
        };
        assert!(!machine_diagnostic_event_visible(&event));
        control.record_remote(&executor.connection, event);
        if mode == "connection" {
            let _replacement = fixture(&root, &control, "exit-machine", 7);
        }
        assert_eq!(pending.await.is_ok(), mode == "match");
        assert!(control.events("exit-machine").is_empty());
    }
}

#[tokio::test]
async fn login_stop_requires_scoped_typed_evidence_on_current_connection() {
    use crate::machine_protocol::MachineEvent;
    use crate::provider_account::{LoginStopObservation, LoginStopRequest};
    for mode in ["match", "scope", "flags", "missing", "connection"] {
        let dir = tempfile::tempdir().unwrap();
        let root = ProviderAuthService::open(dir.path()).unwrap();
        let control = MachineControl::default();
        let (executor, mut commands) = fixture(&root, &control, "stop-machine", 7);
        let request = LoginStopRequest {
            account_id: executor.account_id.clone(),
            provider_id: "grok".into(),
            login_request_id: "original".into(),
        };
        let mut pending =
            Box::pin(control.query_login_stop(&executor.connection, "stop-machine", &request));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        let command = commands.try_recv().unwrap();
        assert_eq!(
            command.minimum_protocol(),
            PROVIDER_ACCOUNT_PROTOCOL_VERSION
        );
        let MachineCommand::QueryLoginStop { request_id, login } = command else {
            panic!("query expected")
        };
        assert_eq!(login, request);
        control.record_remote(
            &executor.connection,
            MachineEvent::CommandResult {
                request_id: request_id.clone(),
                accepted: true,
                detail: None,
            },
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        let mut observation = LoginStopObservation {
            request: login,
            process_started: true,
            leader_exited: true,
            candidate_removed: true,
            process_tree_exited: false,
        };
        if mode == "scope" {
            observation.request.login_request_id = "other".into();
        }
        if mode == "flags" {
            observation.leader_exited = false;
        }
        let event = MachineEvent::LoginStopObserved {
            request_id,
            observation: (mode != "missing").then_some(observation),
        };
        assert!(!machine_diagnostic_event_visible(&event));
        control.record_remote(&executor.connection, event);
        if mode == "connection" {
            let (_, mut replacement) = fixture(&root, &control, "stop-machine", 7);
            assert!(replacement.try_recv().is_err());
        }
        assert_eq!(pending.await.is_ok(), mode == "match");
        assert!(control.events("stop-machine").is_empty());
    }
}

#[tokio::test]
async fn native_logout_requires_typed_matching_reply_on_original_connection() {
    use crate::machine_protocol::MachineEvent;
    use crate::provider_account::{NativeLogoutObservation, NativeLogoutRequest};
    for mode in ["match", "scope", "connection"] {
        let dir = tempfile::tempdir().unwrap();
        let root = ProviderAuthService::open(dir.path()).unwrap();
        let control = MachineControl::default();
        let (executor, mut commands) = fixture(&root, &control, "logout-machine", 7);
        let logout = NativeLogoutRequest {
            account_id: executor.account_id.clone(),
            provider_id: "claude-code".into(),
            generation: 7,
            login_request_id: "login".into(),
            operation_id: "logout".into(),
        };
        let mut pending = Box::pin(
            control
                .begin_native_logout(&executor.connection, "logout-machine", &logout)
                .unwrap(),
        );
        // Dispatch must happen inside the caller's admission callback, before
        // it releases database locks or polls the returned response future.
        let command = commands.try_recv().expect("synchronous dispatch");
        assert_eq!(
            command.minimum_protocol(),
            PROVIDER_ACCOUNT_PROTOCOL_VERSION
        );
        let MachineCommand::NativeLogout {
            request_id,
            logout: sent,
        } = command
        else {
            panic!("logout command expected")
        };
        assert_eq!(sent, logout);
        control.record_remote(
            &executor.connection,
            MachineEvent::CommandResult {
                request_id: request_id.clone(),
                accepted: true,
                detail: None,
            },
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut pending)
                .await
                .is_err()
        );
        let mut observation = NativeLogoutObservation {
            request: sent,
            succeeded: Some(true),
            profile_cleaned: true,
            process_trees_exited: false,
        };
        if mode == "scope" {
            observation.request.generation += 1;
        }
        let event = MachineEvent::NativeLogoutObserved {
            request_id,
            observation: Some(observation),
        };
        assert!(!machine_diagnostic_event_visible(&event));
        control.record_remote(&executor.connection, event);
        if mode == "connection" {
            let (_, mut replacement) = fixture(&root, &control, "logout-machine", 7);
            assert!(replacement.try_recv().is_err());
            assert!(
                control
                    .begin_native_logout(&executor.connection, "logout-machine", &logout)
                    .is_err()
            );
            assert!(replacement.try_recv().is_err());
        }
        assert_eq!(pending.await.is_ok(), mode == "match");
        assert!(control.events("logout-machine").is_empty());
    }
}

#[tokio::test]
async fn native_observation_is_private_correlated_and_rechecks_connection() {
    use crate::machine_protocol::MachineEvent;
    for replace_connection in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = ProviderAuthService::open(dir.path()).unwrap();
        let control = MachineControl::default();
        let (executor, mut commands) = fixture(&root, &control, "native-query", 7);
        let receipt = serde_json::from_value(json!({
            "request_id":"original-login", "provider_id":"claude-code", "native_generation":7,
            "input":{"account_id":executor.account_id,"machine_id":"native-query",
                "provider_version":"fixture","generation_digest":"digest","method":"native"}
        }))
        .unwrap();
        let (result, ()) = tokio::join!(
            control.query_native_profile(&executor.connection, &receipt),
            async {
                let MachineCommand::QueryNativeProfile {
                    request_id,
                    login_request_id,
                    generation,
                    ..
                } = commands.recv().await.unwrap()
                else {
                    panic!("expected metadata query, never login dispatch");
                };
                assert_eq!(login_request_id, "original-login");
                assert_eq!(generation, 7);
                assert_ne!(request_id, login_request_id);
                control.record_remote(
                    &executor.connection,
                    MachineEvent::CommandResult {
                        request_id: request_id.clone(),
                        accepted: true,
                        detail: None,
                    },
                );
                let reply = MachineEvent::NativeProfileObserved {
                    request_id,
                    profile: None,
                };
                assert!(!machine_diagnostic_event_visible(&reply));
                control.record_remote(&executor.connection, reply);
                if replace_connection {
                    let (_, mut replacement) = fixture(&root, &control, "native-query", 7);
                    assert!(replacement.try_recv().is_err());
                }
            }
        );
        assert_eq!(result.is_err(), replace_connection);
        if !replace_connection {
            assert!(result.unwrap().is_none());
        }
        assert!(control.events("native-query").is_empty());
    }
}

#[test]
fn native_login_preparation_never_creates_a_portable_account() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    for key in ["machine-a", "machine-b"] {
        let LoginStorage::Native(login) =
            LoginStorage::prepare(&root, &account, "claude-code", key.into(), 7).unwrap()
        else {
            panic!("native-owned credentials must never use a portable scope")
        };
        assert_eq!(login.recipient_key, key);
    }
    assert!(!dir.path().join("accounts").exists());
    assert!(root.status("claude-code").is_none());
    assert!(
        LoginStorage::prepare(
            &root,
            &ProviderAccountId::default(),
            "claude-code",
            "machine".into(),
            7,
        )
        .is_err()
    );
    // Internal preparation is not permission to expose the public workflow.
    assert_eq!(
        public_login_allowed("claude-code", false),
        Err("native_login_lifecycle_not_enabled")
    );
    assert!(public_login_allowed("claude-code", true).is_ok());
    assert!(public_login_allowed("claude-deepseek", false).is_ok());
    assert!(public_login_allowed("claude-deepseek", true).is_ok());
}

#[test]
fn http_login_operations_are_stable_and_separate_from_mcp() {
    let operation = http_operation("attempt-1").unwrap();
    assert_eq!(operation, http_operation("attempt-1").unwrap());
    assert_ne!(operation, http_operation("attempt-2").unwrap());
    assert!(operation.starts_with("http-login-"));
    assert!(crate::managed_calls::valid_id(&operation));
    for invalid in ["", ".", "..", "a/b", "a?b", "a%20b"] {
        assert!(http_operation(invalid).is_err());
    }
    let request: Start = serde_json::from_value(json!({
        "operation":"attempt-1", "machine_id":"machine-a", "provider_version":"1",
        "generation_digest":"digest", "method":"native"
    }))
    .unwrap();
    assert_eq!(request.operation, "attempt-1");
    for operation in [None, Some(Value::Null)] {
        let mut input = json!({"machine_id":"machine-a", "provider_version":"1",
            "generation_digest":"digest", "method":"native"});
        if let Some(operation) = operation {
            input["operation"] = operation;
        }
        assert!(serde_json::from_value::<Start>(input).is_err());
    }
}

fn fixture(
    root: &ProviderAuthService,
    control: &MachineControl,
    machine: &str,
    seed: u8,
) -> (
    ProviderAuthExecutor,
    tokio::sync::mpsc::UnboundedReceiver<MachineCommand>,
) {
    let account_id =
        ProviderAccountId::try_from("11111111-1111-4111-8111-111111111111".to_owned()).unwrap();
    let key = base64::engine::general_purpose::STANDARD.encode(
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([seed; 32])).as_bytes(),
    );
    let scope = root
        .account(&account_id)
        .unwrap()
        .machine_grant(&key)
        .unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let connection = control.install(
        machine.into(),
        "epoch".into(),
        false,
        PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        tx,
    );
    (
        ProviderAuthExecutor {
            native_login: None,
            named_login: Some(NamedLogin {
                scope,
                completion: Default::default(),
                registry_revision: 1,
                recipient_key: key,
            }),
            cancel_requested: false,
            owner_user_id: "owner".into(),
            account_id,
            machine_id: machine.into(),
            connection,
            provider_id: "grok".into(),
            provider_version: "fixture".into(),
            generation_digest: "sha256:fixture".into(),
            auth_contract_fingerprint: "fixture".into(),
            auth_method: "xai-account".into(),
            expected_generation: 0,
            promotion_started: false,
            expires_at_ms: i64::MAX,
        },
        rx,
    )
}

fn package() -> cowboy_provider_sdk::ProviderPackage {
    let source: cowboy_provider_sdk::StandardProviderSource =
        serde_json::from_str(include_str!("../../../../../../plugins/grok/provider.json")).unwrap();
    cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
}

#[test]
fn native_dispatch_uses_reserved_generation_without_portable_scope() {
    use crate::store::provider_accounts::logins::{Claim, Receipt, Request};
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let (mut executor, mut commands) = fixture(&root, &control, "native", 7);
    executor.provider_id = "claude-code".into();
    executor.native_login = Some(NativeLogin {
        registry_revision: 1,
        recipient_key: executor.named_login.take().unwrap().recipient_key,
    });
    let mut claim = Claim {
        fresh: true,
        receipt: Receipt {
            request_id: "reserved-native".into(),
            provider_id: executor.provider_id.clone(),
            input: Request {
                account_id: executor.account_id.clone(),
                machine_id: executor.machine_id.clone(),
                provider_version: executor.provider_version.clone(),
                generation_digest: executor.generation_digest.clone(),
                method: executor.auth_method.clone(),
            },
            cancel_requested: false,
            dispatch_rejection: None,
            native_completion: None,
            native_logout: None,
            native_generation: Some(7),
            native_binding_revision: None,
            dispatch_recipient_fingerprint: None,
            dispatch_registry_revision: None,
        },
    };
    let requests = parking_lot::Mutex::new(HashMap::new());
    claim.fresh = false;
    assert_eq!(
        native::begin_reserved(&control, &requests, executor.clone(), &claim).status(),
        StatusCode::CONFLICT
    );
    assert!(commands.try_recv().is_err());
    claim.fresh = true;
    claim.receipt.input.machine_id = "wrong".into();
    assert_eq!(
        native::begin_reserved(&control, &requests, executor.clone(), &claim).status(),
        StatusCode::CONFLICT
    );
    assert!(commands.try_recv().is_err());
    claim.receipt.input.machine_id = "native".into();
    assert_eq!(
        native::begin_reserved(&control, &requests, executor.clone(), &claim).status(),
        StatusCode::ACCEPTED
    );
    let MachineCommand::BeginNativeLogin {
        request_id,
        account_id,
        spec,
        ..
    } = commands.try_recv().unwrap()
    else {
        panic!("native command required")
    };
    assert_eq!(request_id, claim.receipt.request_id);
    assert_eq!(account_id, executor.account_id);
    assert_eq!(spec.generation, 7);
    assert_eq!(spec.generation_digest, executor.generation_digest);
    assert_eq!(
        requests
            .lock()
            .get(&request_id)
            .unwrap()
            .expected_generation,
        6
    );
    assert!(
        requests
            .lock()
            .get(&request_id)
            .unwrap()
            .named_login
            .is_none()
    );
    assert!(root.status("claude-code").is_none());
    assert_eq!(
        native::begin_reserved(&control, &requests, executor, &claim).status(),
        StatusCode::CONFLICT
    );
    assert!(commands.try_recv().is_err());
}

#[test]
fn native_completion_requires_exact_live_request_and_stays_private() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let (mut executor, _commands) = fixture(&root, &control, "machine-native", 7);
    executor.provider_id = "claude-code".into();
    executor.native_login = Some(NativeLogin {
        registry_revision: 1,
        recipient_key: executor.named_login.take().unwrap().recipient_key,
    });
    let connection = executor.connection.clone();
    let profile = crate::provider_account::NativeProfileReceipt {
        account_id: executor.account_id.clone(),
        provider_id: executor.provider_id.clone(),
        request_id: "request".into(),
        generation: 1,
        method: executor.auth_method.clone(),
        provider_version: executor.provider_version.clone(),
        generation_digest: executor.generation_digest.clone(),
        auth_contract_fingerprint: executor.auth_contract_fingerprint.clone(),
        identity: crate::provider_account::NativeIdentity {
            authority: "fixture".into(),
            subject: "private".into(),
            tenant: None,
        },
    };
    assert!(native::validate_executor(&executor, &control, &connection, &profile).is_ok());
    for field in 0..6 {
        let mut wrong = profile.clone();
        match field {
            0 => wrong.account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            1 => wrong.generation = 2,
            2 => wrong.provider_version = "other".into(),
            3 => wrong.generation_digest = "other".into(),
            4 => wrong.auth_contract_fingerprint = "other".into(),
            _ => wrong.method = "other".into(),
        }
        assert!(native::validate_executor(&executor, &control, &connection, &wrong).is_err());
    }
    let event = crate::machine_protocol::MachineEvent::NativeProfileCompleted {
        profile: profile.clone(),
    };
    assert!(!machine_diagnostic_event_visible(&event));
    control.record_remote(&connection, event);
    assert!(control.events("machine-native").is_empty());
    executor.cancel_requested = true;
    assert!(native::validate_executor(&executor, &control, &connection, &profile).is_err());
    executor.cancel_requested = false;
    let (replacement, _commands) = fixture(&root, &control, "machine-native", 7);
    assert!(native::validate_executor(&executor, &control, &connection, &profile).is_err());
    assert!(
        native::validate_executor(&executor, &control, &replacement.connection, &profile).is_err()
    );
}

#[test]
fn one_subscription_can_login_independently_on_two_machines() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (first, mut first_commands) = fixture(&root, &control, "machine-a", 7);
    let (second, mut second_commands) = fixture(&root, &control, "machine-b", 8);
    let package = package();
    assert_eq!(
        begin(&control, &requests, first.clone(), &package).status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        begin(&control, &requests, second.clone(), &package).status(),
        StatusCode::ACCEPTED
    );
    let first_id = match first_commands.try_recv().unwrap() {
        MachineCommand::BeginLogin {
            account_id,
            request_id,
            ..
        } => {
            assert_eq!(account_id, first.account_id);
            request_id
        }
        _ => panic!("expected first login"),
    };
    let second_id = match second_commands.try_recv().unwrap() {
        MachineCommand::BeginLogin {
            account_id,
            request_id,
            ..
        } => {
            assert_eq!(account_id, second.account_id);
            request_id
        }
        _ => panic!("expected second login"),
    };
    assert_ne!(first_id, second_id);
    assert_eq!(
        begin(&control, &requests, first.clone(), &package).status(),
        StatusCode::ACCEPTED
    );
    assert!(first_commands.try_recv().is_err());
    let mut other_user = first.clone();
    other_user.owner_user_id = "other-user".into();
    assert_eq!(
        begin(&control, &requests, other_user, &package).status(),
        StatusCode::CONFLICT
    );
    requests.lock().get_mut(&first_id).unwrap().cancel_requested = true;
    assert_eq!(
        begin(&control, &requests, first, &package).status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        begin(&control, &requests, second, &package).status(),
        StatusCode::ACCEPTED
    );
    assert!(first_commands.try_recv().is_err());
    assert!(second_commands.try_recv().is_err());
    // Default login reconciliation cannot reap or resume a named request.
    let result =
        reconcile_provider_auth_executors(&mut requests.lock(), "grok", i64::MAX, &control);
    assert!(result.active.is_none());
    assert!(result.expired.is_empty());
    assert_eq!(requests.lock().len(), 2);
}

#[test]
fn named_login_body_cannot_override_owner_or_account() {
    let valid = json!({"operation":"attempt-1","machine_id":"machine-a","provider_version":"1","generation_digest":"digest","method":"native"});
    assert!(serde_json::from_value::<Start>(valid.clone()).is_ok());
    for field in ["owner_user_id", "account_id", "provider_id", "credentials"] {
        let mut value = valid.clone();
        value[field] = json!("override");
        assert!(serde_json::from_value::<Start>(value).is_err());
    }
}

#[test]
fn reserved_login_id_cannot_resume_a_different_request() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (executor, mut commands) = fixture(&root, &control, "machine-a", 7);
    let package = package();
    assert_eq!(
        begin_reserved(
            &control,
            &requests,
            executor.clone(),
            &package,
            Some("reserved-a")
        )
        .status(),
        StatusCode::ACCEPTED
    );
    let MachineCommand::BeginLogin {
        request_id,
        account_id,
        ..
    } = commands.try_recv().unwrap()
    else {
        panic!("expected native login")
    };
    assert_eq!(request_id, "reserved-a");
    assert_eq!(account_id, executor.account_id);
    assert_eq!(
        begin_reserved(
            &control,
            &requests,
            executor.clone(),
            &package,
            Some("reserved-b")
        )
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        begin_reserved(&control, &requests, executor, &package, Some("reserved-a")).status(),
        StatusCode::ACCEPTED
    );
    assert!(commands.try_recv().is_err());
    assert_eq!(requests.lock().len(), 1);
}

#[tokio::test]
async fn cleanup_waits_for_a_matching_receipt_and_can_retry_rejection() {
    use crate::machine_protocol::MachineEvent;
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let (executor, mut commands) = fixture(&root, &control, "machine-a", 7);
    let mut correlations = Vec::new();
    for accepted in [false, true] {
        let completed = AtomicBool::new(false);
        let (result, correlation) = tokio::join!(
            async {
                let result = executor
                    .named_login
                    .as_ref()
                    .unwrap()
                    .complete(
                        || Ok(()),
                        candidate::finalize(&control, &executor, "native-login"),
                    )
                    .await;
                completed.store(true, Ordering::SeqCst);
                result
            },
            async {
                let MachineCommand::FinalizeProviderAuthCandidate {
                    request_id,
                    account_id,
                    provider_id,
                    auth_method,
                    candidate_request_id,
                } = commands.recv().await.unwrap()
                else {
                    panic!("expected scoped cleanup");
                };
                assert!(
                    !completed.load(Ordering::SeqCst),
                    "enqueue alone is not a receipt"
                );
                assert_eq!(account_id, executor.account_id);
                assert_eq!(provider_id, executor.provider_id);
                assert_eq!(auth_method, executor.auth_method);
                assert_eq!(candidate_request_id, "native-login");
                control.record_remote(
                    &executor.connection,
                    MachineEvent::CommandResult {
                        request_id: request_id.clone(),
                        accepted,
                        detail: (!accepted).then(|| "retry cleanup".into()),
                    },
                );
                request_id
            }
        );
        assert_eq!(result.is_ok(), accepted);
        correlations.push(correlation);
    }
    assert_ne!(correlations[0], correlations[1]);
    candidate::record_completion(&control, &executor, "native-login");
    for index in 0..100 {
        candidate::record_completion(&control, &executor, &format!("other-{index}"));
    }
    assert!(
        !control
            .events("machine-a")
            .iter()
            .any(|event| matches!(event,
        MachineEvent::LoginState { request_id, .. } if request_id == "native-login"))
    );
    // Successful receipt is shared by executor clones; repeated callbacks
    // must not dispatch cleanup again, even after their events are gone.
    let cloned = executor.clone();
    cloned
        .named_login
        .as_ref()
        .unwrap()
        .complete(
            || Ok(()),
            candidate::finalize(&control, &cloned, "native-login"),
        )
        .await
        .unwrap();
    assert!(commands.try_recv().is_err());
    candidate::record_completion(&control, &cloned, "native-login");
    candidate::record_completion(&control, &cloned, "native-login");
    assert_eq!(control.events("machine-a").iter().filter(|event| matches!(event,
        MachineEvent::LoginState { request_id, state: crate::machine_protocol::AuthState::SignedIn, .. } if request_id == "native-login")).count(), 1);
    assert!(
        cloned
            .named_login
            .as_ref()
            .unwrap()
            .complete(|| Err("cancelled".into()), async {
                panic!("inactive completed login must not run work")
            },)
            .await
            .is_err()
    );
    // A request must never migrate to a replacement transport, even when its
    // Machine name and caller-supplied epoch are unchanged.
    let (_replacement, mut replacement_commands) = fixture(&root, &control, "machine-a", 7);
    assert!(
        candidate::finalize(&control, &executor, "native-login")
            .await
            .is_err()
    );
    assert!(replacement_commands.try_recv().is_err());
}

#[tokio::test]
async fn concurrent_login_completion_dispatches_cleanup_once() {
    use crate::machine_protocol::MachineEvent;
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let (executor, mut commands) = fixture(&root, &control, "machine-a", 7);
    let clone = executor.clone();
    let (first, second, ()) = tokio::join!(
        executor
            .named_login
            .as_ref()
            .unwrap()
            .complete(|| Ok(()), candidate::finalize(&control, &executor, "login")),
        clone
            .named_login
            .as_ref()
            .unwrap()
            .complete(|| Ok(()), candidate::finalize(&control, &clone, "login")),
        async {
            let MachineCommand::FinalizeProviderAuthCandidate { request_id, .. } =
                commands.recv().await.unwrap()
            else {
                panic!("expected cleanup")
            };
            tokio::task::yield_now().await;
            assert!(commands.try_recv().is_err());
            control.record_remote(
                &executor.connection,
                MachineEvent::CommandResult {
                    request_id,
                    accepted: true,
                    detail: None,
                },
            );
        },
    );
    first.unwrap();
    second.unwrap();
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn interrupted_completion_remains_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let (executor, _commands) = fixture(&root, &control, "machine-a", 7);
    let login = executor.named_login.as_ref().unwrap();
    let (started, mut received) = tokio::sync::oneshot::channel();
    {
        let interrupted = login.complete(|| Ok(()), async {
            started.send(()).unwrap();
            std::future::pending::<Result<(), String>>().await
        });
        tokio::pin!(interrupted);
        tokio::select! {
            result = &mut interrupted => panic!("unexpected completion: {result:?}"),
            result = &mut received => result.unwrap(),
        }
    }
    assert!(!*login.completion.try_lock().unwrap());
    let ran = std::sync::atomic::AtomicBool::new(false);
    login
        .complete(|| Ok(()), async {
            ran.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
    assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn cancellation_keeps_failed_dispatch_and_never_targets_a_replacement_connection() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (mut executor, commands) = fixture(&root, &control, "machine-a", 7);
    executor.cancel_requested = true;
    requests.lock().insert("login".into(), executor.clone());
    drop(commands); // Closed transport queue: enqueue must fail.
    assert!(send_cancel(&control, &requests, &executor, "login").is_err());
    assert!(requests.lock().get("login").unwrap().cancel_requested);
    let (replacement, mut replacement_commands) = fixture(&root, &control, "machine-a", 7);
    assert!(send_cancel(&control, &requests, &executor, "login").is_err());
    assert!(send_cancel(&control, &requests, &replacement, "login").is_err());
    assert!(replacement_commands.try_recv().is_err());
    assert_eq!(
        begin(&control, &requests, replacement, &package()).status(),
        StatusCode::CONFLICT
    );
    assert!(requests.lock().contains_key("login"));
    assert!(replacement_commands.try_recv().is_err());
}

#[tokio::test]
async fn native_cancel_after_reconnect_requires_current_acceptance_before_retiring_tracker() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(&dir.path().join("auth")).unwrap();
    let store = crate::store::Store::connect(
        &format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("cancel.sqlite").display()
        ),
        dir.path().join("artifacts"),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (mut old, mut old_commands) = fixture(&root, &control, "native", 7);
    old.provider_id = "claude-code".into();
    old.native_login = Some(NativeLogin {
        registry_revision: 1,
        recipient_key: old.named_login.take().unwrap().recipient_key,
    });
    old.cancel_requested = true;
    requests.lock().insert("old-login".into(), old.clone());
    let key = &old.native_login.as_ref().unwrap().recipient_key;
    let enrollment = store
        .create_machine_enrollment("native", "Native", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), key)
        .await
        .unwrap();
    let receipt: crate::store::provider_accounts::logins::Receipt = serde_json::from_value(json!({
        "request_id":"old-login", "provider_id":"claude-code", "cancel_requested":true,
        "native_generation":1, "dispatch_registry_revision":1,
        "dispatch_recipient_fingerprint":crate::admin::hex_sha256(key.as_bytes()),
        "input":{"account_id":old.account_id,"machine_id":"native","provider_version":"fixture","generation_digest":"digest","method":"native"}
    })).unwrap();
    let (replacement, mut commands) = fixture(&root, &control, "native", 7);
    assert!(!control.is_current(&old.connection));
    for accepted in [false, true] {
        let (result, ()) =
            tokio::join!(native_cancel::recover(&store, &control, &receipt), async {
                let MachineCommand::CancelLoginRequest {
                    request_id,
                    login_request_id,
                } = commands.recv().await.unwrap()
                else {
                    panic!("correlated cancellation required");
                };
                assert_eq!(login_request_id, "old-login");
                assert!(requests.lock().contains_key("old-login"));
                control.record_remote(
                    &replacement.connection,
                    crate::machine_protocol::MachineEvent::CommandResult {
                        request_id,
                        accepted,
                        detail: None,
                    },
                );
            });
        if result == Ok(true) {
            forget_cancelled_executor(&requests, &old, "old-login").unwrap();
        }
        assert_eq!(requests.lock().contains_key("old-login"), !accepted);
        assert!(old_commands.try_recv().is_err());
        assert!(commands.try_recv().is_err());
    }
    // A replacement or another owner cannot be erased by the old cancellation.
    requests.lock().insert("old-login".into(), replacement);
    assert!(forget_cancelled_executor(&requests, &old, "old-login").is_err());
    assert!(requests.lock().contains_key("old-login"));
    assert!(root.status("claude-code").is_none());
}

#[tokio::test]
async fn correlated_cancel_ignores_login_ack_and_preserves_refused_request_for_retry() {
    use crate::machine_protocol::MachineEvent;
    use std::time::Duration;
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (mut executor, mut commands) = fixture(&root, &control, "machine-a", 7);
    executor.cancel_requested = true;
    requests.lock().insert("login".into(), executor.clone());
    for accepted in [false, true] {
        let (result, ()) = tokio::join!(
            send_confirmed_cancel(&control, &requests, &executor, "login"),
            async {
                let MachineCommand::CancelLoginRequest {
                    request_id,
                    login_request_id,
                } = commands.recv().await.unwrap()
                else {
                    panic!("correlated cancellation expected")
                };
                assert_eq!(login_request_id, "login");
                assert_ne!(request_id, login_request_id);
                // A late SubmitLoginCode or BeginLogin result cannot confirm cancellation.
                control.record_remote(
                    &executor.connection,
                    MachineEvent::CommandResult {
                        request_id: "login".into(),
                        accepted: true,
                        detail: None,
                    },
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert!(requests.lock().contains_key("login"));
                control.record_remote(
                    &executor.connection,
                    MachineEvent::CommandResult {
                        request_id,
                        accepted,
                        detail: (!accepted).then(|| "cleanup unconfirmed".into()),
                    },
                );
            }
        );
        assert_eq!(result.is_ok(), accepted);
        assert_eq!(requests.lock().contains_key("login"), !accepted);
    }
}

#[tokio::test]
async fn correlated_cancel_rechecks_transport_and_registration_after_ack() {
    use crate::machine_protocol::MachineEvent;
    for replace_transport in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = ProviderAuthService::open(dir.path()).unwrap();
        let control = MachineControl::default();
        let requests = parking_lot::Mutex::new(HashMap::new());
        let (mut executor, mut commands) = fixture(&root, &control, "machine-a", 7);
        executor.cancel_requested = true;
        requests.lock().insert("login".into(), executor.clone());
        let (result, ()) = tokio::join!(
            send_confirmed_cancel(&control, &requests, &executor, "login"),
            async {
                let MachineCommand::CancelLoginRequest { request_id, .. } =
                    commands.recv().await.unwrap()
                else {
                    panic!("correlated cancellation expected")
                };
                control.record_remote(
                    &executor.connection,
                    MachineEvent::CommandResult {
                        request_id,
                        accepted: true,
                        detail: None,
                    },
                );
                // No yield between reply delivery and replacement: the waiter
                // must validate again when it resumes.
                if replace_transport {
                    let (_, mut replacement_commands) = fixture(&root, &control, "machine-a", 7);
                    assert!(replacement_commands.try_recv().is_err());
                } else {
                    requests.lock().get_mut("login").unwrap().owner_user_id = "other-owner".into();
                }
            }
        );
        assert!(result.is_err());
        assert!(requests.lock().contains_key("login"));
    }
}

#[tokio::test]
async fn interrupted_correlated_cancel_retains_request_and_rejects_old_ack_on_retry() {
    use crate::machine_protocol::MachineEvent;
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (mut executor, mut commands) = fixture(&root, &control, "machine-a", 7);
    executor.cancel_requested = true;
    requests.lock().insert("login".into(), executor.clone());
    let old = {
        let waiting = send_confirmed_cancel(&control, &requests, &executor, "login");
        tokio::pin!(waiting);
        tokio::select! {
            result = &mut waiting => panic!("unexpected completion: {result:?}"),
            command = commands.recv() => command.unwrap(),
        }
    }; // Drop the waiting future after dispatch, before acknowledgment.
    assert!(requests.lock().contains_key("login"));
    let MachineCommand::CancelLoginRequest {
        request_id: old_id, ..
    } = old
    else {
        panic!("correlated cancellation expected")
    };
    let (result, ()) = tokio::join!(
        send_confirmed_cancel(&control, &requests, &executor, "login"),
        async {
            let MachineCommand::CancelLoginRequest { request_id, .. } =
                commands.recv().await.unwrap()
            else {
                panic!("correlated cancellation expected")
            };
            assert_ne!(old_id, request_id);
            control.record_remote(
                &executor.connection,
                MachineEvent::CommandResult {
                    request_id: old_id,
                    accepted: true,
                    detail: None,
                },
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            assert!(requests.lock().contains_key("login"));
            control.record_remote(
                &executor.connection,
                MachineEvent::CommandResult {
                    request_id,
                    accepted: true,
                    detail: None,
                },
            );
        }
    );
    result.unwrap();
    assert!(!requests.lock().contains_key("login"));
}

#[test]
fn cancellation_is_owner_checked_and_concurrent_retries_enqueue_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (mut executor, mut commands) = fixture(&root, &control, "machine-a", 7);
    requests.lock().insert("login".into(), executor.clone());
    assert!(send_cancel(&control, &requests, &executor, "login").is_err());
    executor.cancel_requested = true;
    requests.lock().insert("login".into(), executor.clone());
    let mut other = executor.clone();
    other.owner_user_id = "other-owner".into();
    assert!(send_cancel(&control, &requests, &other, "login").is_err());
    assert!(commands.try_recv().is_err());
    std::thread::scope(|scope| {
        let first = scope.spawn(|| send_cancel(&control, &requests, &executor, "login"));
        let second = scope.spawn(|| send_cancel(&control, &requests, &executor, "login"));
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    });
    assert!(
        matches!(commands.try_recv().unwrap(),MachineCommand::CancelLogin {request_id} if request_id == "login")
    );
    assert!(commands.try_recv().is_err());
    assert!(requests.lock().is_empty());
}

#[test]
fn unrelated_login_cannot_reap_an_expired_request_when_cancel_dispatch_fails() {
    let dir = tempfile::tempdir().unwrap();
    let root = ProviderAuthService::open(dir.path()).unwrap();
    let control = MachineControl::default();
    let requests = parking_lot::Mutex::new(HashMap::new());
    let (mut expired, old_commands) = fixture(&root, &control, "machine-a", 7);
    expired.expires_at_ms = 0;
    requests.lock().insert("expired".into(), expired);
    drop(old_commands);
    let (next, mut commands) = fixture(&root, &control, "machine-b", 8);
    assert_eq!(
        begin(&control, &requests, next, &package()).status(),
        StatusCode::ACCEPTED
    );
    assert!(matches!(
        commands.try_recv().unwrap(),
        MachineCommand::BeginLogin { .. }
    ));
    assert!(requests.lock().get("expired").unwrap().cancel_requested);
    assert_eq!(requests.lock().len(), 2);
}

use super::*;
use crate::{
    machine_protocol::{MachineCommand, MachineEvent, PROVIDER_ACCOUNT_PROTOCOL_VERSION},
    store::provider_accounts::{
        Change, Mutation, MutationResult,
        logins::{DispatchScope, Request},
    },
};
use base64::Engine as _;

#[test]
fn native_logout_missing_generation_is_not_permission_to_clean_up() {
    let mut login: crate::store::provider_accounts::logins::Receipt =
        serde_json::from_value(serde_json::json!({
            "request_id":"original", "provider_id":"claude-code", "input": {
                "account_id":"11111111-1111-4111-8111-111111111111", "machine_id":"machine",
                "provider_version":"fixture", "generation_digest":"digest", "method":"native"
            }
        }))
        .unwrap();
    assert!(native_generation(&login).is_err());
    login.native_generation = Some(7);
    assert_eq!(native_generation(&login).unwrap(), Some(7));
    login.native_generation = None;
    login.provider_id = "grok".into();
    assert_eq!(native_generation(&login).unwrap(), None);
}

#[tokio::test]
async fn native_logout_old_success_is_reconciled_until_cleanup_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    fixture(&store).await;
    let control = MachineControl::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let connection = control.install(
        "machine".into(),
        "epoch".into(),
        false,
        PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        tx,
    );
    let mut original = None;
    for profile_cleaned in [false, true] {
        let (result, ()) = tokio::join!(run(&store, &control, "owner", "revoke"), async {
            let MachineCommand::NativeLogout { request_id, logout } = rx.recv().await.unwrap()
            else {
                panic!("logout")
            };
            if let Some(original) = &original {
                assert_eq!(&logout, original);
            } else {
                original = Some(logout.clone());
            }
            control.record_remote(
                &connection,
                MachineEvent::NativeLogoutObserved {
                    request_id,
                    observation: Some(NativeLogoutObservation {
                        request: logout,
                        succeeded: Some(true),
                        profile_cleaned,
                        process_trees_exited: false,
                    }),
                },
            );
        });
        let result = result.unwrap();
        assert_eq!(permits_cleanup(&result), profile_cleaned);
        assert_eq!(
            result[0].state,
            if profile_cleaned {
                State::ProfileCleaned
            } else {
                State::CommandSucceeded
            }
        );
        let saved = store
            .provider_login_receipt("owner", "login")
            .await
            .unwrap()
            .unwrap()
            .native_logout
            .unwrap();
        assert_eq!(saved.observation.profile_cleaned, profile_cleaned);
        if profile_cleaned {
            let mut downgrade = saved;
            downgrade.observation.profile_cleaned = false;
            assert!(
                store
                    .record_native_logout_completion("owner", "login", &downgrade, || true)
                    .await
                    .is_err()
            );
        }
    }
    let final_pass = run(&store, &control, "owner", "revoke").await.unwrap();
    assert!(permits_cleanup(&final_pass));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn native_logout_lost_response_retries_original_scope_and_records_observation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    fixture(&store).await;
    let control = MachineControl::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let connection = control.install(
        "machine".into(),
        "epoch".into(),
        false,
        PROVIDER_ACCOUNT_PROTOCOL_VERSION,
        tx,
    );
    let first = run(&store, &control, "owner", "revoke").await.unwrap();
    assert_eq!(first[0].state, State::Unconfirmed);
    assert!(!permits_cleanup(&first));
    let MachineCommand::NativeLogout {
        request_id: old_id,
        logout: original,
    } = rx.try_recv().unwrap()
    else {
        panic!("logout")
    };
    control.record_remote(
        &connection,
        MachineEvent::NativeLogoutObserved {
            request_id: old_id.clone(),
            observation: Some(NativeLogoutObservation {
                profile_cleaned: true,
                process_trees_exited: false,
                request: original.clone(),
                succeeded: Some(true),
            }),
        },
    );
    assert!(
        store
            .provider_login_receipt("owner", "login")
            .await
            .unwrap()
            .unwrap()
            .native_logout
            .is_none()
    );
    let (retry, ()) = tokio::join!(run(&store, &control, "owner", "revoke"), async {
        let MachineCommand::NativeLogout { request_id, logout } = rx.recv().await.unwrap() else {
            panic!("logout")
        };
        assert_ne!(request_id, old_id);
        assert_eq!(logout, original);
        control.record_remote(
            &connection,
            MachineEvent::NativeLogoutObserved {
                request_id,
                observation: Some(NativeLogoutObservation {
                    profile_cleaned: true,
                    process_trees_exited: false,
                    request: logout,
                    succeeded: Some(true),
                }),
            },
        );
    });
    assert!(permits_cleanup(&retry.unwrap()));
    assert!(
        store
            .provider_login_receipt("owner", "login")
            .await
            .unwrap()
            .unwrap()
            .native_logout
            .is_some()
    );
}

async fn fixture(store: &Store) {
    store.migrate().await.unwrap();
    let MutationResult::Applied(created) = store
        .mutate_provider_accounts(
            "owner",
            "create",
            &Mutation {
                expected_revision: 0,
                change: Change::Create {
                    provider_id: "claude-code".into(),
                    label: "Personal".into(),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("create")
    };
    let key = base64::engine::general_purpose::STANDARD.encode(
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([19; 32])).as_bytes(),
    );
    let enrollment = store
        .create_machine_enrollment("machine", "Native", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
        .await
        .unwrap();
    let claim = store
        .claim_provider_login(
            "owner",
            "login",
            &Request {
                account_id: created.account_id.clone(),
                machine_id: "machine".into(),
                provider_version: "fixture".into(),
                generation_digest: "digest".into(),
                method: "native".into(),
            },
        )
        .await
        .unwrap();
    store
        .with_provider_login_dispatch(
            "owner",
            "login",
            &claim,
            &DispatchScope {
                registry_revision: created.registry.revision,
                recipient_fingerprint: crate::admin::hex_sha256(key.as_bytes()),
            },
            || Ok(()),
        )
        .await
        .unwrap();
    store
        .mutate_provider_accounts(
            "owner",
            "revoke",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::BeginRevocation {
                    id: created.account_id,
                },
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn native_logout_reconciles_typed_results_and_recovers_without_redispatch() {
    for succeeded in [Some(true), Some(false), None] {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("db").display());
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        fixture(&store).await;
        let control = MachineControl::default();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let connection = control.install(
            "machine".into(),
            "epoch".into(),
            false,
            PROVIDER_ACCOUNT_PROTOCOL_VERSION,
            tx,
        );
        let (result, ()) = tokio::join!(run(&store, &control, "owner", "revoke"), async {
            let MachineCommand::NativeLogout { request_id, logout } = rx.recv().await.unwrap()
            else {
                panic!("logout")
            };
            assert_eq!(logout.operation_id, "revoke");
            control.record_remote(
                &connection,
                MachineEvent::NativeLogoutObserved {
                    request_id,
                    observation: Some(NativeLogoutObservation {
                        profile_cleaned: succeeded == Some(true),
                        process_trees_exited: false,
                        request: logout,
                        succeeded,
                    }),
                },
            );
        });
        let result = result.unwrap();
        let expected = match succeeded {
            Some(true) => State::ProfileCleaned,
            Some(false) => State::CommandFailed,
            None => State::Unconfirmed,
        };
        assert_eq!(result[0].state, expected);
        assert_eq!(permits_cleanup(&result), succeeded == Some(true));
        assert_eq!(
            store
                .provider_login_receipt("owner", "login")
                .await
                .unwrap()
                .unwrap()
                .native_logout
                .is_some(),
            succeeded.is_some()
        );
        drop(store);
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        // No live Machine: persisted success AND failure remain observable.
        let offline = MachineControl::default();
        let replay = run(&store, &offline, "owner", "revoke").await.unwrap();
        assert_eq!(replay[0].state, expected);
        assert!(rx.try_recv().is_err());
        assert!(
            run(&store, &control, "other-owner", "revoke")
                .await
                .is_err()
        );
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn native_logout_refuses_legacy_protocol_without_enqueuing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    fixture(&store).await;
    let control = MachineControl::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    control.install(
        "machine".into(),
        "old".into(),
        false,
        PROVIDER_ACCOUNT_PROTOCOL_VERSION - 1,
        tx,
    );
    let result = run(&store, &control, "owner", "revoke").await.unwrap();
    assert_eq!(result[0].state, State::Unconfirmed);
    assert!(!permits_cleanup(&result));
    assert!(rx.try_recv().is_err());
}

use super::*;
use crate::provider_account::{LoginStopObservation, LoginStopRequest};
use crate::store::provider_accounts::{
    Change, Mutation, MutationResult,
    logins::{DispatchScope, Request},
};
use base64::Engine as _;

async fn fixture(store: &Store) -> Evidence {
    store.migrate().await.unwrap();
    let MutationResult::Applied(created) = store
        .mutate_provider_accounts(
            "owner",
            "create",
            &Mutation {
                expected_revision: 0,
                change: Change::Create {
                    provider_id: "grok".into(),
                    label: "Personal".into(),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("create")
    };
    let key = base64::engine::general_purpose::STANDARD.encode([19; 32]);
    let enrollment = store
        .create_machine_enrollment("machine", "Fixture", 60)
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
                method: "device".into(),
            },
        )
        .await
        .unwrap();
    let fingerprint = crate::admin::hex_sha256(key.as_bytes());
    store
        .with_provider_login_dispatch(
            "owner",
            "login",
            &claim,
            &DispatchScope {
                registry_revision: created.registry.revision,
                recipient_fingerprint: fingerprint.clone(),
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
                    id: created.account_id.clone(),
                },
            },
        )
        .await
        .unwrap();
    Evidence {
        login_operation: "login".into(),
        machine_id: "machine".into(),
        recipient_fingerprint: fingerprint,
        observation: LoginStopObservation {
            request: LoginStopRequest {
                account_id: created.account_id,
                provider_id: "grok".into(),
                login_request_id: claim.receipt.request_id,
            },
            process_started: true,
            leader_exited: true,
            candidate_removed: true,
            process_tree_exited: true,
        },
    }
}

#[tokio::test]
async fn login_stop_journal_validates_scope_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("db").display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let evidence = fixture(&store).await;
    assert!(
        store
            .record_provider_login_stop("other", "revoke", &evidence, || true)
            .await
            .is_err()
    );
    assert!(
        store
            .record_provider_login_stop("owner", "create", &evidence, || true)
            .await
            .is_err()
    );
    reject_invalid(&store, &evidence).await;
    assert!(
        store
            .record_provider_login_stop("owner", "revoke", &evidence, || false)
            .await
            .is_err()
    );
    assert!(
        store
            .provider_account_receipt("owner", "revoke")
            .await
            .unwrap()
            .unwrap()
            .login_stops
            .is_empty()
    );
    let (one, two) = tokio::join!(
        store.record_provider_login_stop("owner", "revoke", &evidence, || true),
        store.record_provider_login_stop("owner", "revoke", &evidence, || true),
    );
    assert_eq!(one.unwrap(), evidence);
    assert_eq!(two.unwrap(), evidence);
    assert!(
        store
            .provider_revocation_readiness("owner", "revoke")
            .await
            .unwrap()
            .missing
            .is_empty()
    );
    let mut legacy = evidence.clone();
    legacy.observation.process_tree_exited = false;
    assert!(
        store
            .record_provider_login_stop("owner", "revoke", &legacy, || true)
            .await
            .is_err()
    );
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    assert_eq!(
        store
            .provider_account_receipt("owner", "revoke")
            .await
            .unwrap()
            .unwrap()
            .login_stops["login"],
        evidence
    );
    let StorageBackend::Sqlite(db) = &store.backend else {
        panic!("SQLite")
    };
    sqlx::query("UPDATE machines SET encryption_public_key='replacement' WHERE id='machine'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        store
            .record_provider_login_stop("owner", "revoke", &evidence, || panic!(
                "historical replay needs no connection"
            ))
            .await
            .unwrap(),
        evidence
    );
    let mut conflict = evidence;
    conflict.observation.process_started = false;
    conflict.observation.leader_exited = false;
    assert!(
        store
            .record_provider_login_stop("owner", "revoke", &conflict, || true)
            .await
            .is_err()
    );
}

async fn reject_invalid(store: &Store, evidence: &Evidence) {
    for mode in [
        "operation",
        "machine",
        "fingerprint",
        "request",
        "provider",
        "account",
        "unfinished",
        "flags",
        "tree_without_process",
    ] {
        let mut bad = evidence.clone();
        match mode {
            "operation" => bad.login_operation = "unknown".into(),
            "machine" => bad.machine_id = "other".into(),
            "fingerprint" => bad.recipient_fingerprint = "other".into(),
            "request" => bad.observation.request.login_request_id = "other".into(),
            "provider" => bad.observation.request.provider_id = "codex".into(),
            "account" => {
                bad.observation.request.account_id =
                    uuid::Uuid::new_v4().to_string().try_into().unwrap();
            }
            "unfinished" => bad.observation.candidate_removed = false,
            "tree_without_process" => {
                bad.observation.process_started = false;
                bad.observation.leader_exited = false;
                bad.observation.process_tree_exited = true;
            }
            _ => bad.observation.leader_exited = false,
        }
        assert!(
            store
                .record_provider_login_stop("owner", "revoke", &bad, || panic!(
                    "invalid scope reached connection check"
                ))
                .await
                .is_err(),
            "{mode}"
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_login_stop_journal_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let evidence = fixture(&store).await;
    reject_invalid(&store, &evidence).await;
    assert!(
        store
            .record_provider_login_stop("owner", "revoke", &evidence, || false)
            .await
            .is_err()
    );
    let (one, two) = tokio::join!(
        store.record_provider_login_stop("owner", "revoke", &evidence, || true),
        store.record_provider_login_stop("owner", "revoke", &evidence, || true),
    );
    assert_eq!(one.unwrap(), evidence);
    assert_eq!(two.unwrap(), evidence);
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    assert_eq!(
        store
            .record_provider_login_stop("owner", "revoke", &evidence, || panic!("historical read"))
            .await
            .unwrap(),
        evidence
    );
    assert_eq!(
        store
            .provider_account_receipt("owner", "revoke")
            .await
            .unwrap()
            .unwrap()
            .login_stops["login"],
        evidence
    );
}

#[tokio::test]
async fn login_stop_journal_refuses_changed_enrollment_before_recording() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    let evidence = fixture(&store).await;
    let StorageBackend::Sqlite(db) = &store.backend else {
        panic!("SQLite")
    };
    sqlx::query("UPDATE machines SET encryption_public_key='replacement' WHERE id='machine'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        store
            .record_provider_login_stop("owner", "revoke", &evidence, || panic!(
                "changed enrollment cannot record"
            ))
            .await
            .is_err()
    );
    assert!(
        store
            .provider_account_receipt("owner", "revoke")
            .await
            .unwrap()
            .unwrap()
            .login_stops
            .is_empty()
    );
}

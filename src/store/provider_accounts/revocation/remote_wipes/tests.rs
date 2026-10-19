use super::*;
use crate::store::provider_accounts::{Change, Mutation};
use base64::Engine as _;

const OWNER: &str = "grant-revocation-owner";

async fn fixture(store: &Store) -> Evidence {
    store.migrate().await.unwrap();
    let fixtures = super::super::grant_tests::fixture(store).await;
    let mut grant = fixtures
        .iter()
        .find(|(_, grant)| grant.operation_id == "committed")
        .unwrap()
        .1
        .clone();
    let key = base64::engine::general_purpose::STANDARD.encode(
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([27; 32])).as_bytes(),
    );
    let enrollment = store
        .create_machine_enrollment(&grant.machine_id, "Native", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
        .await
        .unwrap();
    grant.recipient_fingerprint = crate::admin::hex_sha256(key.as_bytes());
    let StorageBackend::Sqlite(db) = &store.backend else {
        panic!("SQLite fixture")
    };
    sqlx::query(
        "UPDATE provider_account_grants SET value=$3 WHERE owner_user_id=$1 AND operation_id=$2",
    )
    .bind(OWNER)
    .bind(&grant.operation_id)
    .bind(serde_json::to_string(&grant).unwrap())
    .execute(&db.pool)
    .await
    .unwrap();
    store
        .mutate_provider_accounts(
            OWNER,
            "revoke",
            &Mutation {
                expected_revision: 1,
                change: Change::BeginRevocation {
                    id: grant.account_id,
                },
            },
        )
        .await
        .unwrap();
    Evidence {
        grant_operation: grant.operation_id,
        machine_id: grant.machine_id,
        recipient_fingerprint: grant.recipient_fingerprint,
        generation: 2,
    }
}

#[tokio::test]
async fn remote_wipe_receipt_rejects_reenrollment_before_first_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    let evidence = fixture(&store).await;
    let StorageBackend::Sqlite(db) = &store.backend else {
        panic!("SQLite fixture")
    };
    sqlx::query("UPDATE machines SET encryption_public_key='replacement' WHERE id=$1")
        .bind(&evidence.machine_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        store
            .record_provider_remote_wipe(OWNER, "revoke", &evidence, || panic!(
                "changed enrollment must fail before the connection callback"
            ))
            .await
            .is_err()
    );
    assert!(
        store
            .provider_account_receipt(OWNER, "revoke")
            .await
            .unwrap()
            .unwrap()
            .remote_wipes
            .is_empty()
    );
}

#[tokio::test]
async fn remote_wipe_receipt_is_scoped_idempotent_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("db").display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let evidence = fixture(&store).await;
    assert!(
        store
            .record_provider_remote_wipe("other-owner", "revoke", &evidence, || true)
            .await
            .is_err()
    );
    assert!(
        store
            .record_provider_remote_wipe(OWNER, "revoke", &evidence, || false)
            .await
            .is_err()
    );
    for mismatch in ["generation", "recipient", "machine", "grant"] {
        let mut wrong = evidence.clone();
        match mismatch {
            "generation" => wrong.generation = 1,
            "recipient" => wrong.recipient_fingerprint = "wrong".into(),
            "machine" => wrong.machine_id = "other".into(),
            _ => wrong.grant_operation = "cancelled".into(),
        }
        assert!(
            store
                .record_provider_remote_wipe(OWNER, "revoke", &wrong, || panic!(
                    "invalid scope cannot reach connection check"
                ))
                .await
                .is_err()
        );
    }
    let (first, second) = tokio::join!(
        store.record_provider_remote_wipe(OWNER, "revoke", &evidence, || true),
        store.record_provider_remote_wipe(OWNER, "revoke", &evidence, || true)
    );
    assert_eq!(first.unwrap(), evidence);
    assert_eq!(second.unwrap(), evidence);
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let StorageBackend::Sqlite(db) = &store.backend else {
        panic!("SQLite fixture")
    };
    sqlx::query("UPDATE machines SET encryption_public_key='changed' WHERE id=$1")
        .bind(&evidence.machine_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        store
            .record_provider_remote_wipe(OWNER, "revoke", &evidence, || panic!(
                "historical read cannot reacquire authority"
            ))
            .await
            .unwrap(),
        evidence
    );
    let receipt = store
        .provider_account_receipt(OWNER, "revoke")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.remote_wipes.get("committed"), Some(&evidence));
    assert!(
        receipt
            .registry
            .accounts
            .iter()
            .all(|account| !account.enabled && account.revoking)
    );
    let mut conflict = evidence;
    conflict.generation += 1;
    assert!(
        store
            .record_provider_remote_wipe(OWNER, "revoke", &conflict, || true)
            .await
            .is_err()
    );
}

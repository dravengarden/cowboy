use super::*;
use crate::store::provider_accounts::{Mutation, MutationResult};

fn key(seed: u8) -> String {
    use base64::Engine as _;
    let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([seed; 32]));
    base64::engine::general_purpose::STANDARD.encode(public.as_bytes())
}

async fn enroll(store: &Store, machine: &str, key: &str) {
    let token = store
        .create_machine_enrollment(machine, machine, 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&token, &format!("ssh-ed25519 {key}"), key)
        .await
        .unwrap();
}

async fn enrollment_fences_identity_binding(store: &Store, input: &Prepare) {
    let before = store.provider_accounts("owner").await.unwrap();
    assert_eq!(
        store
            .prepare_provider_grant("owner", "missing", input)
            .await
            .unwrap(),
        Outcome::Rejected("machine_unavailable")
    );
    enroll(store, "machine-a", &key(7)).await;
    let mut wrong = input.clone();
    wrong.recipient_fingerprint = "a".repeat(64);
    assert_eq!(
        store
            .prepare_provider_grant("owner", "wrong-key", &wrong)
            .await
            .unwrap(),
        Outcome::Rejected("machine_enrollment_changed")
    );
    enroll(store, "machine-c", &key(9)).await;
    wrong.machine_id = "machine-c".into();
    wrong.recipient_fingerprint = crate::admin::hex_sha256(key(9).as_bytes());
    store.revoke_machine("machine-c").await.unwrap();
    assert_eq!(
        store
            .prepare_provider_grant("owner", "revoked", &wrong)
            .await
            .unwrap(),
        Outcome::Rejected("machine_unavailable")
    );
    enroll(store, "machine-c", &key(10)).await;
    assert_eq!(
        store
            .prepare_provider_grant("owner", "reenrolled", &wrong)
            .await
            .unwrap(),
        Outcome::Rejected("machine_enrollment_changed")
    );
    assert_eq!(store.provider_accounts("owner").await.unwrap(), before);
    for operation in ["missing", "wrong-key", "revoked", "reenrolled"] {
        assert!(
            store
                .provider_grant_receipt("owner", operation)
                .await
                .unwrap()
                .is_none()
        );
    }
}

async fn contract(store: &Store) {
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
        panic!("create failed")
    };
    let input = Prepare {
        expected_revision: created.registry.revision,
        account_id: created.account_id,
        provider_id: "claude-code".into(),
        machine_id: "machine-a".into(),
        recipient_fingerprint: crate::admin::hex_sha256(key(7).as_bytes()),
        candidate_fingerprint: "b".repeat(64),
        expected_generation: 0,
        identity: NativeIdentity {
            authority: "native".into(),
            subject: "private-subject".into(),
            tenant: None,
        },
    };
    enrollment_fences_identity_binding(store, &input).await;
    // Concurrent retries must produce one durable intent and one identity bind.
    let (a, b) = tokio::join!(
        store.prepare_provider_grant("owner", "login", &input),
        store.prepare_provider_grant("owner", "login", &input),
    );
    let a = a.unwrap();
    assert_eq!(a, b.unwrap());
    let Outcome::Applied(intent) = a else {
        panic!("prepare failed")
    };
    assert_eq!(intent.registry_revision, input.expected_revision + 1);
    assert!(
        !serde_json::to_string(&intent)
            .unwrap()
            .contains("private-subject")
    );
    let mut replacement = input.clone();
    replacement.candidate_fingerprint = "c".repeat(64);
    assert_eq!(
        store
            .prepare_provider_grant("owner", "login", &replacement)
            .await
            .unwrap(),
        Outcome::Rejected("operation_reused")
    );
    assert_eq!(
        store
            .finish_provider_grant("other-owner", &intent, 1)
            .await
            .unwrap(),
        Outcome::Rejected("grant_not_found")
    );
    assert_eq!(
        store
            .finish_provider_grant("owner", &intent, 2)
            .await
            .unwrap(),
        Outcome::Rejected("grant_receipt_mismatch")
    );
    let mut forged = intent.clone();
    forged.recipient_fingerprint = "d".repeat(64);
    assert_eq!(
        store
            .finish_provider_grant("owner", &forged, 1)
            .await
            .unwrap(),
        Outcome::Rejected("grant_receipt_mismatch")
    );
    let (a, b) = tokio::join!(
        store.finish_provider_grant("owner", &intent, 1),
        store.finish_provider_grant("owner", &intent, 1),
    );
    let committed = a.unwrap();
    assert_eq!(committed, b.unwrap());
    assert!(
        matches!(&committed, Outcome::Applied(receipt) if receipt.phase == Phase::Committed { generation: 1 })
    );
    let second = disable_enable_fences_login(store, &input, &intent, committed).await;
    cancellation_races_commit(store, second).await;
}

async fn disable_enable_fences_login(
    store: &Store,
    input: &Prepare,
    intent: &Receipt,
    committed: Outcome,
) -> Prepare {
    // A second Machine has an independent recipient/candidate and generation.
    let mut second = input.clone();
    second.expected_revision = intent.registry_revision;
    second.machine_id = "machine-b".into();
    enroll(store, "machine-b", &key(8)).await;
    second.recipient_fingerprint = crate::admin::hex_sha256(key(8).as_bytes());
    second.candidate_fingerprint = "f".repeat(64);
    let Outcome::Applied(pending) = store
        .prepare_provider_grant("owner", "login-b", &second)
        .await
        .unwrap()
    else {
        panic!("second Machine")
    };
    assert_eq!(pending.registry_revision, intent.registry_revision);
    // Disable then enable is an ABA change: equality of final enabled flags
    // must not revive the in-flight authorization.
    for (operation, enabled) in [("disable", false), ("enable", true)] {
        let current = store.provider_accounts("owner").await.unwrap();
        assert!(matches!(
            store
                .mutate_provider_accounts(
                    "owner",
                    operation,
                    &Mutation {
                        expected_revision: current.revision,
                        change: Change::SetEnabled {
                            id: input.account_id.clone(),
                            enabled
                        },
                    }
                )
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
    }
    let cancelled = store
        .finish_provider_grant("owner", &pending, 1)
        .await
        .unwrap();
    assert!(matches!(&cancelled, Outcome::Applied(receipt) if receipt.phase == Phase::Cancelled));
    assert_eq!(
        store
            .finish_provider_grant("owner", &pending, 1)
            .await
            .unwrap(),
        cancelled
    );
    // Replays report history only, even after metadata has changed.
    assert_eq!(
        store
            .prepare_provider_grant("owner", "login", input)
            .await
            .unwrap(),
        committed
    );
    assert_eq!(
        store
            .prepare_provider_grant("owner", "login-b", &second)
            .await
            .unwrap(),
        cancelled
    );
    second
}

async fn cancellation_races_commit(store: &Store, mut second: Prepare) {
    second.expected_revision = store.provider_accounts("owner").await.unwrap().revision;
    let Outcome::Applied(racing) = store
        .prepare_provider_grant("owner", "race", &second)
        .await
        .unwrap()
    else {
        panic!("prepare race")
    };
    let (cancel, commit) = tokio::join!(
        store.cancel_provider_grant("owner", &racing),
        store.finish_provider_grant("owner", &racing, 1),
    );
    let terminal = cancel.unwrap();
    assert_eq!(terminal, commit.unwrap());
    assert!(matches!(terminal, Outcome::Applied(receipt) if receipt.phase != Phase::Prepared));
    let Outcome::Applied(cancel_only) = store
        .prepare_provider_grant("owner", "cancel-only", &second)
        .await
        .unwrap()
    else {
        panic!("prepare cancel")
    };
    let cancelled = store
        .cancel_provider_grant("owner", &cancel_only)
        .await
        .unwrap();
    assert!(matches!(&cancelled, Outcome::Applied(receipt) if receipt.phase == Phase::Cancelled));
    assert_eq!(
        store
            .finish_provider_grant("owner", &cancel_only, 1)
            .await
            .unwrap(),
        cancelled
    );
}

#[tokio::test]
async fn sqlite_grant_journal_contract() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("grants.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    contract(&store).await;
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    assert_eq!(store.provider_accounts("owner").await.unwrap().revision, 4);
    for (operation, phase) in [
        ("login", Phase::Committed { generation: 1 }),
        ("login-b", Phase::Cancelled),
    ] {
        let receipt = store
            .provider_grant_receipt("owner", operation)
            .await
            .unwrap()
            .expect("durable receipt");
        assert_eq!(receipt.phase, phase);
        assert!(
            store
                .provider_grant_receipt("other-owner", operation)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_grant_journal_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    contract(&store).await;
}

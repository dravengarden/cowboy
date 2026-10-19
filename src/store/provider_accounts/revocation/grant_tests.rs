use crate::store::provider_accounts::{Change, Mutation, MutationResult, grants};
use crate::store::{StorageBackend, Store};

const OWNER: &str = "grant-revocation-owner";

#[tokio::test]
async fn local_wipe_waits_for_persisted_login_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    cleanup_admission_contract(&store, true).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_local_wipe_login_cleanup_admission() {
    let dir = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    cleanup_admission_contract(&store, true).await;
}

#[tokio::test]
async fn local_wipe_rejects_leader_only_login_exit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    cleanup_admission_contract(&store, false).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_local_wipe_rejects_leader_only_login_exit() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(
        &std::env::var("COWBOY_TEST_POSTGRES_URL").unwrap(),
        dir.path().join("artifacts"),
    )
    .await
    .unwrap();
    cleanup_admission_contract(&store, false).await;
}

async fn cleanup_admission_contract(store: &Store, process_tree_exited: bool) {
    use super::{LocalWipe, login_stops::Evidence};
    use crate::provider_account::{LoginStopObservation, LoginStopRequest};
    use crate::store::provider_accounts::logins::{DispatchScope, Request};
    use base64::Engine as _;
    store.migrate().await.unwrap();
    let grants = fixture(store).await;
    let account = grants[0].1.account_id.clone();
    let key = base64::engine::general_purpose::STANDARD.encode([23; 32]);
    let enrollment = store
        .create_machine_enrollment("login-machine", "Fixture", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
        .await
        .unwrap();
    let fingerprint = crate::admin::hex_sha256(key.as_bytes());
    let claim = store
        .claim_provider_login(
            OWNER,
            "login",
            &Request {
                account_id: account.clone(),
                machine_id: "login-machine".into(),
                provider_version: "fixture".into(),
                generation_digest: "digest".into(),
                method: "device".into(),
            },
        )
        .await
        .unwrap();
    store
        .with_provider_login_dispatch(
            OWNER,
            "login",
            &claim,
            &DispatchScope {
                registry_revision: 1,
                recipient_fingerprint: fingerprint.clone(),
            },
            || Ok(()),
        )
        .await
        .unwrap();
    store
        .mutate_provider_accounts(
            OWNER,
            "revoke",
            &Mutation {
                expected_revision: 1,
                change: Change::BeginRevocation {
                    id: account.clone(),
                },
            },
        )
        .await
        .unwrap();
    assert_cleanup_blocked(store).await;
    let evidence = Evidence {
        login_operation: "login".into(),
        machine_id: "login-machine".into(),
        recipient_fingerprint: fingerprint,
        observation: LoginStopObservation {
            request: LoginStopRequest {
                account_id: account,
                provider_id: "codex".into(),
                login_request_id: claim.receipt.request_id,
            },
            process_started: true,
            leader_exited: true,
            candidate_removed: true,
            process_tree_exited,
        },
    };
    store
        .record_provider_login_stop(OWNER, "revoke", &evidence, || true)
        .await
        .unwrap();
    if !process_tree_exited {
        assert_cleanup_blocked(store).await;
        return;
    }
    let outcome = store
        .with_provider_local_wipe(OWNER, "revoke", "committed", |_| {
            Ok(LocalWipe::Wiped { generation: 2 })
        })
        .await
        .unwrap();
    assert_eq!(outcome, LocalWipe::Wiped { generation: 2 });
    assert_eq!(
        store
            .with_provider_local_wipe(OWNER, "revoke", "committed", |_| panic!(
                "do not repeat deletion"
            ))
            .await
            .unwrap(),
        outcome
    );
}

async fn assert_cleanup_blocked(store: &Store) {
    assert!(
        store
            .with_provider_local_wipe(OWNER, "revoke", "committed", |_| panic!(
                "unconfirmed login must prevent vault mutation"
            ))
            .await
            .is_err()
    );
    assert!(
        store
            .with_provider_grant_revocation(
                OWNER,
                "revoke",
                "committed",
                |_| -> anyhow::Result<()> {
                    panic!("generic path cannot bypass cleanup admission")
                }
            )
            .await
            .is_err()
    );
    assert!(
        store
            .provider_account_receipt(OWNER, "revoke")
            .await
            .unwrap()
            .unwrap()
            .local_wipes
            .is_empty()
    );
}

#[tokio::test]
async fn local_wipe_journal_validates_phase_and_survives_restart() {
    use super::LocalWipe;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("db").display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let fixtures = fixture(&store).await;
    store
        .mutate_provider_accounts(
            OWNER,
            "revoke",
            &Mutation {
                expected_revision: 1,
                change: Change::BeginRevocation {
                    id: fixtures[0].1.account_id.clone(),
                },
            },
        )
        .await
        .unwrap();
    for (grant, evidence) in [
        ("committed", LocalWipe::CancelledDiscarded),
        ("committed", LocalWipe::Wiped { generation: 1 }),
        ("cancelled", LocalWipe::Wiped { generation: 2 }),
    ] {
        assert!(
            store
                .with_provider_local_wipe(OWNER, "revoke", grant, |_| Ok(evidence))
                .await
                .is_err()
        );
    }
    assert!(
        store
            .with_provider_local_wipe("other-owner", "revoke", "committed", |_| panic!(
                "other owner cannot clean up"
            ))
            .await
            .is_err()
    );
    assert!(
        store
            .with_provider_local_wipe(OWNER, "revoke", "committed", |_| anyhow::bail!(
                "cleanup unconfirmed"
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
            .local_wipes
            .is_empty()
    );
    let (committed, cancelled) = tokio::join!(
        store.with_provider_local_wipe(OWNER, "revoke", "committed", |_| Ok(LocalWipe::Wiped {
            generation: 2
        })),
        store.with_provider_local_wipe(OWNER, "revoke", "cancelled", |_| Ok(
            LocalWipe::CancelledDiscarded
        ))
    );
    committed.unwrap();
    cancelled.unwrap();
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let receipt = store
        .provider_account_receipt(OWNER, "revoke")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.local_wipes.len(), 2);
    assert_eq!(
        receipt.local_wipes["committed"],
        LocalWipe::Wiped { generation: 2 }
    );
    assert_eq!(
        receipt.local_wipes["cancelled"],
        LocalWipe::CancelledDiscarded
    );
    assert_eq!(
        store
            .with_provider_local_wipe(OWNER, "revoke", "committed", |_| panic!(
                "recorded cleanup must not run again"
            ))
            .await
            .unwrap(),
        LocalWipe::Wiped { generation: 2 }
    );
}

pub(super) async fn fixture(store: &Store) -> Vec<(&'static str, grants::Receipt)> {
    let MutationResult::Applied(created) = store
        .mutate_provider_accounts(
            OWNER,
            "create",
            &Mutation {
                expected_revision: 0,
                change: Change::Create {
                    provider_id: "codex".into(),
                    label: "Fixture".into(),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("create")
    };
    let account = created.account_id;
    let base = grants::Receipt {
        operation_id: "pending".into(),
        account_id: account.clone(),
        provider_id: "codex".into(),
        machine_id: "offline-machine".into(),
        recipient_fingerprint: "recipient".into(),
        candidate_fingerprint: "candidate".into(),
        identity_fingerprint: "identity".into(),
        expected_generation: 0,
        registry_revision: 1,
        phase: grants::Phase::Prepared,
    };
    let mut fixtures = vec![(OWNER, base.clone())];
    for (operation, phase) in [
        ("committed", grants::Phase::Committed { generation: 1 }),
        ("cancelled", grants::Phase::Cancelled),
    ] {
        fixtures.push((
            OWNER,
            grants::Receipt {
                operation_id: operation.into(),
                phase,
                ..base.clone()
            },
        ));
    }
    fixtures.push(("another-owner", base.clone()));
    fixtures.push((
        OWNER,
        grants::Receipt {
            operation_id: "another-account".into(),
            account_id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            ..base.clone()
        },
    ));
    for (owner, receipt) in &fixtures {
        let value = serde_json::to_string(receipt).unwrap();
        macro_rules! insert { ($db:expr) => {
            sqlx::query("INSERT INTO provider_account_grants (owner_user_id,operation_id,request_hash,value) VALUES ($1,$2,'fixture',$3)")
                .bind(owner).bind(&receipt.operation_id).bind(&value).execute(&$db.pool).await.unwrap()
        }; }
        match &store.backend {
            StorageBackend::Postgres(db) => {
                insert!(db);
            }
            StorageBackend::Sqlite(db) => {
                insert!(db);
            }
        }
    }
    fixtures
}

pub(crate) async fn contract(store: &Store) {
    let fixtures = fixture(store).await;
    let base = fixtures[0].1.clone();
    let mut input = Mutation {
        expected_revision: 0,
        change: Change::BeginRevocation {
            id: base.account_id.clone(),
        },
    };
    assert!(matches!(
        store
            .mutate_provider_accounts(OWNER, "revoke", &input)
            .await
            .unwrap(),
        MutationResult::Conflict(_)
    ));
    assert_eq!(
        store
            .provider_grant_receipt(OWNER, "pending")
            .await
            .unwrap(),
        Some(base.clone())
    );
    input.expected_revision = 1;
    let (first, retry) = tokio::join!(
        store.mutate_provider_accounts(OWNER, "revoke", &input),
        store.mutate_provider_accounts(OWNER, "revoke", &input)
    );
    assert_eq!(first.as_ref().unwrap(), retry.as_ref().unwrap());
    let MutationResult::Applied(receipt) = first.unwrap() else {
        panic!("revocation")
    };
    assert_eq!(receipt.revocation.unwrap().grants.len(), 3);
    for (owner, grant) in [("another-owner", "pending"), (OWNER, "another-account")] {
        assert!(
            store
                .with_provider_grant_revocation(owner, "revoke", grant, |_| -> anyhow::Result<()> {
                    panic!("out-of-scope cleanup")
                })
                .await
                .is_err()
        );
    }
    assert_eq!(
        store
            .with_provider_grant_revocation(OWNER, "revoke", "pending", |grant| Ok(grant
                .phase
                .clone()))
            .await
            .unwrap(),
        grants::Phase::Cancelled
    );
    assert_eq!(
        store
            .with_provider_grant_revocation(OWNER, "revoke", "committed", |grant| Ok(grant
                .phase
                .clone()))
            .await
            .unwrap(),
        grants::Phase::Committed { generation: 1 }
    );
    for (owner, mut expected) in fixtures {
        if owner == OWNER && expected.operation_id == "pending" {
            expected.phase = grants::Phase::Cancelled;
        }
        assert_eq!(
            store
                .provider_grant_receipt(owner, &expected.operation_id)
                .await
                .unwrap(),
            Some(expected)
        );
    }
    // A delayed cancellation observes the original terminal outcome.
    assert!(matches!(
        store.cancel_provider_grant(OWNER, &base).await.unwrap(),
        grants::Outcome::Applied(grants::Receipt {
            phase: grants::Phase::Cancelled,
            ..
        })
    ));
    discovery_contract(store, &base.account_id).await;
}

async fn discovery_contract(store: &Store, account: &crate::provider_account::ProviderAccountId) {
    let registry = store.provider_accounts(OWNER).await.unwrap();
    for index in 0..35 {
        assert!(matches!(
            store
                .mutate_provider_accounts(
                    OWNER,
                    &format!("resume-{index:02}"),
                    &Mutation {
                        expected_revision: registry.revision,
                        change: Change::BeginRevocation {
                            id: account.clone()
                        },
                    }
                )
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
    }
    let mut cursor = None;
    let mut found = std::collections::BTreeSet::new();
    let mut pages = 0;
    loop {
        let page = store
            .provider_account_revocations(cursor.as_ref())
            .await
            .unwrap();
        pages += 1;
        assert!(pages < 10, "recovery cursor did not advance");
        assert!(page.operations.len() <= 32);
        for operation in page.operations {
            assert_ne!(operation.operation, "create");
            if operation.owner == OWNER {
                assert!(found.insert(operation.operation));
            }
        }
        if page.next.is_none() {
            break;
        }
        assert_ne!(page.next, cursor);
        cursor = page.next;
    }
    assert!(pages > 1);
    assert_eq!(found.len(), 36);
    assert!(found.contains("revoke") && found.contains("resume-34"));
}

pub(crate) async fn persisted(store: &Store) {
    assert!(
        store
            .provider_account_revocations(None)
            .await
            .unwrap()
            .operations
            .iter()
            .any(|operation| operation.owner == OWNER)
    );
    assert_eq!(
        store
            .provider_grant_receipt(OWNER, "pending")
            .await
            .unwrap()
            .unwrap()
            .phase,
        grants::Phase::Cancelled
    );
    assert_eq!(
        store
            .provider_grant_receipt(OWNER, "committed")
            .await
            .unwrap()
            .unwrap()
            .phase,
        grants::Phase::Committed { generation: 1 }
    );
}

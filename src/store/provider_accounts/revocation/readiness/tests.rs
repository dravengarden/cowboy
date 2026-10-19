use super::*;
use crate::store::provider_accounts::{Change, Mutation};

#[tokio::test]
async fn pre_dispatch_refusal_needs_no_machine_exit_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("completion.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    refusal_contract(&store).await;
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    assert!(
        store
            .complete_provider_revocation("owner", "revoke")
            .await
            .unwrap()
    );
    assert!(store.provider_accounts("owner").await.unwrap().accounts[0].revoking);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_pre_dispatch_refusal_completion_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    refusal_contract(&store).await;
}

async fn refusal_contract(store: &Store) {
    use crate::store::provider_accounts::{
        MutationResult,
        logins::{DispatchRejection, Request},
    };
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
    let claim = store
        .claim_provider_login(
            "owner",
            "login",
            &Request {
                account_id: created.account_id.clone(),
                machine_id: "offline".into(),
                provider_version: "fixture".into(),
                generation_digest: "digest".into(),
                method: "native".into(),
            },
        )
        .await
        .unwrap();
    assert!(claim.receipt.native_generation.is_some());
    store
        .mutate_provider_accounts(
            "owner",
            "revoke",
            &Mutation {
                expected_revision: 1,
                change: Change::BeginRevocation {
                    id: created.account_id.clone(),
                },
            },
        )
        .await
        .unwrap();
    // Absence of a dispatch stamp alone is unknown, including after cancellation.
    assert!(
        !store
            .complete_provider_revocation("owner", "revoke")
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .provider_revocation_readiness("owner", "revoke")
            .await
            .unwrap()
            .missing
            .len(),
        2
    );
    store
        .reject_provider_login("owner", "login", &claim, DispatchRejection::CleanupRequired)
        .await
        .unwrap();
    let before = store
        .provider_account_receipt("owner", "revoke")
        .await
        .unwrap();
    let report = store
        .provider_revocation_readiness("owner", "revoke")
        .await
        .unwrap();
    assert!(report.missing.is_empty());
    assert!(!report.account_completed);
    assert_eq!(
        before,
        store
            .provider_account_receipt("owner", "revoke")
            .await
            .unwrap()
    );
    assert_completion(store, &created, before.unwrap()).await;
}

async fn assert_completion(store: &Store, created: &Receipt, before: Receipt) {
    use crate::store::provider_accounts::MutationResult;
    let (left, right) = tokio::join!(
        store.complete_provider_revocation("owner", "revoke"),
        store.complete_provider_revocation("owner", "revoke")
    );
    assert!(left.unwrap());
    assert!(right.unwrap());
    let completed = store
        .provider_account_receipt("owner", "revoke")
        .await
        .unwrap()
        .unwrap();
    assert!(completed.revocation_completed);
    assert_eq!(completed.registry, before.registry);
    assert!(
        store
            .provider_revocation_readiness("owner", "revoke")
            .await
            .unwrap()
            .account_completed
    );
    assert!(
        store
            .provider_account_revocations(None)
            .await
            .unwrap()
            .operations
            .is_empty()
    );
    assert!(
        store
            .complete_provider_revocation("other-owner", "revoke")
            .await
            .is_err()
    );
    assert!(
        store
            .complete_provider_revocation("owner", "create")
            .await
            .is_err()
    );
    let registry = store.provider_accounts("owner").await.unwrap();
    assert!(!registry.accounts[0].available());
    assert!(matches!(
        store
            .mutate_provider_accounts(
                "owner",
                "reenable",
                &Mutation {
                    expected_revision: registry.revision,
                    change: Change::SetEnabled {
                        id: created.account_id.clone(),
                        enabled: true
                    },
                }
            )
            .await
            .unwrap(),
        MutationResult::Invalid("account_revoking")
    ));
}

#[tokio::test]
async fn completion_inventory_is_scoped_read_only_and_distinguishes_local_remote_wipes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let fixtures = super::super::grant_tests::fixture(&store).await;
    let owner = "grant-revocation-owner";
    store
        .mutate_provider_accounts(
            owner,
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
    let before = store
        .provider_account_receipt(owner, "revoke")
        .await
        .unwrap()
        .unwrap();
    let report = store
        .provider_revocation_readiness(owner, "revoke")
        .await
        .unwrap();
    assert!(!report.account_completed);
    assert_eq!(report.completion_admission, "internal_only");
    assert!(
        !store
            .complete_provider_revocation(owner, "revoke")
            .await
            .unwrap()
    );
    assert_eq!(
        report
            .missing
            .iter()
            .filter(|m| m.kind == "local_grant_wipe")
            .count(),
        3
    );
    assert_eq!(
        report
            .missing
            .iter()
            .filter(|m| m.kind == "remote_grant_wipe")
            .count(),
        1
    );
    assert_eq!(
        before,
        store
            .provider_account_receipt(owner, "revoke")
            .await
            .unwrap()
            .unwrap()
    );
    assert!(
        store
            .provider_revocation_readiness("other", "revoke")
            .await
            .is_err()
    );
    assert!(
        store
            .provider_revocation_readiness(owner, "create")
            .await
            .is_err()
    );
    assert_partial_wipe(&store, owner, &fixtures).await;
}

async fn assert_partial_wipe(store: &Store, owner: &str, fixtures: &[(&str, grants::Receipt)]) {
    store
        .with_provider_local_wipe(owner, "revoke", "committed", |_| {
            Ok(LocalWipe::Wiped { generation: 2 })
        })
        .await
        .unwrap();
    let report = store
        .provider_revocation_readiness(owner, "revoke")
        .await
        .unwrap();
    assert_eq!(
        report
            .missing
            .iter()
            .filter(|m| m.kind == "local_grant_wipe")
            .count(),
        2
    );
    assert!(
        report
            .missing
            .iter()
            .any(|m| m.kind == "remote_grant_wipe" && m.target == "committed")
    );
    assert!(
        !store
            .complete_provider_revocation(owner, "revoke")
            .await
            .unwrap()
    );
    let receipt = store
        .provider_account_receipt(owner, "revoke")
        .await
        .unwrap()
        .unwrap();
    assert_remote_scope(receipt, fixtures);
}

fn assert_remote_scope(mut receipt: Receipt, fixtures: &[(&str, grants::Receipt)]) {
    let grant = &fixtures
        .iter()
        .find(|(_, g)| g.operation_id == "committed")
        .unwrap()
        .1;
    receipt.remote_wipes.insert(
        "committed".into(),
        super::super::remote_wipes::Evidence {
            grant_operation: "committed".into(),
            machine_id: grant.machine_id.clone(),
            recipient_fingerprint: grant.recipient_fingerprint.clone(),
            generation: 2,
        },
    );
    assert!(grant_missing(&receipt, grant).is_empty());
    receipt
        .remote_wipes
        .get_mut("committed")
        .unwrap()
        .recipient_fingerprint = "other enrollment".into();
    assert_eq!(
        grant_missing(&receipt, grant),
        vec![Missing {
            kind: "remote_grant_wipe",
            target: "committed".into()
        }]
    );
}

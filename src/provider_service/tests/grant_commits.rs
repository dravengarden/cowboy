use super::*;
use crate::provider_account::NativeIdentity;
use crate::store::Store;
use crate::store::provider_accounts::grants::{Outcome, Phase, Prepare, Receipt};
use crate::store::provider_accounts::{Change, Mutation, MutationResult};

mod admission;
mod promotion;
mod revocation;

async fn prepare(store: &Store, key: &str) -> Receipt {
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
    let Outcome::Applied(intent) = store
        .prepare_provider_grant(
            "owner",
            "login",
            &Prepare {
                expected_revision: created.registry.revision,
                account_id: created.account_id,
                provider_id: "grok".into(),
                machine_id: "machine-a".into(),
                recipient_fingerprint: crate::admin::hex_sha256(key.as_bytes()),
                candidate_fingerprint: candidate_hash(&package("grok"), &bundle(&package("grok"))),
                expected_generation: 0,
                identity: NativeIdentity {
                    authority: "native".into(),
                    subject: "fixture".into(),
                    tenant: None,
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("prepare")
    };
    intent
}

fn candidate_hash(package: &ProviderPackage, credentials: &PortableCredentialBundle) -> String {
    let identity = NativeIdentity {
        authority: "native".into(),
        subject: "fixture".into(),
        tenant: None,
    };
    crate::provider_service::grants::candidate_fingerprint(
        package,
        credentials,
        &identity.fingerprint(&package.manifest.id).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn crash_after_vault_commit_recovers_exact_db_intent() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("db").display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let key = base64::engine::general_purpose::STANDARD
        .encode(PublicKey::from(&StaticSecret::from([7; 32])).as_bytes());
    admission::enroll(&store, &key, "ssh-ed25519 QUJD").await;
    let intent = prepare(&store, &key).await;
    let service = ProviderAuthService::open(dir.path()).unwrap();
    let grant = service
        .account(&intent.account_id)
        .unwrap()
        .machine_grant(&key)
        .unwrap();
    let package = package("grok");
    let credentials = bundle(&package);
    assert!(grant.recover_grant(&intent).unwrap().is_none());
    let proof = grant.commit_grant(&package, &credentials, &intent).unwrap();
    assert_pending_grant_is_not_runtime_authority(&grant, &package, &credentials, &key);
    assert_eq!(proof.generation_for(&intent).unwrap(), 1);
    assert_eq!(
        grant
            .commit_grant(&package, &credentials, &intent)
            .unwrap()
            .generation_for(&intent)
            .unwrap(),
        1
    );
    let mut changed = credentials.clone();
    changed.values.insert(
        "auth_json".into(),
        base64::engine::general_purpose::STANDARD.encode(b"changed"),
    );
    assert!(grant.commit_grant(&package, &changed, &intent).is_err());
    let mut wrong = intent.clone();
    wrong.candidate_fingerprint = "b".repeat(64);
    assert!(proof.generation_for(&wrong).is_err());
    assert!(grant.recover_grant(&wrong).unwrap().is_none());
    assert!(grant.commit_grant(&package, &credentials, &wrong).is_err());
    assert!(
        service
            .commit_grant(&package, &credentials, &intent)
            .is_err()
    );
    // Drop every vault handle before DB confirmation, like a process crash.
    drop(grant);
    drop(service);
    let service = ProviderAuthService::open(dir.path()).unwrap();
    let grant = service
        .existing_machine_grant(&intent.account_id, &key)
        .unwrap();
    let recovered = grant.recover_grant(&intent).unwrap().unwrap();
    recovery_rejects_wrong_scope(&service, &store, &key).await;
    let recovery = service
        .reconcile_grant(&store, "owner", "login", "machine-a", &key)
        .await
        .unwrap();
    assert!(!recovery.cleanup_applied);
    let committed = Outcome::Applied(recovery.receipt);
    assert!(
        matches!(&committed, Outcome::Applied(receipt) if receipt.phase == Phase::Committed { generation: 1 })
    );
    assert_eq!(
        store
            .finish_verified_provider_grant("owner", &intent, &recovered)
            .await
            .unwrap(),
        committed
    );
    assert_eq!(grant.status("grok").unwrap().auth_generation, 1);
    // An ordinary later write supersedes the operation: recovery may not use a
    // stale sidecar or a coincidentally newer generation as its proof.
    grant.commit(&package, &credentials, None, Some(1)).unwrap();
    assert!(grant.recover_grant(&intent).unwrap().is_none());
    assert!(grant.commit_grant(&package, &credentials, &intent).is_err());
}

#[test]
fn promotion_commitment_is_authenticated_not_just_metadata() {
    let (root, service) = service();
    let id =
        ProviderAccountId::try_from("11111111-1111-4111-8111-111111111111".to_owned()).unwrap();
    let key = base64::engine::general_purpose::STANDARD
        .encode(PublicKey::from(&StaticSecret::from([7; 32])).as_bytes());
    let grant = service.account(&id).unwrap().machine_grant(&key).unwrap();
    let package = package("grok");
    grant
        .commit(&package, &bundle(&package), None, Some(0))
        .unwrap();
    let mut stored = grant.providers.read().get("grok").unwrap().clone();
    stored.promotion_commitment = Some("a".repeat(64));
    assert!(grant.open_vault(&stored).is_err());
    drop(grant);
    drop(service);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancelled_commit_cleanup_is_exact_and_cannot_erase_a_newer_login() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("db").display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let key = base64::engine::general_purpose::STANDARD
        .encode(PublicKey::from(&StaticSecret::from([7; 32])).as_bytes());
    admission::enroll(&store, &key, "ssh-ed25519 QUJD").await;
    let intent = prepare(&store, &key).await;
    let service = ProviderAuthService::open(dir.path()).unwrap();
    let grant = service
        .account(&intent.account_id)
        .unwrap()
        .machine_grant(&key)
        .unwrap();
    let package = package("grok");
    let credentials = bundle(&package);
    let proof = grant.commit_grant(&package, &credentials, &intent).unwrap();
    assert!(grant.discard_cancelled_grant(&intent).is_err());
    let Outcome::Applied(cancelled) = store.cancel_provider_grant("owner", &intent).await.unwrap()
    else {
        panic!("cancel")
    };
    assert_eq!(cancelled.phase, Phase::Cancelled);
    assert!(
        matches!(store.finish_verified_provider_grant("owner", &intent, &proof).await.unwrap(),
        Outcome::Applied(receipt) if receipt.phase == Phase::Cancelled)
    );
    assert!(
        service
            .reconcile_grant(&store, "owner", "login", "machine-a", &key)
            .await
            .unwrap()
            .cleanup_applied
    );
    assert!(
        !service
            .reconcile_grant(&store, "owner", "login", "machine-a", &key)
            .await
            .unwrap()
            .cleanup_applied
    );
    assert!(grant.discard_revoking_grant(&cancelled).unwrap());
    let status = grant.status("grok").unwrap();
    assert_eq!(status.auth_generation, 2);
    assert_eq!(
        status.authentication_state,
        ServiceAuthenticationState::SignedOut
    );
    assert_eq!(
        status.distribution_state,
        ServiceDistributionState::Revoking
    );
    assert!(grant.recover_grant(&intent).unwrap().is_none());
    assert_eq!(
        grant.seal_for_machine("grok", &key).unwrap().action,
        ProviderAuthAction::Wipe
    );
    grant.commit(&package, &credentials, None, Some(2)).unwrap();
    assert!(!grant.discard_cancelled_grant(&cancelled).unwrap());
    assert!(!grant.discard_revoking_grant(&cancelled).unwrap());
    assert_eq!(grant.status("grok").unwrap().auth_generation, 3);
    assert_eq!(
        grant.status("grok").unwrap().authentication_state,
        ServiceAuthenticationState::Ready
    );
}

fn assert_pending_grant_is_not_runtime_authority(
    grant: &ProviderAuthService,
    package: &ProviderPackage,
    credentials: &PortableCredentialBundle,
    key: &str,
) {
    assert!(grant.seal_for_machine("grok", key).is_err());
    assert!(
        grant
            .with_scheduling_generation("grok", true, Some(1), || panic!(
                "pending grant scheduled a session"
            ))
            .is_err()
    );
    assert!(
        grant
            .compare_and_swap_refresh(std::slice::from_ref(package), credentials, "grok", 1)
            .is_err()
    );
    assert_eq!(grant.status("grok").unwrap().auth_generation, 1);
}

async fn recovery_rejects_wrong_scope(service: &ProviderAuthService, store: &Store, key: &str) {
    assert!(
        service
            .reconcile_grant(store, "other-owner", "login", "machine-a", key)
            .await
            .is_err()
    );
    assert!(
        service
            .reconcile_grant(store, "owner", "login", "machine-b", key)
            .await
            .is_err()
    );
    let other_key = base64::engine::general_purpose::STANDARD
        .encode(PublicKey::from(&StaticSecret::from([8; 32])).as_bytes());
    assert!(
        service
            .reconcile_grant(store, "owner", "login", "machine-a", &other_key)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .provider_grant_receipt("owner", "login")
            .await
            .unwrap()
            .unwrap()
            .phase,
        Phase::Prepared
    );
}

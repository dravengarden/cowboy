use super::*;

pub(super) async fn fixture() -> (
    tempfile::TempDir,
    Store,
    ProviderAuthService,
    Arc<ProviderAuthService>,
    Receipt,
) {
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
    (dir, store, service, grant, intent)
}

#[tokio::test]
async fn candidate_substitution_is_rejected_before_the_first_vault_write() {
    let (_dir, store, _service, grant, intent) = fixture().await;
    let package = package("grok");
    let credentials = bundle(&package);
    let mut changed = credentials.clone();
    changed.values.insert(
        "auth_json".into(),
        base64::engine::general_purpose::STANDARD.encode(b"changed"),
    );
    assert!(grant.commit_grant(&package, &changed, &intent).is_err());
    assert!(
        grant
            .promote_prepared_grant(&store, "owner", &intent, &package, &changed)
            .await
            .is_err()
    );
    let mut other_release = package.clone();
    other_release.manifest.version = "99.0.0".into();
    assert!(
        grant
            .commit_grant(&other_release, &credentials, &intent)
            .is_err()
    );
    let mut other_identity = intent.clone();
    other_identity.identity_fingerprint = "d".repeat(64);
    assert!(
        grant
            .commit_grant(&package, &credentials, &other_identity)
            .is_err()
    );
    assert!(grant.status("grok").is_none());
    let original = store
        .provider_grant_receipt("owner", "login")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original, intent);
    grant
        .promote_prepared_grant(&store, "owner", &intent, &package, &credentials)
        .await
        .unwrap();
    assert!(
        grant
            .promote_prepared_grant(&store, "owner", &intent, &package, &changed)
            .await
            .is_err()
    );
    assert_eq!(grant.status("grok").unwrap().auth_generation, 1);
}

#[tokio::test]
async fn promotion_checks_db_intent_and_retries_never_overwrite_new_credentials() {
    let (_dir, store, _service, grant, intent) = fixture().await;
    let package = package("grok");
    let credentials = bundle(&package);
    assert!(
        grant
            .promote_prepared_grant(&store, "other-owner", &intent, &package, &credentials)
            .await
            .is_err()
    );
    let mut forged = intent.clone();
    forged.candidate_fingerprint = "b".repeat(64);
    assert!(
        grant
            .promote_prepared_grant(&store, "owner", &forged, &package, &credentials)
            .await
            .is_err()
    );
    assert!(grant.status("grok").is_none());
    let first = grant
        .promote_prepared_grant(&store, "owner", &intent, &package, &credentials)
        .await
        .unwrap();
    assert_eq!(first.receipt.phase, Phase::Committed { generation: 1 });
    assert!(!first.cleanup_applied);
    assert_eq!(
        grant
            .seal_current_grant(&store, "machine-a", "grok")
            .await
            .unwrap()
            .auth_generation,
        1
    );
    // A superseding write must not be replaced by a retry of old history.
    grant.commit(&package, &credentials, None, Some(1)).unwrap();
    let replay = grant
        .promote_prepared_grant(&store, "owner", &intent, &package, &credentials)
        .await
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(grant.status("grok").unwrap().auth_generation, 2);
    assert!(
        grant
            .seal_current_grant(&store, "machine-a", "grok")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_before_promotion_never_writes_credentials() {
    let (_dir, store, _service, grant, intent) = fixture().await;
    store.cancel_provider_grant("owner", &intent).await.unwrap();
    let package = package("grok");
    let result = grant
        .promote_prepared_grant(&store, "owner", &intent, &package, &bundle(&package))
        .await
        .unwrap();
    assert_eq!(result.receipt.phase, Phase::Cancelled);
    assert!(!result.cleanup_applied);
    assert!(grant.status("grok").is_none());
}

#[tokio::test]
async fn metadata_change_during_promotion_cleans_only_its_own_vault_write() {
    let (_dir, store, _service, grant, intent) = fixture().await;
    store
        .mutate_provider_accounts(
            "owner",
            "disable",
            &Mutation {
                expected_revision: intent.registry_revision,
                change: Change::SetEnabled {
                    id: intent.account_id.clone(),
                    enabled: false,
                },
            },
        )
        .await
        .unwrap();
    let package = package("grok");
    let credentials = bundle(&package);
    let result = grant
        .promote_prepared_grant(&store, "owner", &intent, &package, &credentials)
        .await
        .unwrap();
    assert_eq!(result.receipt.phase, Phase::Cancelled);
    assert!(result.cleanup_applied);
    assert_eq!(grant.status("grok").unwrap().auth_generation, 2);
    assert_eq!(
        grant.status("grok").unwrap().authentication_state,
        ServiceAuthenticationState::SignedOut
    );
    let replay = grant
        .promote_prepared_grant(&store, "owner", &intent, &package, &credentials)
        .await
        .unwrap();
    assert!(!replay.cleanup_applied);
    assert_eq!(grant.status("grok").unwrap().auth_generation, 2);
}

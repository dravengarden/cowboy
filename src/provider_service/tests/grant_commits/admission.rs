use super::*;

pub(super) async fn enroll(store: &Store, key: &str, signer: &str) {
    let token = store
        .create_machine_enrollment("machine-a", "Machine A", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&token, signer, key)
        .await
        .unwrap();
}

async fn contract(store: &Store, service: &ProviderAuthService) {
    let key = base64::engine::general_purpose::STANDARD
        .encode(PublicKey::from(&StaticSecret::from([7; 32])).as_bytes());
    enroll(store, &key, "ssh-ed25519 QUJD").await;
    let intent = prepare(store, &key).await;
    let grant = service
        .account(&intent.account_id)
        .unwrap()
        .machine_grant(&key)
        .unwrap();
    let package = package("grok");
    let credentials = bundle(&package);
    grant.commit_grant(&package, &credentials, &intent).unwrap();
    assert!(
        store
            .provider_grant_route(&intent.commitment().unwrap())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        grant
            .seal_current_grant(store, "machine-a", "grok")
            .await
            .is_err()
    );
    assert!(
        store
            .with_provider_grant_admission("owner", "login", "machine-a", |admission| grant
                .seal_admitted_grant(admission))
            .await
            .is_err()
    );
    service
        .reconcile_grant(store, "owner", "login", "machine-a", &key)
        .await
        .unwrap();
    let sealed = grant
        .seal_current_grant(store, "machine-a", "grok")
        .await
        .unwrap();
    assert_eq!(sealed.account_id, intent.account_id);
    assert_eq!(sealed.auth_generation, 1);
    assert_eq!(sealed.action, ProviderAuthAction::Apply);
    refresh_keeps_lineage(store, &grant, &package, &credentials).await;
    assert!(
        store
            .with_provider_grant_admission("other-owner", "login", "machine-a", |admission| grant
                .seal_admitted_grant(admission))
            .await
            .is_err()
    );
    assert!(
        store
            .with_provider_grant_admission("owner", "login", "other-machine", |admission| grant
                .seal_admitted_grant(admission))
            .await
            .is_err()
    );
    // A newer vault generation cannot be authorized by the old DB receipt.
    grant.commit(&package, &credentials, None, Some(2)).unwrap();
    assert!(
        grant
            .seal_current_grant(store, "machine-a", "grok")
            .await
            .is_err()
    );
    assert!(
        store
            .with_provider_grant_admission("owner", "login", "machine-a", |admission| grant
                .seal_admitted_grant(admission))
            .await
            .is_err()
    );
    replace_grant(store, service, &grant, &key, &intent).await;
    account_changes_fence_admission(store, &grant, &intent.account_id).await;
}

async fn replace_grant(
    store: &Store,
    service: &ProviderAuthService,
    grant: &ProviderAuthService,
    key: &str,
    intent: &Receipt,
) {
    let package = package("grok");
    let credentials = bundle(&package);
    let Outcome::Applied(next) = store
        .prepare_provider_grant(
            "owner",
            "next",
            &Prepare {
                expected_revision: intent.registry_revision,
                account_id: intent.account_id.clone(),
                provider_id: intent.provider_id.clone(),
                machine_id: intent.machine_id.clone(),
                recipient_fingerprint: intent.recipient_fingerprint.clone(),
                candidate_fingerprint: candidate_hash(&package, &credentials),
                expected_generation: 3,
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
        panic!("prepare next")
    };
    let promoted = grant
        .promote_prepared_grant(store, "owner", &next, &package, &credentials)
        .await
        .unwrap();
    assert_eq!(promoted.receipt.phase, Phase::Committed { generation: 4 });
    assert!(!promoted.cleanup_applied);
    service
        .reconcile_grant(store, "owner", "next", "machine-a", key)
        .await
        .unwrap();
    assert_eq!(
        grant
            .seal_current_grant(store, "machine-a", "grok")
            .await
            .unwrap()
            .auth_generation,
        4
    );
}

async fn refresh_keeps_lineage(
    store: &Store,
    grant: &ProviderAuthService,
    package: &ProviderPackage,
    original: &PortableCredentialBundle,
) {
    let identity = NativeIdentity {
        authority: "native".into(),
        subject: "fixture".into(),
        tenant: None,
    };
    let mut wrong = identity.clone();
    wrong.subject = "other-account".into();
    let mut first = original.clone();
    first.values.insert(
        "auth_json".into(),
        base64::engine::general_purpose::STANDARD.encode(br#"{"token":"first"}"#),
    );
    let mut second = original.clone();
    second.values.insert(
        "auth_json".into(),
        base64::engine::general_purpose::STANDARD.encode(br#"{"token":"second"}"#),
    );
    assert!(
        grant
            .refresh_current_grant(store, "machine-a", package, &first, 1, &wrong)
            .await
            .is_err()
    );
    assert!(
        grant
            .refresh_current_grant(store, "other-machine", package, &first, 1, &identity)
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        grant.refresh_current_grant(store, "machine-a", package, &first, 1, &identity),
        grant.refresh_current_grant(store, "machine-a", package, &second, 1, &identity),
    );
    let winner = match (a.unwrap(), b.unwrap()) {
        (
            ProviderAuthRefreshResult::Updated(_),
            ProviderAuthRefreshResult::Stale {
                current_generation: 2,
            },
        ) => first,
        (
            ProviderAuthRefreshResult::Stale {
                current_generation: 2,
            },
            ProviderAuthRefreshResult::Updated(_),
        ) => second,
        other => panic!("refresh must have one CAS winner: {other:?}"),
    };
    assert_eq!(
        grant
            .seal_current_grant(store, "machine-a", "grok")
            .await
            .unwrap()
            .auth_generation,
        2
    );
    assert!(matches!(
        grant
            .refresh_current_grant(store, "machine-a", package, &winner, 2, &identity)
            .await
            .unwrap(),
        ProviderAuthRefreshResult::Unchanged(_)
    ));
    assert!(
        grant
            .seal_for_machine("grok", grant.recipient_key.as_ref().unwrap())
            .is_err()
    );
}

async fn account_changes_fence_admission(
    store: &Store,
    grant: &ProviderAuthService,
    account: &ProviderAccountId,
) {
    for (operation, enabled) in [("disable", false), ("enable", true)] {
        let registry = store.provider_accounts("owner").await.unwrap();
        assert!(matches!(
            store
                .mutate_provider_accounts(
                    "owner",
                    operation,
                    &Mutation {
                        expected_revision: registry.revision,
                        change: Change::SetEnabled {
                            id: account.clone(),
                            enabled
                        },
                    }
                )
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        assert!(
            store
                .with_provider_grant_admission("owner", "next", "machine-a", |admission| grant
                    .seal_admitted_grant(admission))
                .await
                .is_err()
        );
    }
    store.revoke_machine("machine-a").await.unwrap();
    let error = store
        .with_provider_grant_admission("owner", "next", "machine-a", |admission| {
            grant.seal_admitted_grant(admission)
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Machine unavailable"));
    let new_key = base64::engine::general_purpose::STANDARD
        .encode(PublicKey::from(&StaticSecret::from([8; 32])).as_bytes());
    enroll(store, &new_key, "ssh-ed25519 REVG").await;
    let error = store
        .with_provider_grant_admission("owner", "next", "machine-a", |admission| {
            grant.seal_admitted_grant(admission)
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("not committed for this enrollment")
    );
}

#[tokio::test]
async fn sqlite_current_grant_admission() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}?mode=rwc", dir.path().join("db").display());
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let service = ProviderAuthService::open(dir.path()).unwrap();
    contract(&store, &service).await;
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let receipt = store
        .provider_grant_receipt("owner", "next")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store
            .provider_grant_route(&receipt.commitment().unwrap())
            .await
            .unwrap(),
        Some(("owner".into(), "next".into()))
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_current_grant_admission() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let service = ProviderAuthService::open(dir.path()).unwrap();
    contract(&store, &service).await;
}

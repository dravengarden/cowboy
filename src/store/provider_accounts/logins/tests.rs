use super::*;
use crate::store::provider_accounts::{Change, Mutation, MutationResult};
mod native_revision_tests;

#[tokio::test]
async fn login_discovery_is_owner_scoped_paged_and_never_falls_back_to_old_generations() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("discovery.sqlite").display()
    );
    login_discovery_contract(&url, &dir.path().join("artifacts")).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_login_discovery_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let dir = tempfile::tempdir().unwrap();
    login_discovery_contract(&url, &dir.path().join("artifacts")).await;
}

async fn login_discovery_contract(url: &str, artifacts: &std::path::Path) {
    let store = Store::connect(url, artifacts.to_path_buf()).await.unwrap();
    store.migrate().await.unwrap();
    let account = create_account(&store).await.account_id;
    let mut input = Request {
        account_id: account.clone(),
        machine_id: "a".into(),
        provider_version: "1".into(),
        generation_digest: "digest".into(),
        method: "native".into(),
    };
    let old = store
        .claim_provider_login("login-owner", "old", &input)
        .await
        .unwrap()
        .receipt;
    store
        .cancel_provider_login("login-owner", "old")
        .await
        .unwrap();
    let new = store
        .claim_provider_login("login-owner", "new", &input)
        .await
        .unwrap()
        .receipt;
    input.machine_id = "b".into();
    let other_machine = store
        .claim_provider_login("login-owner", "second", &input)
        .await
        .unwrap()
        .receipt;
    assert_ne!(new.native_generation, old.native_generation);
    drop(store);
    let store = Store::connect(url, artifacts.to_path_buf()).await.unwrap();
    let first = store
        .native_login_receipts("login-owner", &account, "", 1)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].request_id, new.request_id);
    let second = store
        .native_login_receipts("login-owner", &account, "a", 1)
        .await
        .unwrap();
    assert_eq!(second[0].request_id, other_machine.request_id);
    assert!(
        store
            .native_login_receipts("login-owner", &account, "b", 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .native_login_receipts("other-owner", &account, "", 20)
            .await
            .unwrap()
            .is_empty()
    );
    let other_account = uuid::Uuid::new_v4().to_string().parse().unwrap();
    assert!(
        store
            .native_login_receipts("login-owner", &other_account, "", 20)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .native_login_receipts("login-owner", &account, "", 0)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn native_logout_receipt_is_scoped_idempotent_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("logout.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let created = create_account(&store).await;
    let key = enroll_binding_machine(&store, "logout-machine").await;
    let input = Request {
        account_id: created.account_id.clone(),
        machine_id: "logout-machine".into(),
        provider_version: "fixture".into(),
        generation_digest: "digest".into(),
        method: "native".into(),
    };
    let claim = store
        .claim_provider_login("login-owner", "logout-login", &input)
        .await
        .unwrap();
    let fingerprint = crate::admin::hex_sha256(key.as_bytes());
    store
        .with_provider_login_dispatch(
            "login-owner",
            "logout-login",
            &claim,
            &DispatchScope {
                registry_revision: created.registry.revision,
                recipient_fingerprint: fingerprint.clone(),
            },
            || Ok(()),
        )
        .await
        .unwrap();
    let completion = native_logout::Completion {
        revocation_operation: "logout-revoke".into(),
        machine_id: input.machine_id.clone(),
        recipient_fingerprint: fingerprint,
        observation: crate::provider_account::NativeLogoutObservation {
            profile_cleaned: true,
            process_trees_exited: true,
            request: crate::provider_account::NativeLogoutRequest {
                account_id: input.account_id.clone(),
                provider_id: "claude-code".into(),
                generation: claim.receipt.native_generation.unwrap(),
                login_request_id: claim.receipt.request_id,
                operation_id: "logout-revoke".into(),
            },
            succeeded: Some(true),
        },
    };
    assert!(
        store
            .record_native_logout_completion("login-owner", "logout-login", &completion, || true)
            .await
            .is_err()
    );
    assert_native_logout_dispatch(&store, &completion, false).await;
    store
        .mutate_provider_accounts(
            "login-owner",
            "logout-revoke",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::BeginRevocation {
                    id: input.account_id,
                },
            },
        )
        .await
        .unwrap();
    assert_native_logout_dispatch(&store, &completion, true).await;
    admit_native_logout_fixture(&store, &completion).await;
    assert!(
        store
            .provider_revocation_readiness("login-owner", "logout-revoke")
            .await
            .unwrap()
            .missing
            .is_empty()
    );
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    assert_native_logout_replay(&store, completion).await;
}

async fn assert_native_logout_replay(store: &Store, completion: native_logout::Completion) {
    let replay = store
        .record_native_logout_completion("login-owner", "logout-login", &completion, || {
            panic!("historical replay cannot reacquire authority")
        })
        .await
        .unwrap();
    assert_eq!(replay.native_logout, Some(completion.clone()));
    let mut proposal = completion.clone();
    proposal.observation.succeeded = None;
    proposal.observation.profile_cleaned = false;
    proposal.observation.process_trees_exited = false;
    assert_eq!(
        store
            .with_native_logout_dispatch(
                "login-owner",
                "logout-login",
                &proposal,
                || -> Result<()> { panic!("recorded logout must not dispatch again") }
            )
            .await
            .unwrap(),
        None
    );
    let mut wrong = completion;
    wrong.observation.succeeded = Some(false);
    assert!(
        store
            .record_native_logout_completion("login-owner", "logout-login", &wrong, || true)
            .await
            .is_err()
    );
    assert!(
        store
            .provider_grant_receipt("login-owner", "logout-login")
            .await
            .unwrap()
            .is_none()
    );
}

async fn assert_native_logout_dispatch(
    store: &Store,
    completion: &native_logout::Completion,
    admitted: bool,
) {
    let mut proposal = completion.clone();
    proposal.observation.succeeded = None;
    proposal.observation.profile_cleaned = false;
    proposal.observation.process_trees_exited = false;
    let called = std::cell::Cell::new(false);
    let result = store
        .with_native_logout_dispatch("login-owner", "logout-login", &proposal, || {
            called.set(true);
            Ok(42)
        })
        .await;
    assert_eq!(called.get(), admitted);
    if admitted {
        assert_eq!(result.unwrap(), Some(42));
    } else {
        assert!(result.is_err());
    }
    let mut wrong = proposal;
    wrong.recipient_fingerprint = "wrong enrollment".into();
    assert!(
        store
            .with_native_logout_dispatch("login-owner", "logout-login", &wrong, || -> Result<()> {
                panic!("wrong enrollment must not dispatch")
            })
            .await
            .is_err()
    );
}

async fn admit_native_logout_fixture(store: &Store, completion: &native_logout::Completion) {
    let mut old = completion.clone();
    old.observation.profile_cleaned = false;
    store
        .record_native_logout_completion("login-owner", "logout-login", &old, || true)
        .await
        .unwrap();
    // Command success without cleanup evidence must still admit a scoped retry.
    assert_native_logout_dispatch(store, completion, true).await;
    assert!(
        store
            .record_native_logout_completion("other-owner", "logout-login", completion, || true)
            .await
            .is_err()
    );
    assert!(
        store
            .record_native_logout_completion("login-owner", "logout-login", completion, || false)
            .await
            .is_err()
    );
    let mut wrong = completion.clone();
    wrong.observation.request.generation += 1;
    assert!(
        store
            .record_native_logout_completion("login-owner", "logout-login", &wrong, || true)
            .await
            .is_err()
    );
    wrong = completion.clone();
    wrong.observation.succeeded = None;
    assert!(
        store
            .record_native_logout_completion("login-owner", "logout-login", &wrong, || true)
            .await
            .is_err()
    );
    let (first, second) = tokio::join!(
        store.record_native_logout_completion("login-owner", "logout-login", completion, || true),
        store.record_native_logout_completion("login-owner", "logout-login", completion, || true),
    );
    assert_eq!(first.unwrap(), second.unwrap());
    let mut changed = completion.clone();
    changed.observation.process_trees_exited = !completion.observation.process_trees_exited;
    assert!(
        store
            .record_native_logout_completion("login-owner", "logout-login", &changed, || true)
            .await
            .is_err()
    );
}

async fn fresh_dispatch_contract(
    store: &Store,
    claim: &Claim,
    input: &Request,
    revision: i64,
    key: &str,
) -> DispatchScope {
    let dispatch_scope = DispatchScope {
        registry_revision: revision,
        recipient_fingerprint: crate::admin::hex_sha256(key.as_bytes()),
    };
    let calls = std::cell::Cell::new(0);
    store
        .with_provider_login_dispatch("login-owner", "bind", claim, &dispatch_scope, || {
            calls.set(calls.get() + 1);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(calls.get(), 1);
    let persisted = store
        .provider_login_receipt("login-owner", "bind")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.dispatch_registry_revision, Some(revision));
    assert_eq!(
        persisted.dispatch_recipient_fingerprint.as_deref(),
        Some(dispatch_scope.recipient_fingerprint.as_str())
    );
    assert!(
        store
            .with_provider_login_dispatch(
                "login-owner",
                "bind",
                claim,
                &dispatch_scope,
                || -> Result<()> { panic!("the original claimant cannot dispatch twice") }
            )
            .await
            .is_err()
    );
    let replay = store
        .claim_provider_login("login-owner", "bind", input)
        .await
        .unwrap();
    assert!(
        store
            .with_provider_login_dispatch(
                "login-owner",
                "bind",
                &replay,
                &dispatch_scope,
                || -> Result<()> { panic!("historical claim cannot dispatch") }
            )
            .await
            .is_err()
    );
    let wrong_recipient = DispatchScope {
        recipient_fingerprint: "wrong".into(),
        ..dispatch_scope
    };
    assert!(
        store
            .with_provider_login_dispatch(
                "login-owner",
                "bind",
                claim,
                &wrong_recipient,
                || -> Result<()> { panic!("changed enrollment cannot dispatch") }
            )
            .await
            .is_err()
    );
    dispatch_scope
}

async fn completed_dispatch_refused(store: &Store, claim: &Claim, scope: &DispatchScope) {
    assert!(
        store
            .with_provider_login_dispatch("login-owner", "bind", claim, scope, || -> Result<()> {
                panic!("completed claim cannot dispatch")
            })
            .await
            .is_err()
    );
}

async fn enroll_binding_machine(store: &Store, machine: &str) -> String {
    use base64::Engine as _;
    let key = base64::engine::general_purpose::STANDARD.encode(
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([12; 32])).as_bytes(),
    );
    let enrollment = store
        .create_machine_enrollment(machine, "Native", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
        .await
        .unwrap();
    key
}

async fn native_binding_contract(store: &Store, registration: Option<bool>) {
    let created = create_account(store).await;
    let input = Request {
        account_id: created.account_id.clone(),
        machine_id: "native-binding".into(),
        provider_version: "1".into(),
        generation_digest: "digest".into(),
        method: "native".into(),
    };
    let key = enroll_binding_machine(store, &input.machine_id).await;
    let claim = store
        .claim_provider_login("login-owner", "bind", &input)
        .await
        .unwrap();
    let dispatch_scope =
        fresh_dispatch_contract(store, &claim, &input, created.registry.revision, &key).await;
    let completion = NativeCompletion {
        machine_id: input.machine_id.clone(),
        recipient_fingerprint: crate::admin::hex_sha256(key.as_bytes()),
        profile: crate::provider_account::NativeProfileReceipt {
            account_id: input.account_id.clone(),
            provider_id: "claude-code".into(),
            request_id: claim.receipt.request_id.clone(),
            generation: claim.receipt.native_generation.unwrap(),
            method: input.method.clone(),
            provider_version: input.provider_version.clone(),
            generation_digest: input.generation_digest.clone(),
            auth_contract_fingerprint: "contract".into(),
            identity: crate::provider_account::NativeIdentity {
                authority: "fixture".into(),
                subject: "personal".into(),
                tenant: None,
            },
        },
    };
    let revision = created.registry.revision;
    changed_enrollment_cannot_complete_old_dispatch(store, &completion, &key).await;
    assert!(
        store
            .bind_native_login_identity("login-owner", "bind", &completion, revision)
            .await
            .is_err()
    );
    store
        .record_native_login_completion("login-owner", "bind", &completion)
        .await
        .unwrap();
    completed_dispatch_refused(store, &claim, &dispatch_scope).await;
    assert!(
        store
            .bind_native_login_identity("another-owner", "bind", &completion, revision)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .bind_native_login_identity("login-owner", "bind", &completion, revision + 1)
            .await
            .unwrap_err()
            .to_string(),
        "login_admission_revision_changed"
    );
    assert_eq!(
        store.provider_accounts("login-owner").await.unwrap(),
        created.registry
    );
    let bound = concurrent_binding_recovery(store, &completion, revision).await;
    assert_eq!(bound.revision, revision + 1);
    assert_eq!(
        bound.accounts[0].identity_fingerprint,
        Some(
            completion
                .profile
                .identity
                .fingerprint("claude-code")
                .unwrap()
        )
    );
    assert!(
        store
            .provider_grant_receipt("login-owner", &claim.receipt.request_id)
            .await
            .unwrap()
            .is_none()
    );
    native_binding_admission_contract(store, &completion).await;
    native_selection_contract(store, &completion).await;
    if let Some(replacement) = registration {
        Box::pin(native_session_tests::contract(
            store,
            &completion,
            replacement,
        ))
        .await;
        return;
    }
    let bound = native_revision_tests::unrelated_changes(store, &completion).await;
    let bound = native_binding_disable_enable(store, &completion, bound).await;
    native_binding_rejections(store, input, completion, bound).await;
}

async fn native_selection_contract(store: &Store, completion: &NativeCompletion) {
    let id = &completion.profile.account_id;
    assert_eq!(
        store
            .native_login_for_session("login-owner", id, &completion.machine_id)
            .await
            .unwrap(),
        Some(("bind".into(), completion.clone()))
    );
    for (owner, machine) in [
        ("other-owner", completion.machine_id.as_str()),
        ("login-owner", "other-machine"),
    ] {
        assert!(
            store
                .native_login_for_session(owner, id, machine)
                .await
                .unwrap()
                .is_none()
        );
    }
    let other = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert!(
        store
            .native_login_for_session("login-owner", &other, &completion.machine_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .native_login_for_session(
                "login-owner",
                &ProviderAccountId::default(),
                &completion.machine_id
            )
            .await
            .is_err()
    );
}

async fn concurrent_binding_recovery(
    store: &Store,
    completion: &NativeCompletion,
    revision: i64,
) -> Registry {
    let recovered = store
        .provider_login_receipt("login-owner", "bind")
        .await
        .unwrap()
        .unwrap();
    let (a, b) = tokio::join!(
        store.bind_native_login_identity(
            "login-owner",
            "bind",
            recovered.native_completion.as_ref().unwrap(),
            recovered.dispatch_registry_revision.unwrap()
        ),
        store.bind_native_login_identity("login-owner", "bind", completion, revision)
    );
    let bound = a.unwrap();
    assert_eq!(bound, b.unwrap());
    bound
}

async fn changed_enrollment_cannot_complete_old_dispatch(
    store: &Store,
    completion: &NativeCompletion,
    original_key: &str,
) {
    async fn replace_key(store: &Store, machine: &str, key: &str) {
        macro_rules! update {
            ($db:expr) => {
                sqlx::query("UPDATE machines SET encryption_public_key=$2 WHERE id=$1")
                    .bind(machine)
                    .bind(key)
                    .execute(&$db.pool)
                    .await
                    .unwrap()
            };
        }
        match &store.backend {
            StorageBackend::Sqlite(db) => {
                update!(db);
            }
            StorageBackend::Postgres(db) => {
                update!(db);
            }
        }
    }
    let changed_key = "replacement-enrollment-fixture";
    replace_key(store, &completion.machine_id, changed_key).await;
    let mut changed = completion.clone();
    changed.recipient_fingerprint = crate::admin::hex_sha256(changed_key.as_bytes());
    let error = store
        .record_native_login_completion("login-owner", "bind", &changed)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("machine_enrollment_changed"));
    assert!(
        store
            .provider_login_receipt("login-owner", "bind")
            .await
            .unwrap()
            .unwrap()
            .native_completion
            .is_none()
    );
    replace_key(store, &completion.machine_id, original_key).await;
}

async fn native_binding_disable_enable(
    store: &Store,
    completion: &NativeCompletion,
    mut registry: Registry,
) -> Registry {
    let binding_revision = store
        .provider_login_receipt("login-owner", "bind")
        .await
        .unwrap()
        .unwrap()
        .native_binding_revision
        .unwrap();
    for (operation, enabled) in [("disable", false), ("enable", true)] {
        let MutationResult::Applied(changed) = store
            .mutate_provider_accounts(
                "login-owner",
                operation,
                &Mutation {
                    expected_revision: registry.revision,
                    change: Change::SetEnabled {
                        id: completion.profile.account_id.clone(),
                        enabled,
                    },
                },
            )
            .await
            .unwrap()
        else {
            panic!("change account enablement")
        };
        registry = changed.registry;
        assert!(
            store
                .with_native_login_binding("login-owner", "bind", completion, || -> Result<()> {
                    panic!("disabled or renewed accounts must not run the callback")
                })
                .await
                .is_err()
        );
        // Even supplying the new revision cannot renew a previously bound operation.
        assert!(
            store
                .bind_native_login_identity("login-owner", "bind", completion, registry.revision)
                .await
                .is_err()
        );
    }
    let historical = store
        .record_native_login_completion("login-owner", "bind", completion)
        .await
        .unwrap();
    assert_eq!(historical.native_binding_revision, Some(binding_revision));
    assert_eq!(
        store.provider_accounts("login-owner").await.unwrap(),
        registry
    );
    registry
}

async fn native_binding_admission_contract(store: &Store, completion: &NativeCompletion) {
    assert_eq!(
        store
            .with_native_login_binding("login-owner", "bind", completion, || Ok(7))
            .await
            .unwrap(),
        7
    );
    for (owner, operation) in [("other-owner", "bind"), ("login-owner", "missing")] {
        assert!(
            store
                .with_native_login_binding(owner, operation, completion, || -> Result<()> {
                    panic!("wrong scope must not reach the callback")
                })
                .await
                .is_err()
        );
    }
    for field in 0..4 {
        let mut wrong = completion.clone();
        match field {
            0 => wrong.machine_id = "other".into(),
            1 => wrong.profile.generation += 1,
            2 => wrong.recipient_fingerprint = "other".into(),
            _ => wrong.profile.identity.subject = "other".into(),
        }
        assert!(
            store
                .with_native_login_binding("login-owner", "bind", &wrong, || -> Result<()> {
                    panic!("wrong evidence must not reach the callback")
                })
                .await
                .is_err()
        );
    }
    assert!(
        store
            .with_native_login_binding("login-owner", "bind", completion, || -> Result<()> {
                anyhow::bail!("connection_changed")
            })
            .await
            .is_err()
    );
    // A refused live action does not consume or renew the binding.
    store
        .with_native_login_binding("login-owner", "bind", completion, || Ok(()))
        .await
        .unwrap();
}

async fn native_binding_rejections(
    store: &Store,
    input: Request,
    completion: NativeCompletion,
    bound: Registry,
) {
    let revision = bound.revision - 1;
    let replacement = store
        .claim_provider_login("login-owner", "replacement", &input)
        .await
        .unwrap();
    // A pending newer reservation must not fall back to the old successful one.
    assert!(
        store
            .native_login_for_session("login-owner", &input.account_id, &input.machine_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .bind_native_login_identity("login-owner", "bind", &completion, revision)
            .await
            .is_err()
    );
    let mut other = completion.clone();
    other.profile.request_id = replacement.receipt.request_id;
    other.profile.generation = replacement.receipt.native_generation.unwrap();
    other.profile.identity.subject = "another-person".into();
    store
        .record_native_login_completion("login-owner", "replacement", &other)
        .await
        .unwrap();
    assert!(
        store
            .bind_native_login_identity("login-owner", "replacement", &other, bound.revision)
            .await
            .is_err()
    );
    assert_eq!(store.provider_accounts("login-owner").await.unwrap(), bound);
    store
        .cancel_provider_login("login-owner", "replacement")
        .await
        .unwrap();
    assert!(
        store
            .bind_native_login_identity("login-owner", "replacement", &other, bound.revision)
            .await
            .is_err()
    );
    let fresh = store
        .claim_provider_login("login-owner", "revoked", &input)
        .await
        .unwrap();
    let mut revoked = completion;
    revoked.profile.request_id = fresh.receipt.request_id;
    revoked.profile.generation = fresh.receipt.native_generation.unwrap();
    store
        .record_native_login_completion("login-owner", "revoked", &revoked)
        .await
        .unwrap();
    store.revoke_machine(&input.machine_id).await.unwrap();
    assert!(
        store
            .bind_native_login_identity("login-owner", "revoked", &revoked, bound.revision)
            .await
            .is_err()
    );
    assert_eq!(store.provider_accounts("login-owner").await.unwrap(), bound);
}

#[tokio::test]
async fn sqlite_native_binding_checks_current_authority_without_granting_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("binding.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    native_binding_contract(&store, None).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_native_binding_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    native_binding_contract(&store, None).await;
}

mod native_session_tests;
mod transition_tests;

#[tokio::test]
async fn native_generations_are_monotonic_per_machine_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("generations.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let created = create_account(&store).await;
    let input = Request {
        account_id: created.account_id,
        machine_id: "a".into(),
        provider_version: "1".into(),
        generation_digest: "digest".into(),
        method: "native".into(),
    };
    let (a, b) = tokio::join!(
        store.claim_provider_login("login-owner", "a", &input),
        store.claim_provider_login("login-owner", "b", &input)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    let mut numbers = [
        a.receipt.native_generation.unwrap(),
        b.receipt.native_generation.unwrap(),
    ];
    numbers.sort_unstable();
    assert_eq!(numbers, [1, 2]);
    store
        .cancel_provider_login("login-owner", "a")
        .await
        .unwrap();
    let replay = store
        .claim_provider_login("login-owner", "a", &input)
        .await
        .unwrap();
    assert!(!replay.fresh);
    assert_eq!(
        replay.receipt.native_generation,
        a.receipt.native_generation
    );
    let mut another = input.clone();
    another.machine_id = "b".into();
    assert_eq!(
        store
            .claim_provider_login("login-owner", "other-machine", &another)
            .await
            .unwrap()
            .receipt
            .native_generation,
        Some(1)
    );
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    assert_eq!(
        store
            .claim_provider_login("login-owner", "next", &input)
            .await
            .unwrap()
            .receipt
            .native_generation,
        Some(3)
    );
}

#[test]
fn native_generation_migration_preserves_completed_profile_numbers() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE provider_login_operations (owner_user_id TEXT, value TEXT);")
        .unwrap();
    for (machine, generation) in [("a", 3), ("a", 7), ("b", 2)] {
        let value = serde_json::json!({"input":{"account_id":"account","machine_id":machine},
            "native_completion":{"profile":{"generation":generation}}});
        db.execute(
            "INSERT INTO provider_login_operations VALUES ('owner', ?1)",
            [value.to_string()],
        )
        .unwrap();
    }
    db.execute_batch(include_str!(
        "../../../../migrations/sqlite/0041_provider_native_generations.sql"
    ))
    .unwrap();
    let mut query = db
        .prepare(
            "SELECT machine_id,generation FROM provider_native_generations ORDER BY machine_id",
        )
        .unwrap();
    let rows = query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows, vec![("a".into(), 7), ("b".into(), 2)]);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep enrollment, replay and restart in one lifecycle fixture.
async fn sqlite_native_completion_is_exact_durable_evidence_not_a_grant() {
    use base64::Engine as _;
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("native.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let created = create_account(&store).await;
    let input = Request {
        account_id: created.account_id,
        machine_id: "native-machine".into(),
        provider_version: "1.0.0".into(),
        generation_digest: "sha256:fixture".into(),
        method: "native".into(),
    };
    let claim = store
        .claim_provider_login("login-owner", "native", &input)
        .await
        .unwrap();
    assert_eq!(
        store
            .provider_login_operation_for_request("login-owner", &claim.receipt.request_id)
            .await
            .unwrap()
            .as_deref(),
        Some("native")
    );
    assert!(
        store
            .provider_login_operation_for_request("another-owner", &claim.receipt.request_id)
            .await
            .unwrap()
            .is_none()
    );
    let key = base64::engine::general_purpose::STANDARD.encode(
        x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([11; 32])).as_bytes(),
    );
    let token = store
        .create_machine_enrollment(&input.machine_id, "Native", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&token, &format!("ssh-ed25519 {key}"), &key)
        .await
        .unwrap();
    let completion = NativeCompletion {
        machine_id: input.machine_id.clone(),
        recipient_fingerprint: crate::admin::hex_sha256(key.as_bytes()),
        profile: crate::provider_account::NativeProfileReceipt {
            account_id: input.account_id.clone(),
            provider_id: "claude-code".into(),
            request_id: claim.receipt.request_id.clone(),
            generation: 1,
            method: input.method.clone(),
            provider_version: input.provider_version.clone(),
            generation_digest: input.generation_digest.clone(),
            auth_contract_fingerprint: "contract".into(),
            identity: crate::provider_account::NativeIdentity {
                authority: "fixture".into(),
                subject: "personal".into(),
                tenant: None,
            },
        },
    };
    let mut legacy = serde_json::to_value(&claim.receipt).unwrap();
    legacy.as_object_mut().unwrap().remove("native_completion");
    assert!(
        serde_json::from_value::<Receipt>(legacy)
            .unwrap()
            .native_completion
            .is_none()
    );
    let cancelled = store
        .claim_provider_login("login-owner", "cancel-first", &input)
        .await
        .unwrap();
    store
        .cancel_provider_login("login-owner", "cancel-first")
        .await
        .unwrap();
    let mut late = completion.clone();
    late.profile.request_id = cancelled.receipt.request_id;
    late.profile.generation = cancelled.receipt.native_generation.unwrap();
    assert!(
        store
            .record_native_login_completion("login-owner", "cancel-first", &late)
            .await
            .is_err()
    );
    for field in 0..7 {
        let mut wrong = completion.clone();
        match field {
            0 => wrong.machine_id = "other".into(),
            1 => wrong.recipient_fingerprint = "wrong".into(),
            2 => wrong.profile.request_id = "other".into(),
            3 => wrong.profile.provider_version = "other".into(),
            4 => wrong.profile.method = "other".into(),
            5 => wrong.profile.generation_digest = "other".into(),
            _ => wrong.profile.account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap(),
        }
        assert!(
            store
                .record_native_login_completion("login-owner", "native", &wrong)
                .await
                .is_err()
        );
    }
    assert!(
        store
            .record_native_login_completion("other-owner", "native", &completion)
            .await
            .is_err()
    );
    let (a, b) = tokio::join!(
        store.record_native_login_completion("login-owner", "native", &completion),
        store.record_native_login_completion("login-owner", "native", &completion)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let mut conflicting = completion.clone();
    conflicting.profile.generation = 2;
    assert!(
        store
            .record_native_login_completion("login-owner", "native", &conflicting)
            .await
            .is_err()
    );
    assert!(
        store
            .reject_provider_login(
                "login-owner",
                "native",
                &claim,
                DispatchRejection::StartRejected
            )
            .await
            .is_err()
    );
    assert!(
        store
            .provider_grant_receipt("login-owner", &claim.receipt.request_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.provider_accounts("login-owner").await.unwrap(),
        created.registry
    );
    store
        .cancel_provider_login("login-owner", "native")
        .await
        .unwrap();
    store.revoke_machine(&input.machine_id).await.unwrap();
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let replay = store
        .record_native_login_completion("login-owner", "native", &completion)
        .await
        .unwrap();
    assert!(replay.cancel_requested);
    assert_eq!(replay.native_completion, Some(completion.clone()));
    let fresh = store
        .claim_provider_login("login-owner", "revoked", &input)
        .await
        .unwrap();
    let mut revoked = completion;
    revoked.profile.request_id = fresh.receipt.request_id;
    revoked.profile.generation = fresh.receipt.native_generation.unwrap();
    assert!(
        store
            .record_native_login_completion("login-owner", "revoked", &revoked)
            .await
            .is_err()
    );
}

async fn create_account(store: &Store) -> crate::store::provider_accounts::Receipt {
    let MutationResult::Applied(created) = store
        .mutate_provider_accounts(
            "login-owner",
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
        panic!("create account")
    };
    created
}

async fn contract(store: &Store) -> Request {
    let created = create_account(store).await;
    let input = Request {
        account_id: created.account_id,
        machine_id: "machine-a".into(),
        provider_version: "1.0.0".into(),
        generation_digest: "sha256:exact".into(),
        method: "native".into(),
    };
    let (a, b) = tokio::join!(
        store.claim_provider_login("login-owner", "start", &input),
        store.claim_provider_login("login-owner", "start", &input)
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.fresh, b.fresh, "only one concurrent caller can dispatch");
    assert_eq!(a.receipt, b.receipt);
    assert_eq!(a.receipt.provider_id, "claude-code");
    for field in 0..5 {
        let mut changed = input.clone();
        match field {
            0 => changed.account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            1 => changed.machine_id = "machine-b".into(),
            2 => changed.provider_version = "2.0.0".into(),
            3 => changed.generation_digest = "sha256:changed".into(),
            _ => changed.method = "other".into(),
        }
        assert!(
            store
                .claim_provider_login("login-owner", "start", &changed)
                .await
                .unwrap_err()
                .to_string()
                .contains("operation_reused")
        );
    }
    assert!(
        store
            .provider_login_receipt("other-owner", "start")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .claim_provider_login("other-owner", "start", &input)
            .await
            .is_err()
    );
    let MutationResult::Applied(_) = store
        .mutate_provider_accounts(
            "login-owner",
            "disable",
            &Mutation {
                expected_revision: created.registry.revision,
                change: Change::SetEnabled {
                    id: input.account_id.clone(),
                    enabled: false,
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("disable account")
    };
    let replay = store
        .claim_provider_login("login-owner", "start", &input)
        .await
        .unwrap();
    assert!(
        !replay.fresh,
        "historical receipt does not regain dispatch authority"
    );
    assert_eq!(replay.receipt, a.receipt);
    assert!(
        store
            .claim_provider_login("login-owner", "new-start", &input)
            .await
            .is_err()
    );
    assert!(
        store
            .provider_login_receipt("login-owner", "new-start")
            .await
            .unwrap()
            .is_none()
    );
    cancellation_contract(store, &input, &a.receipt.request_id).await;
    input
}

async fn cancellation_contract(store: &Store, input: &Request, request_id: &str) {
    for (owner, operation) in [("other-owner", "start"), ("login-owner", "missing")] {
        assert!(
            store
                .cancel_provider_login(owner, operation)
                .await
                .unwrap()
                .is_none()
        );
    }
    let (cancelled, concurrent) = tokio::join!(
        store.cancel_provider_login("login-owner", "start"),
        store.claim_provider_login("login-owner", "start", input)
    );
    let cancelled = cancelled.unwrap().unwrap();
    assert!(cancelled.cancel_requested);
    assert_eq!(&cancelled.input, input);
    assert_eq!(cancelled.request_id, request_id);
    assert!(!concurrent.unwrap().fresh);
    assert_eq!(
        store
            .cancel_provider_login("login-owner", "start")
            .await
            .unwrap(),
        Some(cancelled)
    );
    let replay = store
        .claim_provider_login("login-owner", "start", input)
        .await
        .unwrap();
    assert!(!replay.fresh);
    assert!(replay.receipt.cancel_requested);
}

#[tokio::test]
async fn sqlite_login_dispatch_survives_restart_without_redispatch() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("login.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let input = contract(&store).await;
    let before = store
        .provider_login_receipt("login-owner", "start")
        .await
        .unwrap();
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let after = store
        .claim_provider_login("login-owner", "start", &input)
        .await
        .unwrap();
    assert!(!after.fresh);
    assert!(after.receipt.cancel_requested);
    assert_eq!(Some(after.receipt), before);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_login_dispatch_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    contract(&store).await;
}

async fn rejection_contract(store: &Store) -> (Request, Receipt) {
    let created = create_account(store).await;
    let input = Request {
        account_id: created.account_id,
        machine_id: "machine-a".into(),
        provider_version: "1".into(),
        generation_digest: "digest".into(),
        method: "native".into(),
    };
    let claim = store
        .claim_provider_login("login-owner", "rejected", &input)
        .await
        .unwrap();
    assert!(
        store
            .reject_provider_login(
                "other-owner",
                "rejected",
                &claim,
                DispatchRejection::StartRejected
            )
            .await
            .is_err()
    );
    let (rejected, cancelled) = tokio::join!(
        store.reject_provider_login(
            "login-owner",
            "rejected",
            &claim,
            DispatchRejection::StartRejected
        ),
        store.cancel_provider_login("login-owner", "rejected")
    );
    rejected.unwrap();
    cancelled.unwrap();
    let receipt = store
        .reject_provider_login(
            "login-owner",
            "rejected",
            &claim,
            DispatchRejection::StartRejected,
        )
        .await
        .unwrap();
    assert!(receipt.cancel_requested);
    assert_eq!(
        receipt.dispatch_rejection,
        Some(DispatchRejection::StartRejected)
    );
    assert!(
        store
            .reject_provider_login(
                "login-owner",
                "rejected",
                &claim,
                DispatchRejection::CleanupRequired
            )
            .await
            .is_err()
    );
    let replay = store
        .claim_provider_login("login-owner", "rejected", &input)
        .await
        .unwrap();
    assert!(!replay.fresh);
    assert!(
        store
            .reject_provider_login(
                "login-owner",
                "rejected",
                &replay,
                DispatchRejection::StartRejected
            )
            .await
            .is_err()
    );
    let other = store
        .claim_provider_login("login-owner", "other", &input)
        .await
        .unwrap();
    assert!(
        store
            .reject_provider_login(
                "login-owner",
                "rejected",
                &other,
                DispatchRejection::StartRejected
            )
            .await
            .is_err()
    );
    (input, receipt)
}

#[tokio::test]
async fn sqlite_definite_rejection_survives_restart_and_concurrent_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("reject.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let (input, before) = rejection_contract(&store).await;
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let after = store
        .claim_provider_login("login-owner", "rejected", &input)
        .await
        .unwrap();
    assert!(!after.fresh);
    assert_eq!(after.receipt, before);
    let mut legacy = serde_json::to_value(before).unwrap();
    legacy.as_object_mut().unwrap().remove("dispatch_rejection");
    assert_eq!(
        serde_json::from_value::<Receipt>(legacy)
            .unwrap()
            .dispatch_rejection,
        None
    );
}
#[test]
fn never_dispatched_requires_explicit_pre_enqueue_refusal() {
    let mut receipt: super::Receipt = serde_json::from_value(serde_json::json!({
        "request_id": "request", "provider_id": "claude-code",
        "input": {"account_id":"11111111-1111-4111-8111-111111111111", "machine_id":"machine",
            "provider_version":"fixture", "generation_digest":"digest", "method":"native"},
        "native_generation": 1,
    }))
    .unwrap();
    assert!(!receipt.never_dispatched());
    receipt.dispatch_rejection = Some(super::DispatchRejection::StartRejected);
    assert!(!receipt.never_dispatched());
    receipt.dispatch_rejection = Some(super::DispatchRejection::CleanupRequired);
    assert!(receipt.never_dispatched());
    let original = receipt.clone();
    receipt.dispatch_recipient_fingerprint = Some("original".into());
    assert!(!receipt.never_dispatched());
    receipt = original.clone();
    receipt.dispatch_registry_revision = Some(1);
    assert!(!receipt.never_dispatched());
    receipt = original;
    receipt.native_binding_revision = Some(1);
    assert!(!receipt.never_dispatched());
}

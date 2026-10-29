use super::*;

async fn seed_owner(store: &Store) {
    let sql = "INSERT INTO users (id,username,password_algo,password_hash) VALUES ('login-owner','login-owner','fixture','fixture')";
    match &store.backend {
        StorageBackend::Postgres(db) => {
            sqlx::query(sql).execute(&db.pool).await.unwrap();
        }
        StorageBackend::Sqlite(db) => {
            sqlx::query(sql).execute(&db.pool).await.unwrap();
        }
    }
}

fn session(completion: &NativeCompletion) -> crate::core::SessionMeta {
    let hub = crate::core::Hub::new();
    hub.create_local_session(
        "native-session".into(),
        "claude-code".into(),
        "/fixture".into(),
        "original".into(),
        crate::core::SessionOrigin::Api,
        false,
    );
    let mut meta = hub.session_info("native-session").unwrap().meta;
    meta.owner_user_id = Some("login-owner".into());
    meta.machine_id = completion.machine_id.clone();
    meta.provider_account_id = completion.profile.account_id.clone();
    meta.provider_version = completion.profile.provider_version.clone();
    meta.provider_generation_digest = completion.profile.generation_digest.clone();
    meta.provider_auth_generation = Some(completion.profile.generation);
    meta.status = crate::core::Status::Starting;
    meta
}

async fn row_title(store: &Store, id: &str) -> Option<String> {
    let sql = "SELECT title FROM sessions WHERE id=$1";
    match &store.backend {
        StorageBackend::Postgres(db) => sqlx::query_scalar(sql)
            .bind(id)
            .fetch_optional(&db.pool)
            .await
            .unwrap(),
        StorageBackend::Sqlite(db) => sqlx::query_scalar(sql)
            .bind(id)
            .fetch_optional(&db.pool)
            .await
            .unwrap(),
    }
}

pub(super) async fn contract(store: &Store, completion: &NativeCompletion, replacement: bool) {
    seed_owner(store).await;
    let meta = session(completion);
    let (sink, mut writes) = crate::core::StoreSink::channel(16, std::sync::Arc::default());
    let hub = crate::core::Hub::with_store(Some(sink));
    // Catalog retention keeps the release this login was made under.
    assert!(
        store
            .native_login_generation_digests()
            .await
            .unwrap()
            .contains(&completion.profile.generation_digest)
    );
    for field in 0..8 {
        let mut bad = meta.clone();
        match field {
            0 => bad.owner_user_id = Some("other".into()),
            1 => bad.provider_account_id = crate::provider_account::ProviderAccountId::default(),
            2 => bad.machine_id = "other".into(),
            3 => bad.provider = "codex".into(),
            // A compatible later release is resolved by registration and
            // verified by the Machine; the row still needs an exact release.
            4 => bad.provider_version = String::new(),
            5 => bad.provider_generation_digest = String::new(),
            6 => bad.provider_auth_generation = None,
            _ => bad.status = crate::core::Status::Exited,
        }
        assert_eq!(
            store
                .insert_native_session("login-owner", "bind", completion, &bad)
                .await
                .unwrap_err()
                .to_string(),
            "native_session_scope_mismatch"
        );
        assert_eq!(row_title(store, &meta.id).await, None);
    }
    store
        .register_native_session("login-owner", "bind", completion, meta.clone(), &hub)
        .await
        .unwrap();
    assert_eq!(hub.session_info(&meta.id).unwrap().meta.title, "original");
    while let Ok(write) = writes.try_recv() {
        assert!(
            !matches!(write, crate::core::StoreWrite::InsertSession(_)),
            "committed row must not be inserted again by the async writer"
        );
    }
    let mut duplicate = meta.clone();
    duplicate.title = "overwritten".into();
    assert!(
        store
            .register_native_session("login-owner", "bind", completion, duplicate, &hub)
            .await
            .is_err()
    );
    assert_eq!(
        row_title(store, &meta.id).await.as_deref(),
        Some("original")
    );
    assert_eq!(hub.session_info(&meta.id).unwrap().meta.title, "original");
    projection_conflict(store, completion).await;
    transport_contract(store, &hub, &meta, true).await;
    if replacement {
        Box::pin(replacement_login_preserves_pinned_session(
            store, completion, &hub, &meta,
        ))
        .await;
        Box::pin(super::transition_tests::contract(store, completion, &meta)).await;
    }
    race_revocation(store, completion, meta.clone()).await;
    transport_contract(store, &hub, &meta, false).await;
}

async fn replacement_login_preserves_pinned_session(
    store: &Store,
    completion: &NativeCompletion,
    hub: &crate::core::Hub,
    meta: &crate::core::SessionMeta,
) {
    let original = store
        .provider_login_receipt("login-owner", "bind")
        .await
        .unwrap()
        .unwrap();
    let mut input = original.input;
    input.provider_version = "next-version".into();
    input.generation_digest = "next-digest".into();
    let replacement = store
        .claim_provider_login("login-owner", "replacement", &input)
        .await
        .unwrap();
    assert!(replacement.receipt.native_generation.unwrap() > completion.profile.generation);
    assert!(
        store
            .native_login_for_session("login-owner", &input.account_id, &input.machine_id)
            .await
            .unwrap()
            .is_none(),
        "new sessions must not fall back to the old login while replacement is pending"
    );
    assert_eq!(
        store
            .native_login_for_registered_session("login-owner", &meta.id)
            .await
            .unwrap(),
        Some(("bind".into(), completion.clone()))
    );
    assert!(
        store
            .native_login_for_registered_session("other-owner", &meta.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .native_login_for_registered_session("login-owner", "absent")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .with_native_login_binding("login-owner", "bind", completion, || Ok(()))
            .await
            .is_err()
    );
    let mut new_session = meta.clone();
    new_session.id = "old-login-new-session".into();
    assert!(
        store
            .insert_native_session("login-owner", "bind", completion, &new_session)
            .await
            .is_err()
    );
    assert!(
        store
            .with_native_session_binding("login-owner", "bind", completion, "absent", || async {
                panic!("missing session must not authorize transport");
                #[allow(unreachable_code)]
                Ok::<(), anyhow::Error>(())
            })
            .await
            .is_err()
    );
    transport_contract(store, hub, meta, true).await;
    durable_binding_changes(store, completion, hub, meta).await;
    store
        .cancel_provider_login("login-owner", "replacement")
        .await
        .unwrap();
    Box::pin(abandoned_replacement_falls_back(
        store, completion, hub, meta, &input,
    ))
    .await;
}

async fn abandoned_replacement_falls_back(
    store: &Store,
    completion: &NativeCompletion,
    hub: &crate::core::Hub,
    meta: &crate::core::SessionMeta,
    input: &crate::store::provider_accounts::logins::Request,
) {
    // A cancelled replacement that never completed falls back to the previous
    // working login, for discovery, the login list and locked admission.
    assert!(
        !store
            .native_login_pending("login-owner", &input.account_id, &input.machine_id)
            .await
            .unwrap()
    );
    let listed = store
        .native_login_receipts("login-owner", &input.account_id, "", 10)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].native_generation,
        Some(completion.profile.generation)
    );
    assert_eq!(
        store
            .native_login_for_session("login-owner", &input.account_id, &input.machine_id)
            .await
            .unwrap(),
        Some(("bind".into(), completion.clone()))
    );
    transport_contract(store, hub, meta, true).await;
    // A newer pending replacement on top of a cancelled one still blocks.
    store
        .claim_provider_login("login-owner", "replacement-2", input)
        .await
        .unwrap();
    assert!(
        store
            .native_login_for_session("login-owner", &input.account_id, &input.machine_id)
            .await
            .unwrap()
            .is_none(),
        "a pending replacement never falls back"
    );
    assert!(
        store
            .native_login_pending("login-owner", &input.account_id, &input.machine_id)
            .await
            .unwrap(),
        "an unfinished replacement is reported as pending, not as signed out"
    );
    let listed = store
        .native_login_receipts("login-owner", &input.account_id, "", 10)
        .await
        .unwrap();
    assert!(listed[0].native_generation > Some(completion.profile.generation));
    let mut blocked = meta.clone();
    blocked.id = "pending-replacement-new-session".into();
    assert!(
        store
            .insert_native_session("login-owner", "bind", completion, &blocked)
            .await
            .is_err()
    );
    store
        .cancel_provider_login("login-owner", "replacement-2")
        .await
        .unwrap();
    assert_eq!(
        store
            .native_login_for_session("login-owner", &input.account_id, &input.machine_id)
            .await
            .unwrap(),
        Some(("bind".into(), completion.clone())),
        "every abandoned newer generation is skipped"
    );
    let mut fallback = meta.clone();
    fallback.id = "fallback-new-session".into();
    store
        .insert_native_session("login-owner", "bind", completion, &fallback)
        .await
        .unwrap();
    transport_contract(store, hub, meta, true).await;
    Box::pin(named_release_write_keeps_its_login(store, &fallback)).await;
}

async fn stored_release(store: &Store, id: &str) -> (String, Option<i64>) {
    let query = "SELECT provider_version, provider_auth_generation FROM sessions WHERE id=$1";
    match &store.backend {
        StorageBackend::Postgres(db) => sqlx::query_as(query).bind(id).fetch_one(&db.pool).await,
        StorageBackend::Sqlite(db) => sqlx::query_as(query).bind(id).fetch_one(&db.pool).await,
    }
    .unwrap()
}

/// A named row's release write applies only on the login generation it was
/// admitted with, so a concurrently committed transition is never overwritten.
async fn named_release_write_keeps_its_login(store: &Store, meta: &crate::core::SessionMeta) {
    let generation = i64::try_from(meta.provider_auth_generation.unwrap()).unwrap();
    let mut moved = meta.clone();
    moved.provider_version = "carried-version".into();
    store.reload_provider(&moved).await.unwrap();
    assert_eq!(
        stored_release(store, &meta.id).await,
        ("carried-version".into(), Some(generation))
    );
    let query = "UPDATE sessions SET provider_auth_generation=$1 WHERE id=$2";
    match &store.backend {
        StorageBackend::Postgres(db) => {
            sqlx::query(query)
                .bind(generation + 1)
                .bind(&meta.id)
                .execute(&db.pool)
                .await
                .unwrap();
        }
        StorageBackend::Sqlite(db) => {
            sqlx::query(query)
                .bind(generation + 1)
                .bind(&meta.id)
                .execute(&db.pool)
                .await
                .unwrap();
        }
    }
    moved.provider_version = "stale-version".into();
    assert!(store.reload_provider(&moved).await.is_err());
    assert_eq!(
        stored_release(store, &meta.id).await,
        ("carried-version".into(), Some(generation + 1))
    );
}

async fn durable_binding_changes(
    store: &Store,
    completion: &NativeCompletion,
    hub: &crate::core::Hub,
    meta: &crate::core::SessionMeta,
) {
    // A cached Hub binding must not survive a conflicting durable edit. Check
    // both discovery and the final locked admission after discovery has raced.
    for version in ["changed-version", meta.provider_version.as_str()] {
        let query = "UPDATE sessions SET provider_version=$1 WHERE id=$2";
        match &store.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query(query)
                    .bind(version)
                    .bind(&meta.id)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query(query)
                    .bind(version)
                    .bind(&meta.id)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
        }
        let expected = version == meta.provider_version;
        let invoked = std::sync::atomic::AtomicBool::new(false);
        let admitted = store
            .with_native_session_binding("login-owner", "bind", completion, &meta.id, || async {
                invoked.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(admitted.is_ok(), expected);
        assert_eq!(invoked.load(std::sync::atomic::Ordering::SeqCst), expected);
        // Runtime admission compares the row with the session's own pinned
        // release, which may be a compatible release other than the login's.
        let mut pinned = meta.clone();
        pinned.provider_version = version.into();
        assert!(
            store
                .with_native_session_release_binding(
                    "login-owner",
                    "bind",
                    completion,
                    &pinned,
                    || async { Ok(()) }
                )
                .await
                .is_ok(),
            "a session pinned to {version} with the original login is admitted"
        );
        assert_eq!(
            store
                .with_native_session_release_binding(
                    "login-owner",
                    "bind",
                    completion,
                    meta,
                    || async { Ok(()) }
                )
                .await
                .is_ok(),
            expected,
            "a stale expected release never passes a changed row"
        );
        transport_contract(store, hub, meta, expected).await;
    }
    // Remove the durable identity while retaining the same live Hub fixture.
    // Renaming it back restores only this test fixture, not production state.
    for (from, to, expected) in [
        (meta.id.as_str(), "temporarily-absent", false),
        ("temporarily-absent", meta.id.as_str(), true),
    ] {
        let query = "UPDATE sessions SET id=$1 WHERE id=$2";
        match &store.backend {
            StorageBackend::Postgres(db) => {
                sqlx::query(query)
                    .bind(to)
                    .bind(from)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
            StorageBackend::Sqlite(db) => {
                sqlx::query(query)
                    .bind(to)
                    .bind(from)
                    .execute(&db.pool)
                    .await
                    .unwrap();
            }
        }
        transport_contract(store, hub, meta, expected).await;
    }
}

async fn transport_contract(
    store: &Store,
    hub: &crate::core::Hub,
    meta: &crate::core::SessionMeta,
    admitted: bool,
) {
    use crate::runtime_wire::{CoreCommand, StartSession};
    let recipient = store
        .machine_encryption_public_key(&meta.machine_id)
        .await
        .unwrap()
        .unwrap();
    let runtime =
        crate::remote_runtime::RemoteRuntime::for_native_transport_test(hub.clone(), recipient);
    let mut session: StartSession = serde_json::from_value(serde_json::json!({
        "session_id":meta.id, "provider":meta.provider, "provider_account_id":meta.provider_account_id,
        "provider_version":meta.provider_version, "provider_generation_digest":meta.provider_generation_digest,
        "provider_auth_generation":meta.provider_auth_generation, "cwd":meta.cwd, "generation":"fixture"
    })).unwrap();
    let mut bytes = Vec::new();
    assert!(
        !runtime
            .send_native_test(
                &mut bytes,
                CoreCommand::EnsureSession {
                    session: session.clone()
                }
            )
            .await
            .unwrap()
    );
    assert!(bytes.is_empty(), "missing admission store must not write");
    runtime.attach_native_admission(meta.machine_id.clone(), store.clone());
    let stale = crate::remote_runtime::RemoteRuntime::for_native_transport_test(
        hub.clone(),
        "old-enrollment".into(),
    );
    stale.attach_native_admission(meta.machine_id.clone(), store.clone());
    assert!(
        !stale
            .send_native_test(
                &mut bytes,
                CoreCommand::EnsureSession {
                    session: session.clone()
                }
            )
            .await
            .unwrap()
    );
    assert!(
        bytes.is_empty(),
        "an old connection key must not inherit fresh enrollment authority"
    );
    for adopt in [false, true] {
        session.adopt_only = adopt;
        bytes.clear();
        assert_eq!(
            runtime
                .send_native_test(
                    &mut bytes,
                    CoreCommand::EnsureSession {
                        session: session.clone()
                    }
                )
                .await
                .unwrap(),
            admitted
        );
        assert_eq!(!bytes.is_empty(), admitted);
    }
    if admitted {
        stale_placement_contract(&runtime, &session).await;
        transport_failure(&runtime, session.clone()).await;
    }
    snapshot_contract(&runtime, &stale, &session, admitted).await;
    session.provider_auth_generation = Some(999);
    bytes.clear();
    assert!(
        !runtime
            .send_native_test(&mut bytes, CoreCommand::EnsureSession { session })
            .await
            .unwrap()
    );
    assert!(bytes.is_empty());
    assert!(
        runtime
            .send_native_test(
                &mut bytes,
                CoreCommand::Cancel {
                    session_id: meta.id.clone(),
                    command_id: "cancel-fixture".into(),
                }
            )
            .await
            .unwrap()
    );
    assert!(
        !bytes.is_empty(),
        "cleanup must remain available after revocation"
    );
}

async fn stale_placement_contract(
    runtime: &crate::remote_runtime::RemoteRuntime,
    session: &crate::runtime_wire::StartSession,
) {
    for field in 0..4 {
        let mut stale = session.clone();
        match field {
            0 => stale.cwd.push_str("/other-workspace"),
            1 => stale.execution_binding = Some(crate::execution_environment::fixture()),
            2 => stale.system = !stale.system,
            _ => {
                let manifest: serde_json::Value = serde_json::from_str(include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/plugins/claude-code/provider.json"
                )))
                .unwrap();
                stale.provider_behavior =
                    Some(serde_json::from_value(manifest["runtime"]["behavior"].clone()).unwrap());
            }
        }
        assert_ne!(&stale, session);
        let mut bytes = Vec::new();
        assert!(
            !runtime
                .send_native_test(
                    &mut bytes,
                    crate::runtime_wire::CoreCommand::EnsureSession {
                        session: stale.clone()
                    }
                )
                .await
                .unwrap()
        );
        assert!(
            bytes.is_empty(),
            "stale placement must be refused before writing"
        );
        let worker = serde_json::from_value(serde_json::json!({
            "session_id":session.session_id,"worker_epoch":"fixture","generation":"fixture",
            "launch":stale,"state":"running"
        }))
        .unwrap();
        assert!(
            !runtime.snapshot_native_test(worker).await,
            "stale placement must not enter the worker cache or Hub"
        );
    }
}

async fn snapshot_contract(
    runtime: &crate::remote_runtime::RemoteRuntime,
    stale: &crate::remote_runtime::RemoteRuntime,
    session: &crate::runtime_wire::StartSession,
    admitted: bool,
) {
    let worker: crate::runtime_wire::WorkerSnapshot = serde_json::from_value(serde_json::json!({
        "session_id":session.session_id, "worker_epoch":"fixture", "generation":"fixture",
        "launch":session, "state":"running"
    }))
    .unwrap();
    assert!(!stale.snapshot_native_test(worker.clone()).await);
    assert_eq!(runtime.snapshot_native_test(worker.clone()).await, admitted);
    assert_eq!(
        runtime.event_native_test(&worker, true, "fixture").await,
        admitted
    );
    assert!(!runtime.event_native_test(&worker, false, "fixture").await);
    assert!(!runtime.event_native_test(&worker, true, "old-worker").await);
    assert!(!stale.event_native_test(&worker, true, "fixture").await);
    for field in 0..4 {
        let mut bad = worker.clone();
        match field {
            0 => bad.launch = None,
            1 => bad.launch.as_mut().unwrap().provider_auth_generation = Some(999),
            2 => {
                bad.launch.as_mut().unwrap().provider_account_id =
                    crate::provider_account::ProviderAccountId::default();
            }
            _ => bad.launch.as_mut().unwrap().session_id = "other".into(),
        }
        assert!(!runtime.snapshot_native_test(bad).await);
    }
}

async fn transport_failure(
    runtime: &crate::remote_runtime::RemoteRuntime,
    session: crate::runtime_wire::StartSession,
) {
    let (mut writer, reader) = tokio::io::duplex(1);
    drop(reader);
    assert!(
        runtime
            .send_native_test(
                &mut writer,
                crate::runtime_wire::CoreCommand::EnsureSession { session }
            )
            .await
            .is_err(),
        "transport failure must propagate rather than become a rejection"
    );
}

async fn projection_conflict(store: &Store, completion: &NativeCompletion) {
    let mut meta = session(completion);
    meta.id = "projection-conflict".into();
    let hub = crate::core::Hub::new();
    hub.create_local_session(
        meta.id.clone(),
        "claude-code".into(),
        "/existing".into(),
        "existing session".into(),
        crate::core::SessionOrigin::Api,
        false,
    );
    let error = store
        .register_native_session("login-owner", "bind", completion, meta.clone(), &hub)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "session_projection_conflict");
    assert_eq!(
        row_title(store, &meta.id).await.as_deref(),
        Some("original")
    );
    assert_eq!(
        hub.session_info(&meta.id).unwrap().meta.title,
        "existing session"
    );
    assert_eq!(hub.session_list().len(), 1);
}

async fn race_revocation(
    store: &Store,
    completion: &NativeCompletion,
    mut meta: crate::core::SessionMeta,
) {
    meta.id = "racing-session".into();
    let hub = crate::core::Hub::new();
    let mutation = Mutation {
        expected_revision: store
            .provider_accounts("login-owner")
            .await
            .unwrap()
            .revision,
        change: Change::BeginRevocation {
            id: meta.provider_account_id.clone(),
        },
    };
    let (insert, revoke) = tokio::join!(
        store.register_native_session("login-owner", "bind", completion, meta.clone(), &hub),
        store.mutate_provider_accounts("login-owner", "revoke-registration", &mutation),
    );
    let MutationResult::Applied(receipt) = revoke.unwrap() else {
        panic!("revocation must commit")
    };
    let plan = receipt.revocation.unwrap();
    assert!(
        plan.sessions
            .iter()
            .any(|s| s.session_id == "native-session")
    );
    assert!(
        plan.sessions
            .iter()
            .any(|s| s.session_id == "projection-conflict")
    );
    let persisted = row_title(store, &meta.id).await.is_some();
    assert_eq!(
        plan.sessions.iter().any(|s| s.session_id == meta.id),
        persisted
    );
    assert_eq!(hub.session_info(&meta.id).is_some(), insert.is_ok());
    assert!(insert.is_err() || persisted);
    meta.id = "after-revocation".into();
    assert!(
        store
            .register_native_session("login-owner", "bind", completion, meta.clone(), &hub)
            .await
            .is_err()
    );
    assert_eq!(row_title(store, &meta.id).await, None);
    assert!(hub.session_info(&meta.id).is_none());
}

#[tokio::test]
async fn sqlite_replacement_login_preserves_existing_session_binding() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("replacement.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    native_binding_contract(&store, Some(true)).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_replacement_login_preserves_existing_session_binding() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    native_binding_contract(&store, Some(true)).await;
}

#[tokio::test]
async fn sqlite_native_session_registration_cannot_escape_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("registration.sqlite").display()
    );
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    native_binding_contract(&store, Some(false)).await;
    drop(store);
    let recovered = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let receipt = recovered
        .provider_account_receipt("login-owner", "revoke-registration")
        .await
        .unwrap()
        .unwrap();
    assert!(
        receipt
            .revocation
            .unwrap()
            .sessions
            .iter()
            .any(|s| s.session_id == "native-session")
    );
    assert_eq!(
        row_title(&recovered, "native-session").await.as_deref(),
        Some("original")
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL fixture; just test-postgres"]
async fn postgres_native_session_registration_cannot_escape_revocation() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated database URL");
    let dir = tempfile::tempdir().unwrap();
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    native_binding_contract(&store, Some(false)).await;
}

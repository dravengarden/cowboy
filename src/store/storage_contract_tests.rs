use super::*;

pub(super) fn session(id: &str) -> SessionMeta {
    SessionMeta {
        continued_from: None,
        forked_from: None,
        id: id.to_owned(),
        provider: "codex".to_owned(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "hawk".to_owned(),
        workspace_id: Some("cowboy".to_owned()),
        workspace_name: Some("Cowboy".to_owned()),
        workspace_source_path: Some("/tmp/cowboy".to_owned()),
        execution_binding: None,
        cwd: "/tmp/cowboy-worktree".to_owned(),
        title: "Storage contract".to_owned(),
        status: Status::Starting,
        origin: SessionOrigin::Web,
        agent_session_id: None,
        paused: false,
        closing: false,
        system: false,
        context_used: 0,
        context_size: 0,
        usage: None,
        background_tasks: 0,
        supports_current_turn: false,
        provider_update: None,
        provider_update_available: None,
        resource_wait: None,
        next_schedule_ms: None,
        owner_user_id: None,
        owner_username: None,
        trashed_at_ms: None,
    }
}

#[test]
fn generated_machine_id_matches_wire_rules() {
    let id = generate_machine_id().expect("random Machine id");
    assert!(valid_machine_id(&id), "{id}");
    assert!(id.starts_with("m-"), "{id}");
    assert_eq!(id.len(), 18, "{id}");
}

async fn execution_binding_restore_contract(url: &str, root: &std::path::Path) {
    use crate::execution_environment::ExecutionBinding;
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    let mut known = crate::execution_environment::fixture().record().clone();
    known["runtime"]["machine_id"] = "hawk".into();
    known["runtime"]["cwd"] = "/tmp/cowboy-worktree".into();
    known["environment"]["machine_id"] = "falcon".into();
    let mut unknown = known.clone();
    unknown["schema"] = 99.into();
    unknown["future_field"] = serde_json::json!({"preserve": [1, 2, 3]});
    let records = [
        None,
        Some(known),
        Some(unknown),
        Some(serde_json::Value::Null),
    ];
    for (index, record) in records.iter().enumerate() {
        let mut meta = session(&format!("execution-binding-{index}"));
        meta.execution_binding = record.clone().map(ExecutionBinding::from_record);
        store.insert_session(&meta).await.unwrap();
    }
    drop(store);
    // A second real database open exercises the persisted column, rather
    // than a cached Hub copy. Unknown records survive alongside local ones.
    let reopened = Store::connect(url, root.join("artifacts")).await.unwrap();
    reopened.migrate().await.unwrap();
    let rows = reopened.load_all().await.unwrap();
    for (index, record) in records.iter().enumerate() {
        let meta = &rows
            .iter()
            .find(|row| row.meta.id == format!("execution-binding-{index}"))
            .unwrap()
            .meta;
        assert_eq!(
            meta.execution_binding
                .as_ref()
                .map(ExecutionBinding::record),
            record.as_ref()
        );
        assert_eq!(meta.require_runtime_launch().is_ok(), index == 0);
        assert_eq!(meta.machine_id, "hawk");
        assert_eq!(meta.cwd, "/tmp/cowboy-worktree");
        if index == 1 {
            let binding = meta.execution_binding.as_ref().unwrap().decode().unwrap();
            assert_eq!(binding.environment.machine_id, "falcon");
            assert_eq!(binding.workspace.cwd, "/tasks/cowboy");
        }
    }
    execution_binding_creation_contract(&reopened).await;
}

async fn execution_binding_creation_contract(reopened: &Store) {
    use crate::execution_environment::ExecutionBinding;
    let mut ready = crate::execution_environment::fixture().record().clone();
    ready["runtime"]["machine_id"] = "hawk".into();
    ready["environment"]["machine_id"] = "falcon".into();
    let ready = ExecutionBinding::from_record(ready);
    let binding = ready.decode().unwrap();
    let intent = crate::execution_environment::PreparationV1 {
        schema: 1,
        phase: "preparing".into(),
        runtime: binding.runtime.clone(),
        machine_id: binding.environment.machine_id.clone(),
        workspace_id: binding.workspace.id.clone(),
        source_path: binding.workspace.source_path.clone(),
        executor_digest: binding.environment.executor_digest.clone(),
    };
    let expected = serde_json::to_value(intent).unwrap();
    let mut meta = session("execution-preparing");
    meta.machine_id = binding.runtime.machine_id;
    meta.cwd = binding.runtime.cwd;
    meta.execution_binding = Some(ExecutionBinding::from_record(expected.clone()));
    meta.provider_version = "3.2.0".into();
    meta.provider_generation_digest = format!("sha256:{}", "d".repeat(64));
    let mut behavior = crate::provider::legacy_behavior("codex");
    behavior.execution = Some(cowboy_provider_sdk::ExecutionBehavior::JsonrpcV1 {
        executor_digests: std::collections::BTreeSet::from([binding.environment.executor_digest]),
    });
    meta.provider_behavior = Some(behavior);
    assert!(meta.require_runtime_launch().is_err());
    reopened.insert_session(&meta).await.unwrap();
    assert!(
        !reopened
            .commit_execution_binding(&meta.id, &serde_json::Value::Null, ready.record())
            .await
            .unwrap()
    );
    assert!(
        reopened
            .execution_binding_matches(&meta.id, &expected)
            .await
            .unwrap()
    );
    assert!(
        reopened
            .commit_execution_binding(&meta.id, &expected, ready.record())
            .await
            .unwrap()
    );
    assert!(
        reopened
            .commit_execution_binding(&meta.id, &expected, ready.record())
            .await
            .unwrap()
    );
    let restored = reopened
        .load_all()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.meta.id == meta.id)
        .unwrap();
    assert_eq!(restored.meta.execution_binding, Some(ready));
    assert!(restored.meta.require_runtime_launch().is_ok());
    let mut no_grant = restored.meta;
    no_grant.provider_behavior.as_mut().unwrap().execution = None;
    assert!(no_grant.require_runtime_launch().is_err());
}

/// The sweeper's reference scan must find every `/api/artifacts/` URL a
/// retained event carries, and prune only unreferenced files past the
/// minimum age.
#[tokio::test]
async fn sqlite_purge_prunes_only_unreferenced_aged_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}",
        root.path().join("sessions.sqlite3").display()
    );
    let artifacts = root.path().join("artifacts");
    let store = Store::connect(&url, artifacts.clone()).await.unwrap();
    store.migrate().await.unwrap();
    let session_id = "sess-artifacts";
    store.insert_session(&session(session_id)).await.unwrap();
    let image = vec![9_u8; 40_000];
    let referenced = format!("{}.png", hex_sha256(&image));
    let event = Envelope {
        session_id: session_id.to_owned(),
        seq: 0,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "image",
                    "data": base64::engine::general_purpose::STANDARD.encode(&image),
                    "mimeType": "image/png",
                },
            }),
        },
        cmid: None,
    };
    store
        .upsert_event_batch(&[event], &HashMap::from([(session_id.to_owned(), 1)]))
        .await
        .unwrap();
    let referenced_path = store
        .artifact_path(&referenced)
        .expect("externalized image");
    let aged = artifacts.join(format!("{}.png", "b".repeat(64)));
    let fresh = artifacts.join(format!("{}.png", "c".repeat(64)));
    std::fs::write(&aged, b"stale").unwrap();
    std::fs::write(&fresh, b"recent").unwrap();
    let two_days_ago = std::time::SystemTime::now() - Duration::from_hours(48);
    for path in [&referenced_path, &aged] {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(two_days_ago)
            .unwrap();
    }

    store.purge_deleted(0).await.unwrap();

    assert!(
        referenced_path.exists(),
        "a referenced artifact survives regardless of age"
    );
    assert!(
        !aged.exists(),
        "an unreferenced artifact past the minimum age is pruned"
    );
    assert!(
        fresh.exists(),
        "an unreferenced artifact inside the minimum age is kept"
    );
}

/// A fresh SQLite store keeps its free pages reclaimable: after a purged
/// session leaves pages behind, the sweeper's bounded reclaim returns them
/// and a following pass has nothing left to return.
#[tokio::test]
async fn sqlite_reclaims_free_pages_after_a_purge() {
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}",
        root.path().join("sessions.sqlite3").display()
    );
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let session_id = "sess-bulky";
    store.insert_session(&session(session_id)).await.unwrap();
    let padding = "x".repeat(16 * 1024);
    let events: Vec<Envelope> = (0..800)
        .map(|seq| Envelope {
            session_id: session_id.to_owned(),
            seq,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": padding},
                }),
            },
            cmid: None,
        })
        .collect();
    store
        .upsert_event_batch(&events, &HashMap::from([(session_id.to_owned(), 800)]))
        .await
        .unwrap();
    store.delete_session(session_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert_eq!(store.purge_deleted(0).await.unwrap(), 1);

    let reclaimed = store.reclaim_free_pages().await.unwrap();
    // 800 × 16 KiB of text leaves well over the 4 MiB headroom the
    // reclaim keeps for the next inserts.
    assert!(
        reclaimed >= 6 * 1024 * 1024,
        "a purged 12 MiB session leaves pages to reclaim, got {reclaimed}"
    );
    assert_eq!(
        store.reclaim_free_pages().await.unwrap(),
        0,
        "a second pass finds only the retained headroom"
    );
}

#[tokio::test]
async fn sqlite_execution_environment_binding_survives_restore() {
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}",
        root.path().join("sessions.sqlite3").display()
    );
    execution_binding_restore_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_execution_environment_binding_survives_restore() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    let root = tempfile::tempdir().unwrap();
    execution_binding_restore_contract(&url, root.path()).await;
}

/// Every key a Hub caller may set must come back on load; an unregistered
/// key would otherwise fail the store writer in production.
async fn hub_settings_round_trip_contract(url: &str, root: &std::path::Path) {
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    let keys = crate::core::settings_keys::examples();
    for (index, key) in keys.iter().enumerate() {
        store
            .put_setting(key, &serde_json::json!({"index": index}))
            .await
            .unwrap_or_else(|error| panic!("{key}: {error:#}"));
    }
    assert!(
        store
            .put_setting("session.autoResume.default", &serde_json::json!(true))
            .await
            .is_err()
    );
    let reopened = Store::connect(url, root.join("artifacts")).await.unwrap();
    let loaded: HashMap<String, serde_json::Value> = reopened
        .load_settings()
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(loaded.len(), keys.len(), "{loaded:?}");
    for (index, key) in keys.iter().enumerate() {
        assert_eq!(
            loaded.get(key),
            Some(&serde_json::json!({"index": index})),
            "{key}"
        );
    }
    let key = format!(
        "{}cas-session",
        crate::core::settings_keys::SESSION_DELIVERY_POLICY_PREFIX
    );
    let first = serde_json::json!({"mode":"next_turn","revision":1});
    let second = serde_json::json!({"mode":"steer","revision":2});
    assert!(
        !store
            .compare_and_put_setting(&key, Some(&first), &second)
            .await
            .unwrap()
    );
    assert!(
        store
            .compare_and_put_setting(&key, None, &first)
            .await
            .unwrap()
    );
    assert!(
        !reopened
            .compare_and_put_setting(&key, None, &second)
            .await
            .unwrap()
    );
    let (left, right) = tokio::join!(
        store.compare_and_put_setting(&key, Some(&first), &second),
        reopened.compare_and_put_setting(&key, Some(&first), &second),
    );
    assert_ne!(
        left.unwrap(),
        right.unwrap(),
        "only one concurrent policy edit may commit"
    );
    assert!(
        !store
            .compare_and_put_setting(&key, Some(&first), &second)
            .await
            .unwrap()
    );
    assert!(
        reopened
            .load_settings()
            .await
            .unwrap()
            .contains(&(key, second))
    );
}

#[tokio::test]
async fn sqlite_every_hub_setting_key_round_trips() {
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}",
        root.path().join("settings.sqlite3").display()
    );
    hub_settings_round_trip_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_every_hub_setting_key_round_trips() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    let root = tempfile::tempdir().unwrap();
    hub_settings_round_trip_contract(&url, root.path()).await;
}

#[test]
fn reset_provider_ids_use_plugin_slug_contract() {
    for provider in ["codex", "xai", "future-reset", "provider2"] {
        ensure_reset_provider(provider).unwrap();
    }
    for provider in [
        "",
        "Future",
        "-future",
        "future-",
        "future--reset",
        "future_reset",
    ] {
        assert!(ensure_reset_provider(provider).is_err(), "{provider:?}");
    }
    assert!(ensure_reset_provider(&"a".repeat(64)).is_ok());
    assert!(ensure_reset_provider(&"a".repeat(65)).is_err());
}

#[test]
fn machine_encryption_key_must_be_canonical_and_contributory() {
    let secret = x25519_dalek::StaticSecret::from([0x24_u8; 32]);
    let public = x25519_dalek::PublicKey::from(&secret);
    let encoded = base64::engine::general_purpose::STANDARD.encode(public.as_bytes());
    assert!(validate_encryption_public_key(&encoded).is_ok());
    assert!(
        validate_encryption_public_key(
            &base64::engine::general_purpose::STANDARD.encode([0_u8; 32])
        )
        .unwrap_err()
        .to_string()
        .contains("non-contributory")
    );
    assert!(validate_encryption_public_key(encoded.trim_end_matches('=')).is_err());
}

async fn assert_machine_contract(store: &Store) -> Result<()> {
    assert!(store.list_machines().await?.iter().any(|machine| {
        machine.id == "hawk" && machine.connection_mode == "local" && !machine.revoked
    }));
    let encryption_secret = x25519_dalek::StaticSecret::from([0x24_u8; 32]);
    let encryption_public = base64::engine::general_purpose::STANDARD
        .encode(x25519_dalek::PublicKey::from(&encryption_secret).as_bytes());
    let cancelled_token = store
        .create_machine_enrollment("cancelled-machine", "Cancelled Machine", 60)
        .await?;
    store.cancel_machine_enrollment(&cancelled_token).await?;
    assert!(
        store
            .consume_machine_enrollment(&cancelled_token, "ssh-ed25519 QUJD", &encryption_public,)
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid, expired, or already used")
    );
    let token = store
        .create_machine_enrollment("contract-machine", "Contract Machine", 60)
        .await?;
    let enrolled = store
        .consume_machine_enrollment(&token, "ssh-ed25519 QUJD", &encryption_public)
        .await?;
    assert_eq!(enrolled.id, "contract-machine");
    assert_eq!(enrolled.display_name, "Contract Machine");
    assert!(!enrolled.fingerprint.is_empty());
    assert_eq!(
        store
            .machine_public_key("contract-machine")
            .await?
            .as_deref(),
        Some("ssh-ed25519 QUJD")
    );
    assert_eq!(
        store
            .machine_encryption_public_key("contract-machine")
            .await?
            .as_deref(),
        Some(encryption_public.as_str())
    );

    store
        .machine_connected(
            "contract-machine",
            "contract-epoch",
            "linux",
            "x86_64",
            "outbound_wss",
            &serde_json::json!([{"component": "worker", "version": "test"}]),
        )
        .await?;
    assert!(
        store
            .machine_connection_is_current("contract-machine", "contract-epoch")
            .await?
    );
    store
        .machine_seen(
            "contract-machine",
            "contract-epoch",
            Some(&serde_json::json!([{"component": "worker", "healthy": true}])),
        )
        .await?;
    let machines = store.list_machines().await?;
    let remote = machines
        .iter()
        .find(|machine| machine.id == "contract-machine")
        .context("enrolled Machine was not listed")?;
    assert_eq!(remote.status, "online");
    assert_eq!(remote.inventory[0]["healthy"], true);
    assert!(remote.last_seen_at_ms.is_some());

    store
        .machine_disconnected("contract-machine", "contract-epoch", 0)
        .await?;
    assert!(store.expire_machine_reconnects().await? >= 1);
    store.revoke_machine("contract-machine").await?;
    assert!(
        store
            .machine_public_key("contract-machine")
            .await?
            .is_none()
    );
    assert!(
        store
            .list_machines()
            .await?
            .iter()
            .any(|machine| machine.id == "contract-machine" && machine.revoked)
    );
    Ok(())
}

async fn assert_provider_action_contract(store: &Store) -> Result<()> {
    let fire_at_ms = 1_900_000_000_000;
    store
        .upsert_provider_reset("codex", fire_at_ms, "storage-contract-reset")
        .await?;
    let scheduled = store
        .load_provider_reset("codex")
        .await?
        .context("scheduled provider action was not restored")?;
    assert_eq!(scheduled.fire_at_ms, fire_at_ms);
    assert_eq!(scheduled.idempotency_key, "storage-contract-reset");
    assert_eq!(scheduled.attempt_count, 0);
    assert_eq!(scheduled.next_attempt_at_ms, fire_at_ms);

    store
        .defer_provider_reset("codex", "storage-contract-reset", fire_at_ms + 1_000)
        .await?;
    let deferred = store
        .load_provider_reset("codex")
        .await?
        .context("deferred provider action was not restored")?;
    assert_eq!(deferred.attempt_count, 1);
    assert_eq!(deferred.next_attempt_at_ms, fire_at_ms + 1_000);
    store
        .append_provider_action_log(
            "codex",
            "scheduled",
            "succeeded",
            "contract",
            "storage contract action",
            Some("credit-test"),
            Some("storage-contract-reset"),
            fire_at_ms,
        )
        .await?;
    let action_logs = store.provider_action_logs(10).await?;
    let action = action_logs
        .first()
        .context("provider action log was not restored")?;
    assert_eq!(action.status, "succeeded");
    assert_eq!(action.idempotency_suffix.as_deref(), Some("ct-reset"));
    let diagnostic_id = format!("automation:{}", action.id);
    assert!(store.diagnostic_log_detail(&diagnostic_id).await?.is_some());
    store.delete_provider_reset("codex").await?;
    assert!(store.load_provider_reset("codex").await?.is_none());

    let future_provider = "future-reset";
    store
        .upsert_provider_reset(
            future_provider,
            fire_at_ms + 2_000,
            "storage-contract-future-reset",
        )
        .await?;
    let future = store
        .load_provider_reset(future_provider)
        .await?
        .context("scheduled plugin-declared provider action was not restored")?;
    assert_eq!(future.fire_at_ms, fire_at_ms + 2_000);
    assert!(
        !store
            .claim_provider_reset(future_provider, "stale-storage-contract-key")
            .await?
    );
    store
        .append_provider_action_log(
            future_provider,
            "scheduled",
            "scheduled",
            "contract",
            "plugin-declared storage contract action",
            None,
            Some("storage-contract-future-reset"),
            fire_at_ms + 2_000,
        )
        .await?;
    let future_log = store
        .provider_action_logs(10)
        .await?
        .into_iter()
        .find(|log| log.provider == future_provider)
        .context("plugin-declared provider action log was not restored")?;
    assert_eq!(future_log.status, "scheduled");
    assert!(
        store
            .claim_provider_reset(future_provider, "storage-contract-future-reset")
            .await?
    );
    assert!(store.load_provider_reset(future_provider).await?.is_none());
    Ok(())
}

async fn assert_cache_lineage_contract(store: &Store) -> Result<()> {
    let occurred_at_ms = chrono::Utc::now().timestamp_millis();
    let mut hot = super::provider_usage_validation_tests::event();
    hot.sequence = 10;
    hot.occurred_at_ms = occurred_at_ms;
    hot.input_tokens = Some(10_000);
    hot.cache_hit_tokens = Some(9_000);
    hot.cache_miss_tokens = Some(1_000);

    // This healthy but tiny request is outside the diagnostic lineage. It
    // must not hide the prior eligible hot observation from the next miss.
    let mut ineligible = super::provider_usage_validation_tests::event();
    ineligible.sequence = 11;
    ineligible.occurred_at_ms = occurred_at_ms + 1;

    let mut cold = hot.clone();
    cold.sequence = 12;
    cold.occurred_at_ms = occurred_at_ms + 2;
    cold.cache_hit_tokens = Some(0);
    cold.cache_miss_tokens = Some(10_000);
    assert_eq!(
        store
            .ingest_provider_usage("hawk", "codex-deepseek", &[hot, ineligible, cold],)
            .await?,
        12
    );
    let logs = store
        .diagnostic_logs(&DiagnosticLogFilter {
            since_ms: occurred_at_ms.saturating_sub(1_000),
            until_ms: occurred_at_ms.saturating_add(1_000),
            kinds: vec!["cache_anomaly".to_owned()],
            severities: Vec::new(),
            states: Vec::new(),
            agents: Vec::new(),
            session_ref: None,
            cursor_ms: None,
            cursor_id: None,
            limit: 20,
        })
        .await?;
    let anomaly = logs
        .iter()
        .find(|log| log.id.ends_with(":12"))
        .context("eligible cache transition was not listed")?;
    assert_eq!(
        anomaly.classification.as_deref(),
        Some("unexpected_exact_prefix_miss")
    );
    assert!(store.diagnostic_log_detail(&anomaly.id).await?.is_some());
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn assert_product_user_contract(store: &Store) -> Result<()> {
    let created_at_ms = 1_900_000_000_000;
    let user = ProductUser {
        id: "0123456789abcdef0123456789abcdef".to_owned(),
        username: " Draven ".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash:
            "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$dGVzdGhhc2h2YWx1ZW5vdHJlYWw"
                .to_owned(),
        created_at_ms,
        updated_at_ms: created_at_ms,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await?;
    let loaded = store
        .user_by_username("DRAVEN")
        .await?
        .context("product user was not restored by username")?;
    assert_eq!(loaded.id, user.id);
    assert_eq!(loaded.username, "draven");
    assert_eq!(loaded.password_algo, "argon2id");
    assert_eq!(loaded.password_hash, user.password_hash);
    assert_eq!(loaded.disabled_at_ms, None);
    store
        .record_automation_credential_issued(
            &user.id,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "CI smoke",
            &["api:read".to_owned(), "websocket".to_owned()],
            created_at_ms + 300_000,
            created_at_ms,
        )
        .await?;
    assert!(
        store
            .record_automation_credential_issued(
                &user.id,
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "CI smoke",
                &["admin".to_owned()],
                created_at_ms + 300_000,
                created_at_ms,
            )
            .await
            .is_err(),
        "automation audit must reject unknown scopes"
    );
    assert_eq!(
        store.user_by_id(&user.id).await?.map(|row| row.username),
        Some("draven".to_owned())
    );
    let external_ceremony = crate::passkey::ExternalPasskeyCeremonyRecord {
        transaction_hash: "ef".repeat(32),
        ceremony_json: serde_json::json!({ "contract": true }).to_string(),
        expires_at_ms: created_at_ms + 300_000,
        created_at_ms,
    };
    store
        .upsert_external_passkey_ceremony(&external_ceremony)
        .await?;
    let restored_ceremony = store
        .external_passkey_ceremony(&external_ceremony.transaction_hash, created_at_ms + 1)
        .await?
        .context("external Passkey ceremony was not restored")?;
    // PostgreSQL jsonb normalizes whitespace/key order; the contract is
    // identical ceremony data, not an identical JSON serialization.
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&restored_ceremony.ceremony_json)?,
        serde_json::from_str::<serde_json::Value>(&external_ceremony.ceremony_json)?
    );
    assert_eq!(
        restored_ceremony.transaction_hash,
        external_ceremony.transaction_hash
    );
    assert_eq!(
        restored_ceremony.expires_at_ms,
        external_ceremony.expires_at_ms
    );
    assert_eq!(
        restored_ceremony.created_at_ms,
        external_ceremony.created_at_ms
    );
    assert!(
        store
            .external_passkey_ceremony(
                &external_ceremony.transaction_hash,
                external_ceremony.expires_at_ms,
            )
            .await?
            .is_none(),
        "expired external Passkey ceremonies must not be restored"
    );
    assert_eq!(store.list_users().await?.len(), 1);
    let initial_passkey_policy = store
        .user_passkey_policy(&user.id)
        .await?
        .context("new user Passkey policy was not restored")?;
    assert!(!initial_passkey_policy.enabled);
    assert_eq!(
        initial_passkey_policy.reauth_after_ms,
        crate::passkey::DEFAULT_PASSKEY_REAUTH_AFTER_MS
    );
    let duplicate = ProductUser {
        id: "fedcba9876543210fedcba9876543210".to_owned(),
        username: "DRAVEN".to_owned(),
        ..user.clone()
    };
    assert!(
        store
            .insert_user(&duplicate)
            .await
            .expect_err("duplicate username should fail")
            .to_string()
            .contains("account already exists")
    );

    let mut owned = session("sess-owned-contract");
    owned.owner_user_id = Some(user.id.clone());
    owned.owner_username = Some("ignored-client-username".to_owned());
    store.insert_session(&owned).await?;
    let restored_owned = store
        .load_all()
        .await?
        .into_iter()
        .find(|loaded| loaded.meta.id == owned.id)
        .context("owned session was not restored")?;
    assert_eq!(
        restored_owned.meta.owner_user_id.as_deref(),
        Some(user.id.as_str())
    );
    assert_eq!(
        restored_owned.meta.owner_username.as_deref(),
        Some("draven")
    );
    store.delete_session(&owned.id).await?;

    let session = ProductUserSession {
        token_hash: "aa".repeat(32),
        session_id: "01aa01aa01aa01aa01aa01aa01aa01aa".to_owned(),
        user_id: user.id.clone(),
        client_kind: "browser".to_owned(),
        principal_class: "human".to_owned(),
        created_at_ms,
        expires_at_ms: created_at_ms + 14 * 24 * 60 * 60 * 1_000,
        last_seen_at_ms: created_at_ms,
        user_agent: Some("CowboyContract/1.0".to_owned()),
        passkey_verified_at_ms: Some(created_at_ms),
        primary_authenticated_at_ms: created_at_ms,
        primary_auth_method: Some("password".to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: None,
        auth_issuer: None,
        auth_subject: None,
        auth_sid: None,
        id_token_ciphertext: None,
    };
    store.insert_user_session(&session).await?;
    let restored_session = store
        .user_session_by_token_hash(&session.token_hash)
        .await?
        .context("product user session was not restored")?;
    assert_eq!(restored_session.user_id, user.id);
    assert_eq!(
        restored_session.user_agent.as_deref(),
        Some("CowboyContract/1.0")
    );
    assert_eq!(restored_session.passkey_verified_at_ms, Some(created_at_ms));
    assert_eq!(restored_session.primary_authenticated_at_ms, created_at_ms);
    assert_eq!(
        restored_session.primary_auth_method.as_deref(),
        Some("password")
    );
    let replacement_session = ProductUserSession {
        token_hash: "ab".repeat(32),
        ..session.clone()
    };
    store
        .replace_user_session(&session.token_hash, &replacement_session)
        .await?;
    assert!(
        store
            .user_session_by_token_hash(&session.token_hash)
            .await?
            .is_none(),
        "primary authentication must invalidate the previous cookie"
    );
    assert_eq!(
        store
            .user_session_by_token_hash(&replacement_session.token_hash)
            .await?
            .context("replacement product session was not restored")?
            .primary_auth_method
            .as_deref(),
        Some("password")
    );
    store
        .replace_user_session(&replacement_session.token_hash, &session)
        .await?;
    assert_eq!(
        store
            .touch_user_session_activity(&session.token_hash, created_at_ms + 60_001)
            .await?,
        1
    );
    assert_eq!(
        store
            .touch_user_session_activity(&session.token_hash, created_at_ms + 60_002)
            .await?,
        0,
        "activity writes are coalesced to at most once per minute"
    );
    let rotated = ProductUserSession {
        token_hash: "cc".repeat(32),
        expires_at_ms: created_at_ms + 30 * 24 * 60 * 60 * 1_000,
        passkey_verified_at_ms: Some(created_at_ms + 1_000),
        ..session.clone()
    };
    assert!(
        store
            .rotate_user_session(&session.token_hash, &rotated)
            .await
            .expect_err("disabled Passkey refresh must reject a long session")
            .to_string()
            .contains("Passkey refresh is disabled")
    );
    assert!(
        store
            .user_session_by_token_hash(&session.token_hash)
            .await?
            .is_some()
    );
    store
        .insert_user_passkey(&crate::passkey::UserPasskey {
            id: "1234567890abcdef1234567890abcdef".to_owned(),
            user_id: user.id.clone(),
            credential_id: "contract-credential".to_owned(),
            nickname: "Contract Passkey".to_owned(),
            passkey_json: "{}".to_owned(),
            created_at_ms,
            last_used_at_ms: Some(created_at_ms),
        })
        .await?;
    store
        .set_user_passkey_reauth(&user.id, true, 4 * 60 * 60 * 1_000)
        .await?;
    let short_passkey_policy = store
        .user_passkey_policy(&user.id)
        .await?
        .context("short Passkey policy was not restored")?;
    assert!(short_passkey_policy.enabled);
    assert_eq!(short_passkey_policy.reauth_after_ms, 4 * 60 * 60 * 1_000);
    for interval in crate::passkey::PASSKEY_REAUTH_INTERVALS_MS {
        store
            .set_user_passkey_reauth(&user.id, true, interval)
            .await?;
        assert_eq!(
            store
                .user_passkey_policy(&user.id)
                .await?
                .context("Passkey verification interval was not restored")?
                .reauth_after_ms,
            interval
        );
    }
    store
        .rotate_user_session(&session.token_hash, &rotated)
        .await?;
    assert!(
        store
            .user_session_by_token_hash(&session.token_hash)
            .await?
            .is_none()
    );
    let shortened_expiry = created_at_ms + 24 * 60 * 60 * 1_000;
    assert_eq!(
        store
            .cap_user_session_expiry(&user.id, shortened_expiry)
            .await?,
        1
    );
    let shortened = store
        .user_session_by_token_hash(&rotated.token_hash)
        .await?
        .context("rotated product user session was not restored")?;
    assert_eq!(shortened.expires_at_ms, shortened_expiry);
    assert_eq!(
        shortened.passkey_verified_at_ms,
        rotated.passkey_verified_at_ms
    );
    assert_eq!(
        shortened.primary_authenticated_at_ms,
        rotated.primary_authenticated_at_ms
    );
    assert_eq!(store.delete_user_sessions_for_user(&user.id).await?, 1);

    let token = ProductApiToken {
        id: "abcdef0123456789abcdef0123456789".to_owned(),
        user_id: user.id.clone(),
        name: "zed".to_owned(),
        token_prefix: "cow_abcd".to_owned(),
        token_hash: "bb".repeat(32),
        created_at_ms,
        expires_at_ms: Some(created_at_ms + 365 * 24 * 60 * 60 * 1_000),
        last_used_at_ms: None,
        revoked_at_ms: None,
    };
    store.insert_user_api_token(&token).await?;
    let restored_token = store
        .user_api_token_by_hash(&token.token_hash)
        .await?
        .context("product API token was not restored")?;
    assert_eq!(restored_token.name, "zed");
    assert_eq!(restored_token.token_prefix, "cow_abcd");
    assert_eq!(restored_token.expires_at_ms, token.expires_at_ms);
    let listed = store.list_user_api_tokens_for_user(&user.id).await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, token.id);
    assert_eq!(
        store
            .touch_user_api_token_last_used(&token.id, created_at_ms)
            .await?,
        1
    );
    assert_eq!(
        store
            .touch_user_api_token_last_used(&token.id, created_at_ms + 1)
            .await?,
        0
    );
    let touched = store
        .user_api_token_by_hash(&token.token_hash)
        .await?
        .context("touched token")?;
    assert_eq!(touched.last_used_at_ms, Some(created_at_ms));
    assert_eq!(
        store
            .revoke_user_api_token_for_user("missing-user", &token.id)
            .await?,
        0
    );
    assert_eq!(
        store
            .revoke_user_api_token_for_user(&user.id, &token.id)
            .await?,
        1
    );
    assert!(
        store
            .list_user_api_tokens_for_user(&user.id)
            .await?
            .is_empty()
    );
    store
        .insert_user_api_token(&ProductApiToken {
            id: "fedcba9876543210fedcba9876543210".to_owned(),
            token_hash: "cc".repeat(32),
            last_used_at_ms: None,
            revoked_at_ms: None,
            ..token.clone()
        })
        .await?;
    assert_eq!(store.revoke_user_api_tokens_for_user(&user.id).await?, 1);

    let signing_key = crate::client_auth::new_signing_key()?;
    let device = ProductDevice {
        id: "deafbeefdeafbeefdeafbeefdeafbeef".to_owned(),
        user_id: user.id.clone(),
        name: "Zed on Hawk".to_owned(),
        public_key: crate::client_auth::public_key_to_base64(&signing_key),
        created_at_ms,
        last_used_at_ms: None,
        revoked_at_ms: None,
    };
    let refresh = ProductDeviceRefreshToken {
        token_hash: "dd".repeat(32),
        device_id: device.id.clone(),
        family_id: "facefeedfacefeedfacefeedfacefeed".to_owned(),
        created_at_ms,
        expires_at_ms: created_at_ms + crate::client_auth::REFRESH_TOKEN_TTL_MS,
        used_at_ms: None,
        revoked_at_ms: None,
    };
    store.insert_user_device(&device, &refresh).await?;
    assert_eq!(
        store.list_user_devices_for_user(&user.id).await?,
        vec![device.clone()]
    );
    let (restored_device, restored_refresh) = store
        .user_device_refresh_by_hash(&refresh.token_hash)
        .await?
        .context("device refresh token was not restored")?;
    assert_eq!(restored_device, device);
    assert_eq!(restored_refresh, refresh);
    assert_eq!(
        store
            .touch_user_device_last_used(&device.id, created_at_ms + 1)
            .await?,
        1
    );
    assert_eq!(
        store
            .touch_user_device_last_used(&device.id, created_at_ms + 2)
            .await?,
        0
    );
    let replacement = ProductDeviceRefreshToken {
        token_hash: "ee".repeat(32),
        created_at_ms: created_at_ms + 3,
        ..refresh.clone()
    };
    assert!(
        store
            .rotate_user_device_refresh(&refresh.token_hash, &replacement, created_at_ms + 3,)
            .await?
    );
    assert!(
        !store
            .rotate_user_device_refresh(&refresh.token_hash, &replacement, created_at_ms + 4,)
            .await?
    );
    assert_eq!(
        store
            .revoke_user_device_for_user("missing-user", &device.id, created_at_ms + 5)
            .await?,
        0
    );
    assert_eq!(
        store
            .revoke_user_device_for_user(&user.id, &device.id, created_at_ms + 5)
            .await?,
        1
    );
    assert!(store.list_user_devices_for_user(&user.id).await?.is_empty());
    let (revoked_device, revoked_refresh) = store
        .user_device_refresh_by_hash(&replacement.token_hash)
        .await?
        .context("revoked device refresh token was not retained for replay detection")?;
    assert_eq!(revoked_device.revoked_at_ms, Some(created_at_ms + 5));
    assert_eq!(revoked_refresh.revoked_at_ms, Some(created_at_ms + 5));

    let admitted_device = ProductDevice {
        id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        created_at_ms: created_at_ms + 6,
        last_used_at_ms: Some(created_at_ms + 6),
        revoked_at_ms: None,
        ..device.clone()
    };
    let admitted_refresh = ProductDeviceRefreshToken {
        token_hash: "ab".repeat(32),
        device_id: admitted_device.id.clone(),
        family_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
        created_at_ms: created_at_ms + 6,
        expires_at_ms: created_at_ms + crate::client_auth::REFRESH_TOKEN_TTL_MS,
        used_at_ms: None,
        revoked_at_ms: None,
    };
    assert!(matches!(
        store
            .admit_user_device(&admitted_device, &admitted_refresh, 1, true)
            .await?,
        ProductDeviceAdmission::Admitted {
            active_clients: 1,
            observed_overflow: false,
            ..
        }
    ));
    let blocked_device = ProductDevice {
        id: "cccccccccccccccccccccccccccccccc".to_owned(),
        created_at_ms: created_at_ms + 7,
        last_used_at_ms: Some(created_at_ms + 7),
        ..admitted_device.clone()
    };
    let blocked_refresh = ProductDeviceRefreshToken {
        token_hash: "cd".repeat(32),
        device_id: blocked_device.id.clone(),
        family_id: "dddddddddddddddddddddddddddddddd".to_owned(),
        created_at_ms: created_at_ms + 7,
        ..admitted_refresh.clone()
    };
    assert!(matches!(
        store
            .admit_user_device(&blocked_device, &blocked_refresh, 1, true)
            .await?,
        ProductDeviceAdmission::CapacityFull {
            active_clients: 1,
            limit: 1,
        }
    ));
    assert_eq!(store.list_user_devices_for_user(&user.id).await?.len(), 1);
    assert_eq!(
        store
            .revoke_user_device_for_user(&user.id, &admitted_device.id, created_at_ms + 8,)
            .await?,
        1
    );

    let capacity_claim = |client_id: &str| ActiveClientClaim {
        client_id: client_id.to_owned(),
        user_id: Some(user.id.clone()),
        principal_class: "human".to_owned(),
        session_id: Some(rotated.session_id.clone()),
        client_kind: "browser".to_owned(),
        now_ms: created_at_ms,
        lease_ms: 120_000,
        reservation_ms: 30_000,
        per_user_limit: 1,
        service_limit: 1,
        automation_limit: 1,
        enforce: true,
    };
    let first = store
        .claim_active_client(&capacity_claim("client-aaaaaaaaaaaaaaaa"))
        .await?;
    let first_fence = match first {
        ActiveClientAdmission::Granted {
            lease, capacity, ..
        } => {
            assert_eq!(capacity.active_for_user, 1);
            assert_eq!(capacity.active_for_service, 1);
            lease.fencing_token
        }
        ActiveClientAdmission::Waiting { .. } => {
            anyhow::bail!("first active client unexpectedly waited")
        }
    };
    let renewed = store
        .claim_active_client(&capacity_claim("client-aaaaaaaaaaaaaaaa"))
        .await?;
    assert!(matches!(
        renewed,
        ActiveClientAdmission::Granted { ref lease, .. }
            if lease.fencing_token == first_fence
    ));
    let waiting = store
        .claim_active_client(&capacity_claim("client-bbbbbbbbbbbbbbbb"))
        .await?;
    assert!(matches!(
        waiting,
        ActiveClientAdmission::Waiting { position: 1, .. }
    ));
    assert!(
        store
            .release_active_client(&ActiveClientRelease {
                client_id: "client-aaaaaaaaaaaaaaaa",
                fencing_token: first_fence,
                reason: "contract_release",
                now_ms: created_at_ms + 1,
                reservation_ms: 30_000,
                per_user_limit: 1,
            })
            .await?
    );
    let second = store
        .claim_active_client(&ActiveClientClaim {
            now_ms: created_at_ms + 2,
            ..capacity_claim("client-bbbbbbbbbbbbbbbb")
        })
        .await?;
    let second_fence = match second {
        ActiveClientAdmission::Granted { lease, .. } => lease.fencing_token,
        ActiveClientAdmission::Waiting { .. } => {
            anyhow::bail!("reserved active client did not acquire the released seat")
        }
    };
    assert_eq!(
        store
            .list_active_clients_for_user(&user.id, created_at_ms + 2)
            .await?
            .len(),
        1
    );
    assert!(
        store
            .heartbeat_active_client(
                "client-bbbbbbbbbbbbbbbb",
                second_fence,
                created_at_ms + 3,
                120_000,
            )
            .await?
    );
    assert!(
        store
            .release_active_client(&ActiveClientRelease {
                client_id: "client-bbbbbbbbbbbbbbbb",
                fencing_token: second_fence,
                reason: "contract_release",
                now_ms: created_at_ms + 4,
                reservation_ms: 30_000,
                per_user_limit: 1,
            })
            .await?
    );
    let reacquired = store
        .claim_active_client(&ActiveClientClaim {
            now_ms: created_at_ms + 5,
            ..capacity_claim("client-bbbbbbbbbbbbbbbb")
        })
        .await?;
    let reacquired_fence = match reacquired {
        ActiveClientAdmission::Granted { lease, .. } => lease.fencing_token,
        ActiveClientAdmission::Waiting { .. } => {
            anyhow::bail!("released active client did not reacquire")
        }
    };
    assert!(reacquired_fence > second_fence);
    assert!(
        !store
            .heartbeat_active_client(
                "client-bbbbbbbbbbbbbbbb",
                second_fence,
                created_at_ms + 6,
                120_000,
            )
            .await?,
        "a stale fencing token must never extend a replacement lease"
    );
    assert!(
        !store
            .release_active_client_for_user(
                "another-user",
                &ActiveClientRelease {
                    client_id: "client-bbbbbbbbbbbbbbbb",
                    fencing_token: reacquired_fence,
                    reason: "user_reclaimed",
                    now_ms: created_at_ms + 7,
                    reservation_ms: 30_000,
                    per_user_limit: 1,
                },
            )
            .await?,
        "an account cannot reclaim another account's active client"
    );
    assert!(
        !store
            .release_active_client_for_user(
                &user.id,
                &ActiveClientRelease {
                    client_id: "client-bbbbbbbbbbbbbbbb",
                    fencing_token: second_fence,
                    reason: "user_reclaimed",
                    now_ms: created_at_ms + 7,
                    reservation_ms: 30_000,
                    per_user_limit: 1,
                },
            )
            .await?,
        "a stale UI cannot reclaim a replacement lease"
    );
    assert!(
        store
            .release_active_client_for_user(
                &user.id,
                &ActiveClientRelease {
                    client_id: "client-bbbbbbbbbbbbbbbb",
                    fencing_token: reacquired_fence,
                    reason: "user_reclaimed",
                    now_ms: created_at_ms + 7,
                    reservation_ms: 30_000,
                    per_user_limit: 1,
                },
            )
            .await?
    );

    let password_session = ProductUserSession {
        session_id: "session-password-contract".to_owned(),
        token_hash: "12".repeat(32),
        ..rotated.clone()
    };
    store.insert_user_session(&password_session).await?;
    let password_token = ProductApiToken {
        id: "11111111111111111111111111111111".to_owned(),
        token_hash: "13".repeat(32),
        revoked_at_ms: None,
        ..token.clone()
    };
    store.insert_user_api_token(&password_token).await?;
    let password_device = ProductDevice {
        id: "22222222222222222222222222222222".to_owned(),
        name: "Cowboy CLI on Hawk".to_owned(),
        revoked_at_ms: None,
        ..device.clone()
    };
    let password_refresh = ProductDeviceRefreshToken {
        token_hash: "14".repeat(32),
        device_id: password_device.id.clone(),
        family_id: "33333333333333333333333333333333".to_owned(),
        used_at_ms: None,
        revoked_at_ms: None,
        ..refresh.clone()
    };
    store
        .insert_user_device(&password_device, &password_refresh)
        .await?;
    store
        .update_user_password(&user.id, "argon2id", "replacement-hash")
        .await?;
    assert_eq!(
        store
            .user_by_id(&user.id)
            .await?
            .map(|row| row.password_hash),
        Some("replacement-hash".to_owned())
    );
    assert!(
        store
            .user_session_by_token_hash(&password_session.token_hash)
            .await?
            .is_none()
    );
    assert!(
        store
            .list_user_api_tokens_for_user(&user.id)
            .await?
            .is_empty()
    );
    assert!(store.list_user_devices_for_user(&user.id).await?.is_empty());
    let (password_revoked_device, password_revoked_refresh) = store
        .user_device_refresh_by_hash(&password_refresh.token_hash)
        .await?
        .context("password-revoked device refresh token was not retained")?;
    assert!(password_revoked_device.revoked_at_ms.is_some());
    assert!(password_revoked_refresh.revoked_at_ms.is_some());

    store
        .set_user_disabled_at(&user.id, Some(created_at_ms + 1))
        .await?;
    assert_eq!(
        store
            .user_by_id(&user.id)
            .await?
            .and_then(|row| row.disabled_at_ms),
        Some(created_at_ms + 1)
    );
    store.delete_user(&user.id).await?;
    assert!(store.user_by_username("draven").await?.is_none());
    assert!(
        store
            .user_api_token_by_hash(&token.token_hash)
            .await?
            .is_none()
    );
    Ok(())
}

#[allow(clippy::too_many_lines)] // one shared end-to-end contract for every storage backend
async fn run_storage_contract(store: &Store, session_id: &str) -> Result<()> {
    store.migrate().await?;
    assert_machine_contract(store).await?;
    assert_provider_action_contract(store).await?;
    assert_product_user_contract(store).await?;
    let mut pinned_session = session(session_id);
    pinned_session.provider_account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    store.insert_session(&pinned_session).await?;
    let companion_id = format!("{session_id}-artifact");
    store.insert_session(&session(&companion_id)).await?;
    let numeric_id = session_id
        .strip_prefix("sess-")
        .context("storage contract needs a numeric session id")?
        .parse::<u64>()?;
    assert_eq!(store.next_session_number().await?, numeric_id + 1);
    store.update_status(session_id, Status::Running).await?;
    store
        .update_agent_session_id(session_id, Some("agent-thread"))
        .await?;
    store
        .update_config_options(session_id, &serde_json::json!({"model": "gpt-test"}))
        .await?;
    store
        .update_config_preferences(session_id, &serde_json::json!({"model": "gpt-test"}))
        .await?;
    store
        .update_title(session_id, "Storage contract renamed")
        .await?;
    store
        .update_cwd(
            session_id,
            "/tmp/cowboy-retargeted",
            Some("Storage contract retargeted"),
        )
        .await?;
    store
        .update_mobile_review_state(
            session_id,
            &serde_json::json!({"mode": "code", "tabs": ["src/store.rs"]}),
        )
        .await?;
    store
        .update_pending(
            session_id,
            &[QueuedMessage {
                id: "queue-1".to_owned(),
                text: "queued contract prompt".to_owned(),
                content: Vec::new(),
                cmid: Some("contract-cmid".to_owned()),
                schedule: None,
            }],
            &[QueuedMessage {
                id: "draft-1".to_owned(),
                text: "draft contract prompt".to_owned(),
                content: Vec::new(),
                cmid: None,
                schedule: None,
            }],
        )
        .await?;
    store
        .upsert_wakeup(session_id, 1_900_000_000_000, "continue")
        .await?;
    let events = vec![
        Envelope {
            session_id: session_id.to_owned(),
            seq: 0,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "user_message_chunk",
                    "content": {"text": "verify both storage backends"},
                }),
            },
            cmid: None,
        },
        Envelope {
            session_id: session_id.to_owned(),
            seq: 1,
            event: Event::TurnEnd {
                usage: None,
                stop_reason: "end_turn".to_owned(),
            },
            cmid: None,
        },
    ];
    store
        .upsert_event_batch(&events, &HashMap::from([(session_id.to_owned(), 2)]))
        .await?;

    let artifact_bytes = vec![7_u8; 40_000];
    let artifact_name = format!("{}.png", hex_sha256(&artifact_bytes));
    let artifact_event = Envelope {
        session_id: companion_id.clone(),
        seq: 0,
        event: Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {
                    "type": "image",
                    "data": base64::engine::general_purpose::STANDARD.encode(artifact_bytes),
                    "mimeType": "image/png",
                },
            }),
        },
        cmid: None,
    };
    store
        .upsert_event_batch(
            &[artifact_event],
            &HashMap::from([(companion_id.clone(), 1)]),
        )
        .await?;
    assert!(store.artifact_path(&artifact_name).is_some());
    store
        .update_session_order(&[companion_id.clone(), session_id.to_owned()])
        .await?;
    let folder = |id: &str, parent: Option<&str>, position: i64| SessionFolder {
        id: id.to_owned(),
        owner_user_id: None,
        name: format!("Folder {id}"),
        parent: parent.map(str::to_owned),
        position,
        project: (id == "f-root").then(|| "cowboy".to_owned()),
        defaults: crate::session_folders::defaults::parse(
            &serde_json::json!({"machine":"hawk","preset":"claude-code/opus-default"}),
        )
        .unwrap(),
        trashed_at_ms: None,
    };
    store
        .replace_session_folders(
            None,
            &[
                folder("f-root", None, 0),
                folder("f-child", Some("f-root"), 0),
                folder("f-stale", None, 1),
            ],
        )
        .await?;
    let mut renamed = folder("f-child", Some("f-root"), 0);
    renamed.name = "Child renamed".to_owned();
    renamed.trashed_at_ms = Some(1_700_000_000_000);
    // A whole-set write drops the owner's rows that left the set and only
    // touches that owner's slot.
    store
        .replace_session_folders(None, &[folder("f-root", None, 0), renamed])
        .await?;
    store
        .replace_session_folders(
            Some("user-b"),
            &[SessionFolder {
                owner_user_id: Some("user-b".to_owned()),
                ..folder("f-b", None, 0)
            }],
        )
        .await?;
    store
        .update_session_placement(&[
            (session_id.to_owned(), Some("f-child".to_owned())),
            (companion_id.clone(), None),
        ])
        .await?;
    store
        .set_sessions_trashed(&[session_id.to_owned()], Some(1_700_000_000_001))
        .await?;

    let loaded = store.load_all().await?;
    assert_eq!(
        loaded.first().map(|loaded| loaded.meta.id.as_str()),
        Some(companion_id.as_str())
    );
    let restored = loaded
        .iter()
        .find(|loaded| loaded.meta.id == session_id)
        .context("storage contract session was not restored")?;
    assert_eq!(restored.meta.status, Status::Running);
    assert_eq!(
        restored.meta.agent_session_id.as_deref(),
        Some("agent-thread")
    );
    assert!(restored.meta.provider_auth_generation.is_none());
    assert_eq!(
        restored.meta.provider_account_id,
        pinned_session.provider_account_id
    );
    assert_eq!(restored.meta.title, "Storage contract retargeted");
    assert_eq!(restored.meta.cwd, "/tmp/cowboy-retargeted");
    assert!(restored.meta.owner_user_id.is_none());
    assert!(restored.meta.owner_username.is_none());
    assert_eq!(restored.events.len(), 2);
    assert_eq!(restored.next_seq, 2);
    // The newest event's wall-clock time restores the idle clock.
    let written_at = restored
        .last_event_at_ms
        .context("restore lost the newest event time")?;
    let now = chrono::Utc::now().timestamp_millis();
    assert!(written_at <= now + 1_000 && written_at > now - 10 * 60 * 1_000);
    assert_eq!(restored.queue[0].id, "queue-1");
    assert_eq!(restored.drafts[0].id, "draft-1");
    assert_eq!(
        restored.config_options.as_ref().unwrap()["model"],
        "gpt-test"
    );
    assert_eq!(restored.config_preferences["model"], "gpt-test");
    assert_eq!(restored.mobile_review_state["mode"], "code");
    assert_eq!(restored.folder_id.as_deref(), Some("f-child"));
    assert_eq!(restored.meta.trashed_at_ms, Some(1_700_000_000_001));
    assert!(
        loaded
            .iter()
            .find(|loaded| loaded.meta.id == companion_id)
            .is_some_and(|companion| companion.folder_id.is_none())
    );
    let mut folders = store.load_session_folders().await?;
    folders.sort_by(|a, b| a.id.cmp(&b.id));
    let folder_ids: Vec<&str> = folders.iter().map(|folder| folder.id.as_str()).collect();
    assert_eq!(folder_ids, vec!["f-b", "f-child", "f-root"]);
    assert_eq!(folders[0].owner_user_id.as_deref(), Some("user-b"));
    assert_eq!(folders[1].name, "Child renamed");
    assert_eq!(folders[1].parent.as_deref(), Some("f-root"));
    assert_eq!(folders[1].trashed_at_ms, Some(1_700_000_000_000));
    assert!(folders[2].trashed_at_ms.is_none());
    assert!(folders[2].owner_user_id.is_none());
    assert_eq!(folders[2].project.as_deref(), Some("cowboy"));
    assert_eq!(folders[2].defaults, folder("f-root", None, 0).defaults);

    // Simulate a newer writer, then rename with this older reader. Unknown
    // keys must neither fail startup nor disappear from durable storage.
    let mut future = serde_json::to_value(&folders[2].defaults)?;
    future["future_directory"] = serde_json::json!({"path":"/next/checkout"});
    macro_rules! inject_future {
        ($db:expr) => {
            sqlx::query("UPDATE session_folders SET defaults_json=$1 WHERE id='f-root'")
                .bind(future.to_string())
                .execute(&$db.pool)
                .await?;
        };
    }
    match &store.backend {
        StorageBackend::Postgres(db) => {
            inject_future!(db);
        }
        StorageBackend::Sqlite(db) => {
            inject_future!(db);
        }
    }
    let mut older = store
        .load_session_folders()
        .await?
        .into_iter()
        .filter(|f| f.owner_user_id.is_none())
        .collect::<Vec<_>>();
    older.iter_mut().find(|f| f.id == "f-root").unwrap().name = "Older reader rename".into();
    store.replace_session_folders(None, &older).await?;
    macro_rules! read_future {
        ($db:expr) => {
            sqlx::query_scalar::<_, String>(
                "SELECT defaults_json FROM session_folders WHERE id='f-root'",
            )
            .fetch_one(&$db.pool)
            .await?
        };
    }
    let saved = match &store.backend {
        StorageBackend::Postgres(db) => read_future!(db),
        StorageBackend::Sqlite(db) => read_future!(db),
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&saved)?["future_directory"],
        future["future_directory"]
    );

    let mut reloaded = restored.meta.clone();
    reloaded.provider_version = "new-version".to_owned();
    reloaded.provider_generation_digest = "new-digest".to_owned();
    reloaded.provider_behavior = Some(crate::provider::legacy_behavior("codex"));
    reloaded.status = Status::Starting;
    // The persistence operation must never overwrite the credential home
    // or native context, even if its input contains unrelated stale fields.
    reloaded.provider_auth_generation = Some(999);
    reloaded.provider_account_id = crate::provider_account::ProviderAccountId::default();
    reloaded.agent_session_id = Some("must-not-replace".to_owned());
    store.reload_provider(&reloaded).await?;
    let after_reload = store
        .load_all()
        .await?
        .into_iter()
        .find(|row| row.meta.id == session_id)
        .unwrap();
    assert_eq!(after_reload.meta.provider_version, "new-version");
    assert_eq!(after_reload.meta.provider_generation_digest, "new-digest");
    assert_eq!(
        after_reload.meta.provider_behavior,
        reloaded.provider_behavior
    );
    assert_eq!(after_reload.meta.status, Status::Starting);
    assert_eq!(
        after_reload.meta.agent_session_id,
        restored.meta.agent_session_id
    );
    assert_eq!(
        after_reload.meta.provider_auth_generation,
        restored.meta.provider_auth_generation
    );
    assert_eq!(after_reload.meta.cwd, restored.meta.cwd);
    assert_eq!(
        after_reload.meta.provider_account_id,
        restored.meta.provider_account_id
    );
    assert_eq!(after_reload.meta.title, restored.meta.title);
    assert_eq!(after_reload.config_preferences, restored.config_preferences);
    assert_eq!(after_reload.queue[0].id, "queue-1");
    assert_eq!(after_reload.drafts[0].id, "draft-1");
    assert_eq!(after_reload.events.len(), restored.events.len());

    let (history, _, reached_start) = store.history_page(session_id, 2, 200).await?;
    assert_eq!(history.len(), 2);
    assert!(reached_start);
    let (questions, _, total) = store.question_page_summaries(session_id, None, 20).await?;
    assert_eq!(total, 1);
    assert_eq!(questions.len(), 1);
    assert!(questions[0].title.contains("verify both storage backends"));
    let page = store
        .question_page_at(session_id, 0)
        .await?
        .context("question page missing")?;
    assert_eq!(page.len(), 2);
    let (previous_page, previous_cursor, previous_reached_start) =
        store.question_page_before(session_id, 2).await?;
    assert_eq!(previous_page.len(), 2);
    assert!(previous_cursor.is_none());
    assert!(previous_reached_start);

    assert!(
        store
            .load_wakeups()
            .await?
            .iter()
            .any(|(id, _, prompt)| id == session_id && prompt == "continue")
    );
    store.delete_wakeup(session_id).await?;
    assert!(
        store
            .load_wakeups()
            .await?
            .iter()
            .all(|(id, _, _)| id != session_id)
    );

    let incident_id = format!("storage-contract:{session_id}");
    store
        .upsert_runtime_incident(&RuntimeIncidentWrite {
            id: incident_id.clone(),
            occurred_at_ms: 1_900_000_000_000,
            source: "controller".to_owned(),
            classification: "storage_contract".to_owned(),
            severity: "warning".to_owned(),
            state: "active".to_owned(),
            summary: "storage contract incident".to_owned(),
            fingerprint: "contract".to_owned(),
            session_id: Some(session_id.to_owned()),
            client_id: None,
            machine_id: None,
            trace_id: None,
            build: Some("test".to_owned()),
            evidence_start_ms: 1_899_999_999_000,
            evidence_end_ms: 1_900_000_001_000,
            detail: serde_json::json!({"contract": true}),
        })
        .await?;
    store
        .upsert_runtime_incident(&RuntimeIncidentWrite {
            id: incident_id.clone(),
            occurred_at_ms: 1_900_000_000_000,
            source: "controller".to_owned(),
            classification: "storage_contract".to_owned(),
            severity: "warning".to_owned(),
            state: "active".to_owned(),
            summary: "storage contract incident".to_owned(),
            fingerprint: "contract".to_owned(),
            session_id: Some(session_id.to_owned()),
            client_id: None,
            machine_id: None,
            trace_id: None,
            build: Some("test".to_owned()),
            evidence_start_ms: 1_899_999_999_000,
            evidence_end_ms: 1_900_000_002_000,
            detail: serde_json::json!({"contract": null, "second": true}),
        })
        .await?;
    let incident = store
        .runtime_incidents(20)
        .await?
        .into_iter()
        .find(|incident| incident.id == incident_id)
        .context("runtime incident was not restored")?;
    assert_eq!(
        incident.detail.get("contract"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(incident.detail["second"], true);
    assert_eq!(incident.evidence_end_ms, 1_900_000_002_000);
    let logs = store
        .diagnostic_logs(&DiagnosticLogFilter {
            since_ms: 1_899_999_000_000,
            until_ms: 1_900_001_000_000,
            kinds: Vec::new(),
            severities: Vec::new(),
            states: Vec::new(),
            agents: Vec::new(),
            session_ref: Some(session_id.to_owned()),
            cursor_ms: None,
            cursor_id: None,
            limit: 20,
        })
        .await?;
    let log_id = format!("runtime:{incident_id}");
    assert!(logs.iter().any(|log| log.id == log_id));
    assert!(store.diagnostic_log_detail(&log_id).await?.is_some());
    assert_eq!(
        store
            .recover_runtime_incident(session_id, 1_900_000_002_000, "contract complete")
            .await?,
        1
    );
    let recovered = store
        .runtime_incidents(20)
        .await?
        .into_iter()
        .find(|incident| incident.id == incident_id)
        .context("recovered incident was not restored")?;
    assert_eq!(recovered.state, "recovered");
    assert_eq!(
        recovered.recovery_outcome.as_deref(),
        Some("contract complete")
    );

    let mut usage = super::provider_usage_validation_tests::event();
    usage.sequence = 1;
    usage.occurred_at_ms = chrono::Utc::now().timestamp_millis();
    usage.search_requests = Some(2);
    usage.fetch_requests = Some(1);
    store
        .ingest_provider_usage("hawk", "codex-deepseek", &[usage.clone()])
        .await?;
    // Replaying the same producer sequence is idempotent, and the producer
    // watermark remains monotonic.
    assert_eq!(
        store
            .ingest_provider_usage("hawk", "codex-deepseek", &[usage.clone()])
            .await?,
        1
    );
    let mut failed_usage = super::provider_usage_validation_tests::event();
    failed_usage.sequence = 2;
    failed_usage.occurred_at_ms = usage.occurred_at_ms;
    failed_usage.status = 401;
    failed_usage.completed = Some(false);
    assert_eq!(
        store
            .ingest_provider_usage("hawk", "codex-deepseek", &[failed_usage])
            .await?,
        2
    );
    let usage_summary = store.provider_usage_summary("deepseek", 1, 30).await?;
    assert_eq!(usage_summary["summary"]["requests"], 2);
    assert_eq!(usage_summary["summary"]["errors"], 1);
    assert_eq!(usage_summary["summary"]["inputTokens"], 20);
    assert_eq!(usage_summary["summary"]["searchRequests"], 2);
    assert_eq!(usage_summary["summary"]["fetchRequests"], 1);
    let to_ms = chrono::Utc::now().timestamp_millis().saturating_add(1_000);
    let usage_activity = store
        .provider_usage_activity(
            "deepseek",
            to_ms.saturating_sub(60_000),
            to_ms,
            &["codex".to_owned()],
            &["flash".to_owned()],
        )
        .await?;
    assert_eq!(usage_activity["summary"]["requests"], 2);
    let usage_logs = store
        .diagnostic_logs(&DiagnosticLogFilter {
            since_ms: to_ms.saturating_sub(60_000),
            until_ms: to_ms,
            kinds: vec!["provider_error".to_owned()],
            severities: Vec::new(),
            states: Vec::new(),
            agents: Vec::new(),
            session_ref: None,
            cursor_ms: None,
            cursor_id: None,
            limit: 20,
        })
        .await?;
    let usage_log = usage_logs
        .iter()
        .find(|log| log.kind == "provider_error")
        .context("provider error diagnostic was not listed")?;
    assert!(store.diagnostic_log_detail(&usage_log.id).await?.is_some());
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(store.purge_provider_usage(0).await? >= 2);
    assert_cache_lineage_contract(store).await?;
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(store.purge_provider_usage(0).await? >= 3);

    let (_, event_count, _) = store.storage_metrics().await?;
    assert!(event_count >= 2);
    store.clear_events(&companion_id, i64::MAX).await?;
    assert!(
        store
            .history_page(&companion_id, 1, 200)
            .await?
            .0
            .is_empty()
    );
    store.delete_session(session_id).await?;
    store.delete_session(&companion_id).await?;
    let (_, _, deleted_count) = store.storage_metrics().await?;
    assert!(deleted_count >= 2);
    assert!(
        store
            .load_all()
            .await?
            .iter()
            .all(|loaded| loaded.meta.id != session_id)
    );
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(store.purge_deleted(0).await? >= 2);
    Ok(())
}

async fn assert_restore_hot_tail_is_byte_bounded(store: &Store, session_id: &str) -> Result<()> {
    store.migrate().await?;
    store.insert_session(&session(session_id)).await?;
    let payload = "x".repeat(384 * 1024);
    let events: Vec<_> = (0..5)
        .map(|seq| Envelope {
            session_id: session_id.to_owned(),
            seq,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "payload": payload,
                }),
            },
            cmid: None,
        })
        .collect();
    store
        .upsert_event_batch(&events, &HashMap::from([(session_id.to_owned(), 5)]))
        .await?;

    let loaded = store
        .load_all()
        .await?
        .into_iter()
        .find(|loaded| loaded.meta.id == session_id)
        .context("byte-bounded restore session missing")?;
    assert_eq!(loaded.event_count, 5);
    assert!(!loaded.reached_start);
    assert_eq!(loaded.events.last().map(|event| event.seq), Some(4));
    assert!(loaded.events.len() < events.len());
    let restored_bytes = loaded.events.iter().fold(0usize, |size, event| {
        size.saturating_add(crate::core::estimated_envelope_bytes(event))
    });
    assert!(restored_bytes <= crate::core::HOT_TAIL_MAX_BYTES);
    Ok(())
}

#[tokio::test]
async fn sqlite_implements_storage_contract() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-sqlite-contract-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let url = format!("sqlite://{}", root.join("cowboy.sqlite3").display());
    let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
    Box::pin(run_storage_contract(&store, "sess-900000001"))
        .await
        .unwrap();
    assert_restore_hot_tail_is_byte_bounded(&store, "sess-900000011")
        .await
        .unwrap();
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sqlite_memory_implements_storage_contract() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-sqlite-memory-contract-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    Box::pin(run_storage_contract(&store, "sess-900000003"))
        .await
        .unwrap();
    assert_restore_hot_tail_is_byte_bounded(&store, "sess-900000013")
        .await
        .unwrap();
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[allow(clippy::too_many_lines)] // end-to-end provider scoping, fencing, and replay contract
#[tokio::test]
async fn sqlite_oidc_logout_is_provider_scoped_fenced_and_replay_safe() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-sqlite-oidc-logout-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let now = 1_900_000_000_000_i64;
    let user = ProductUser {
        id: "0123456789abcdef0123456789abcdef".to_owned(),
        username: "draven".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: "test-hash".to_owned(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let make_session = |token_byte: &str,
                        session_id: &str,
                        provider: Option<&str>,
                        issuer: Option<&str>,
                        subject: Option<&str>,
                        sid: Option<&str>| ProductUserSession {
        token_hash: token_byte.repeat(64),
        session_id: session_id.to_owned(),
        user_id: user.id.clone(),
        client_kind: "browser".to_owned(),
        principal_class: "human".to_owned(),
        created_at_ms: now,
        expires_at_ms: now + 86_400_000,
        last_seen_at_ms: now,
        user_agent: None,
        passkey_verified_at_ms: None,
        primary_authenticated_at_ms: now,
        primary_auth_method: Some(provider.unwrap_or("password").to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: provider.map(ToOwned::to_owned),
        auth_issuer: issuer.map(ToOwned::to_owned),
        auth_subject: subject.map(ToOwned::to_owned),
        auth_sid: sid.map(ToOwned::to_owned),
        id_token_ciphertext: None,
    };
    let password = make_session("1", "session-password-00000001", None, None, None, None);
    let cardea_a = make_session(
        "2",
        "session-cardea-a-00000001",
        Some("cardea"),
        Some("https://cardea.example"),
        Some("draven"),
        Some("sid-a"),
    );
    let cardea_b = make_session(
        "3",
        "session-cardea-b-00000001",
        Some("cardea"),
        Some("https://cardea.example"),
        Some("draven"),
        Some("sid-b"),
    );
    let google = make_session(
        "4",
        "session-google-00000001",
        Some("google"),
        Some("https://accounts.google.com"),
        Some("draven-google"),
        Some("google-sid"),
    );
    for session in [&password, &cardea_a, &cardea_b, &google] {
        store.insert_user_session(session).await.unwrap();
    }
    let lease = match store
        .claim_active_client(&ActiveClientClaim {
            client_id: "client-cardea-a-00000001".to_owned(),
            user_id: Some(user.id.clone()),
            principal_class: "human".to_owned(),
            session_id: Some(cardea_a.session_id.clone()),
            client_kind: "browser".to_owned(),
            now_ms: now,
            lease_ms: 120_000,
            reservation_ms: 30_000,
            per_user_limit: 4,
            service_limit: 32,
            automation_limit: 32,
            enforce: true,
        })
        .await
        .unwrap()
    {
        ActiveClientAdmission::Granted { lease, .. } => lease,
        ActiveClientAdmission::Waiting { .. } => panic!("test lease unexpectedly waited"),
    };
    assert_eq!(
        store
            .apply_oidc_backchannel_logout(
                "cardea",
                "https://cardea.example",
                None,
                Some("sid-a"),
                "logout-jti-1",
                now + 60_000,
                now + 1,
            )
            .await
            .unwrap(),
        OidcBackchannelLogout::Applied {
            revoked_sessions: 1
        }
    );
    assert!(
        store
            .user_session_by_token_hash(&cardea_a.token_hash)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !store
            .heartbeat_active_client(&lease.client_id, lease.fencing_token, now + 2, 120_000,)
            .await
            .unwrap(),
        "back-channel logout must fence an already connected client"
    );
    assert_eq!(
        store
            .apply_oidc_backchannel_logout(
                "cardea",
                "https://cardea.example",
                None,
                Some("sid-a"),
                "logout-jti-1",
                now + 60_000,
                now + 2,
            )
            .await
            .unwrap(),
        OidcBackchannelLogout::Replay
    );
    assert_eq!(
        store
            .revoke_user_sessions_for_provider(&user.id, "cardea", "provider_logout", now + 3,)
            .await
            .unwrap(),
        1
    );
    assert!(
        store
            .user_session_by_token_hash(&cardea_b.token_hash)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .user_session_by_token_hash(&password.token_hash)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .user_session_by_token_hash(&google.token_hash)
            .await
            .unwrap()
            .is_some()
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[allow(clippy::too_many_lines)] // end-to-end replacement fencing and waiter cleanup contract
#[tokio::test]
async fn sqlite_session_replacement_fences_lease_and_removes_waiter() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-sqlite-session-replacement-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let now = 1_900_000_000_000_i64;
    let user = ProductUser {
        id: "11111111111111111111111111111111".to_owned(),
        username: "capacity-replacement".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: "test-hash".to_owned(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let session = |token: &str, session_id: &str, created_at_ms: i64| ProductUserSession {
        token_hash: token.repeat(64),
        session_id: session_id.to_owned(),
        user_id: user.id.clone(),
        client_kind: "browser".to_owned(),
        principal_class: "human".to_owned(),
        created_at_ms,
        expires_at_ms: created_at_ms + 86_400_000,
        last_seen_at_ms: created_at_ms,
        user_agent: None,
        passkey_verified_at_ms: None,
        primary_authenticated_at_ms: created_at_ms,
        primary_auth_method: Some("password".to_owned()),
        revoked_at_ms: None,
        revoke_reason: None,
        auth_provider_id: None,
        auth_issuer: None,
        auth_subject: None,
        auth_sid: None,
        id_token_ciphertext: None,
    };
    let old_session = session("1", "session-capacity-old-0001", now);
    assert!(matches!(
        store
            .admit_user_session(&old_session, 1, true, false)
            .await
            .unwrap(),
        ProductSessionAdmission::Admitted {
            active_sessions: 1,
            ..
        }
    ));
    let claim = |client_id: &str, session_id: &str, now_ms: i64| ActiveClientClaim {
        client_id: client_id.to_owned(),
        user_id: Some(user.id.clone()),
        principal_class: "human".to_owned(),
        session_id: Some(session_id.to_owned()),
        client_kind: "browser".to_owned(),
        now_ms,
        lease_ms: 120_000,
        reservation_ms: 30_000,
        per_user_limit: 1,
        service_limit: 2,
        automation_limit: 2,
        enforce: true,
    };
    let old_lease = match store
        .claim_active_client(&claim(
            "client-capacity-old-active",
            &old_session.session_id,
            now,
        ))
        .await
        .unwrap()
    {
        ActiveClientAdmission::Granted { lease, .. } => lease,
        ActiveClientAdmission::Waiting { .. } => panic!("old session did not get a seat"),
    };
    assert!(matches!(
        store
            .claim_active_client(&claim(
                "client-capacity-old-waiter",
                &old_session.session_id,
                now + 1,
            ))
            .await
            .unwrap(),
        ActiveClientAdmission::Waiting { position: 1, .. }
    ));

    let new_session = session("2", "session-capacity-new-0001", now + 2);
    let admission = store
        .admit_user_session(&new_session, 1, true, false)
        .await
        .unwrap();
    assert!(matches!(
        admission,
        ProductSessionAdmission::Admitted {
            active_sessions: 1,
            ref revoked_session_ids,
            ..
        } if revoked_session_ids == std::slice::from_ref(&old_session.session_id)
    ));
    assert!(
        !store
            .heartbeat_active_client(
                &old_lease.client_id,
                old_lease.fencing_token,
                now + 3,
                120_000,
            )
            .await
            .unwrap(),
        "replaced sessions must immediately fence their active leases"
    );
    assert!(
        store
            .list_active_clients_for_user(&user.id, now + 3)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store
            .claim_active_client(&claim(
                "client-capacity-new-active",
                &new_session.session_id,
                now + 3,
            ))
            .await
            .unwrap(),
        ActiveClientAdmission::Granted { .. }
    ));

    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sqlite_capacity_queue_skips_per_user_full_lane() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-sqlite-capacity-fairness-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let now = 1_900_000_000_000_i64;
    let user = |id: &str, username: &str| ProductUser {
        id: id.to_owned(),
        username: username.to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: "test-hash".to_owned(),
        created_at_ms: now,
        updated_at_ms: now,
        disabled_at_ms: None,
    };
    let user_a = user("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "capacity-user-a");
    let user_b = user("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "capacity-user-b");
    store.insert_user(&user_a).await.unwrap();
    store.insert_user(&user_b).await.unwrap();
    let claim = |client_id: &str, user_id: &str, now_ms: i64| ActiveClientClaim {
        client_id: client_id.to_owned(),
        user_id: Some(user_id.to_owned()),
        principal_class: "human".to_owned(),
        session_id: None,
        client_kind: "cli".to_owned(),
        now_ms,
        lease_ms: 120_000,
        reservation_ms: 30_000,
        per_user_limit: 1,
        service_limit: 3,
        automation_limit: 3,
        enforce: true,
    };
    assert!(matches!(
        store
            .claim_active_client(&claim("client-user-a-active", &user_a.id, now))
            .await
            .unwrap(),
        ActiveClientAdmission::Granted { .. }
    ));
    assert!(matches!(
        store
            .claim_active_client(&claim("client-user-a-waiter", &user_a.id, now + 1))
            .await
            .unwrap(),
        ActiveClientAdmission::Waiting { position: 1, .. }
    ));
    assert!(matches!(
        store
            .claim_active_client(&claim("client-user-b-active", &user_b.id, now + 2))
            .await
            .unwrap(),
        ActiveClientAdmission::Granted { .. }
    ));

    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sqlite_do_shaped_restore_keeps_compact_working_set() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-sqlite-do-mock-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::connect("sqlite::memory:", root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let mut highwaters = HashMap::new();
    let mut events = Vec::new();
    for index in 0..17 {
        let session_id = format!("sess-do-{index:02}");
        store.insert_session(&session(&session_id)).await.unwrap();
        let mut envelope = Envelope {
            session_id: session_id.clone(),
            seq: 0,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "read",
                    "status": "completed",
                    "content": [{"type": "raw_output", "text": "ok"}],
                    "rawOutput": {"result": "x".repeat(2_580_000)},
                }),
            },
            cmid: None,
        };
        crate::persistence::compact_canonical_tool_output(&mut envelope);
        highwaters.insert(session_id, 1);
        events.push(envelope);
    }
    store
        .upsert_event_batch(&events, &highwaters)
        .await
        .unwrap();
    let loaded = store.load_all().await.unwrap();
    assert_eq!(loaded.len(), 17);
    let restored_bytes = loaded.iter().fold(0usize, |size, session| {
        session.events.iter().fold(size, |size, event| {
            size.saturating_add(crate::core::estimated_envelope_bytes(event))
        })
    });
    assert!(
        restored_bytes < 64 * 1024,
        "compact DO-shaped restore was {restored_bytes} bytes"
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_complete_legacy_ledger_is_marked_without_reapplying_schema() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    let root = std::env::temp_dir().join(format!(
        "cowboy-postgres-baseline-compat-{}",
        std::process::id()
    ));
    let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    let StorageBackend::Postgres(storage) = &store.backend else {
        panic!("expected PostgreSQL backend");
    };
    // Compare the complete current ledger, including post-baseline
    // migrations and their checksums, instead of freezing a version count.
    let ledger_before: Vec<(i64, Vec<u8>, bool)> =
        sqlx::query_as("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&storage.pool)
            .await
            .unwrap();
    assert!(ledger_before.iter().any(|(version, _, success)| {
        *version == crate::migration_compat::POSTGRES_BASELINE_VERSION && *success
    }));
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
        .bind(crate::migration_compat::POSTGRES_BASELINE_VERSION)
        .execute(&storage.pool)
        .await
        .unwrap();
    let machines_before: i64 = sqlx::query_scalar("SELECT count(*) FROM machines")
        .fetch_one(&storage.pool)
        .await
        .unwrap();

    store.migrate().await.unwrap();

    let ledger_after: Vec<(i64, Vec<u8>, bool)> =
        sqlx::query_as("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&storage.pool)
            .await
            .unwrap();
    assert_eq!(ledger_after, ledger_before);
    let machines_after: i64 = sqlx::query_scalar("SELECT count(*) FROM machines")
        .fetch_one(&storage.pool)
        .await
        .unwrap();
    assert_eq!(machines_after, machines_before);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_implements_storage_contract() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    let root =
        std::env::temp_dir().join(format!("cowboy-postgres-contract-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
    Box::pin(run_storage_contract(&store, "sess-900000002"))
        .await
        .unwrap();
    assert_restore_hot_tail_is_byte_bounded(&store, "sess-900000012")
        .await
        .unwrap();
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

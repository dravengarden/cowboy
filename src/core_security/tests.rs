use super::*;
use crate::passkey::{ExternalPasskeyCeremonyRecord, UserPasskey};
use crate::plugin_dir::PluginDir;
use crate::store::{HandoffPoint, Store};
use std::os::unix::fs::{PermissionsExt as _, symlink};

fn config(source: Source) -> Config {
    Config {
        schema: CONFIG_SCHEMA.to_owned(),
        passkeys: PasskeyConfig {
            namespace_id: "passkey".to_owned().try_into().unwrap(),
            source,
        },
    }
}

fn write_config(path: &Path, source: &str) {
    std::fs::write(
        path,
        serde_json::to_vec(&serde_json::json!({
            "schema": CONFIG_SCHEMA,
            "passkeys": { "namespace_id": "passkey", "source": source },
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

struct Fixture {
    root: tempfile::TempDir,
    store: Store,
    dir: PluginDir,
    url: String,
}

impl Fixture {
    async fn new(postgres: Option<&str>) -> Self {
        let root = tempfile::Builder::new()
            .prefix("cowboy-core-security-")
            .tempdir()
            .unwrap();
        let url = postgres.map_or_else(
            || format!("sqlite://{}", root.path().join("core.sqlite").display()),
            str::to_owned,
        );
        let store = Store::connect(&url, root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let dir = PluginDir::open(root.path()).unwrap();
        Self {
            root,
            store,
            dir,
            url,
        }
    }

    async fn reopen(&self) -> Store {
        let store = Store::connect(&self.url, self.root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        store
    }

    async fn legacy(&self) -> crate::plugin_storage::PluginNamespace {
        let host = crate::plugin_host::PluginHostSpec::from_json(include_bytes!(
            "../../examples/authentication/passkey/host.json"
        ))
        .unwrap();
        self.store
            .plugin_storage(self.dir.clone())
            .migrate_plugin("passkey", &host.storage.unwrap())
            .await
            .unwrap()
    }

    async fn execute_core_fixture_sql(&self, sql: &str) {
        if let Some(pool) = self.store.postgres_pool() {
            sqlx::raw_sql(sql).execute(pool).await.unwrap();
        } else {
            let pool = sqlx::SqlitePool::connect(&self.url).await.unwrap();
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
            pool.close().await;
        }
    }
}

fn key(id: &str, owner: &str) -> UserPasskey {
    UserPasskey {
        id: id.to_owned(),
        user_id: owner.to_owned(),
        credential_id: format!("credential-{id}"),
        nickname: format!("fixture {id}"),
        passkey_json: "{ \"counter\":7, \"publicKey\":\"fixture-only\" }".to_owned(),
        created_at_ms: 17,
        last_used_at_ms: Some(23),
    }
}

fn assert_key(actual: &UserPasskey, expected: &UserPasskey) {
    assert_eq!(actual.id, expected.id);
    assert_eq!(actual.user_id, expected.user_id);
    assert_eq!(actual.credential_id, expected.credential_id);
    assert_eq!(actual.nickname, expected.nickname);
    assert_eq!(actual.passkey_json, expected.passkey_json);
    assert_eq!(actual.created_at_ms, expected.created_at_ms);
    assert_eq!(actual.last_used_at_ms, expected.last_used_at_ms);
}

#[test]
fn namespace_identity_is_a_closed_portable_slug() {
    for id in [
        "",
        "../passkey",
        "Passkey",
        "pass_key",
        "-passkey",
        "pass--key",
    ] {
        assert!(NamespaceId::try_from(id.to_owned()).is_err());
    }
    assert!(NamespaceId::try_from("a".repeat(57)).is_err());
    assert!(NamespaceId::try_from("passkey".to_owned()).is_ok());
}

#[test]
fn core_policy_is_private_closed_bounded_and_redacted() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("core.json");
    write_config(&path, "fresh");
    assert_eq!(
        Config::load(Some(&path)).unwrap().unwrap().passkeys,
        config(Source::Fresh).passkeys
    );
    for bytes in [
        format!(r#"{{"schema":"{CONFIG_SCHEMA}","misplaced-secret":"do-not-echo"}}"#),
        format!(
            r#"{{"schema":"{CONFIG_SCHEMA}","passkeys":{{"namespace_id":"passkey","source":"do-not-echo"}}}}"#
        ),
        "x".repeat(8193),
        "{\"do-not-echo\":".to_owned(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        let error = format!("{:#}", Config::load(Some(&path)).unwrap_err());
        assert!(!error.contains("do-not-echo"));
        assert!(!error.contains("misplaced-secret"));
    }
    write_config(&path, "fresh");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Config::load(Some(&path)).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = root.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(Config::load(Some(&link)).is_err());
    assert!(Config::load(Some(Path::new("relative"))).is_err());
}

#[test]
fn core_preflight_with_empty_catalog_needs_no_local_auth_plugin_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("core.json");
    write_config(&path, "fresh");
    let data = root.path().join("absent-data");
    let mut args = crate::cli::ServeArgs::test_plugin_check(&data);
    args.core_security_config = Some(path);
    args.database_url = Some(format!("sqlite://{}", data.join("core.sqlite").display()));
    args.product_auth_enabled = true;
    let (catalog, auth, _) = crate::plugin_activation::prepare_controller_hosts(&args).unwrap();
    assert!(auth.password_enabled);
    let report = serde_json::to_value(catalog.host_preflight_report().unwrap()).unwrap();
    assert_eq!(report["core_security"]["source"], "fresh");
    assert_eq!(report["source_policy"], "catalog_only");
    assert_eq!(report["webauthn_storage_required"], false);
    assert_eq!(report["exact_selections"], serde_json::json!([]));
    assert!(!data.exists());
    args.database_url = None;
    assert!(crate::plugin_activation::prepare_controller_hosts(&args).is_err());
    assert!(!data.exists());
}

#[test]
fn controller_lock_is_exclusive_and_releases_on_drop() {
    let root = tempfile::tempdir().unwrap();
    let dir = PluginDir::open(root.path()).unwrap();
    let owner = ControllerLock::acquire(&dir).unwrap();
    assert!(ControllerLock::acquire(&dir).is_err());
    drop(owner);
    assert!(ControllerLock::acquire(&dir).is_ok());
}

#[tokio::test]
async fn sqlite_core_fresh_round_trip_without_plugins() {
    fresh_round_trip(None).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_core_fresh_round_trip_without_plugins() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL gate required");
    fresh_round_trip(Some(&url)).await;
}

async fn fresh_round_trip(postgres: Option<&str>) {
    let fixture = Fixture::new(postgres).await;
    let early_clone = fixture.store.clone();
    let config = config(Source::Fresh);
    fixture
        .store
        .initialize_core_security(Some(&config), &fixture.dir)
        .await
        .unwrap();
    let (authority, phase) = fixture.store.security_authority().await.unwrap().unwrap();
    assert_eq!(phase, Phase::Ready);
    assert_eq!(
        Authority::inspect(&fixture.dir).unwrap(),
        Some(authority.clone())
    );
    let user = key("user", &"a".repeat(32));
    let admin = key("admin", "root");
    early_clone.insert_user_passkey(&user).await.unwrap();
    early_clone.insert_admin_passkey(&admin).await.unwrap();
    assert!(
        fixture
            .store
            .export_passkey_snapshot()
            .await
            .unwrap()
            .user
            .is_empty()
    );
    let reopened = fixture.reopen().await;
    reopened
        .initialize_core_security(Some(&config), &fixture.dir)
        .await
        .unwrap();
    assert_key(
        &reopened.list_user_passkeys(&user.user_id).await.unwrap()[0],
        &user,
    );
    assert_key(
        &reopened.list_admin_passkeys("root").await.unwrap()[0],
        &admin,
    );
    assert_eq!(
        reopened.security_authority().await.unwrap(),
        Some((authority, Phase::Ready))
    );
    assert!(
        fixture
            .reopen()
            .await
            .initialize_core_security(None, &fixture.dir)
            .await
            .is_err()
    );
    let host = crate::plugin_host::PluginHostSpec::from_json(include_bytes!(
        "../../examples/authentication/passkey/host.json"
    ))
    .unwrap();
    assert!(
        fixture
            .store
            .plugin_storage(fixture.dir.clone())
            .migrate_plugin("passkey", &host.storage.unwrap())
            .await
            .is_err()
    );
    let mut wrong = config;
    wrong.passkeys.namespace_id = "other".to_owned().try_into().unwrap();
    assert!(
        fixture
            .reopen()
            .await
            .initialize_core_security(Some(&wrong), &fixture.dir)
            .await
            .is_err()
    );
    assert!(!fixture.dir.plugin_live_dir("other").unwrap().exists());
    let mut args = crate::cli::ServeArgs::test_plugin_check(fixture.root.path());
    assert!(
        crate::plugin_activation::prepare_controller_hosts(&args).is_err(),
        "omitting both config and DB must not start in-memory"
    );
    args.database_url = Some(fixture.url.clone());
    assert!(crate::plugin_activation::prepare_controller_hosts(&args).is_err());
}

#[tokio::test]
async fn sqlite_core_adoption_keeps_current_credentials_and_never_reimports() {
    adoption_round_trip(None).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_core_adoption_keeps_current_credentials_and_never_reimports() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL gate required");
    adoption_round_trip(Some(&url)).await;
}

async fn adoption_round_trip(postgres: Option<&str>) {
    let fixture = Fixture::new(postgres).await;
    let original = key("admin", "root");
    fixture.store.insert_admin_passkey(&original).await.unwrap();
    let namespace = fixture.legacy().await;
    let mut legacy = fixture.store.clone();
    legacy.attach_passkey_storage(&namespace).await.unwrap();
    legacy
        .update_admin_passkey("root", "admin", "{\"counter\":99}", 50)
        .await
        .unwrap();
    let current = legacy.list_admin_passkeys("root").await.unwrap().remove(0);
    let ceremony = ExternalPasskeyCeremonyRecord {
        transaction_hash: "c".repeat(64),
        ceremony_json: "{ \"fixture\": true }".to_owned(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 100_000,
        created_at_ms: 51,
    };
    legacy
        .upsert_external_passkey_ceremony(&ceremony)
        .await
        .unwrap();
    let core = fixture.reopen().await;
    core.initialize_core_security(Some(&config(Source::AdoptLegacy)), &fixture.dir)
        .await
        .unwrap();
    assert_key(
        &core.list_admin_passkeys("root").await.unwrap()[0],
        &current,
    );
    let loaded = core
        .external_passkey_ceremony(&ceremony.transaction_hash, 0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.ceremony_json, ceremony.ceremony_json);
    assert_eq!(loaded.created_at_ms, ceremony.created_at_ms);
    assert_eq!(loaded.expires_at_ms, ceremony.expires_at_ms);
    assert_eq!(core.delete_admin_passkey("root", "admin").await.unwrap(), 1);
    let restarted = fixture.reopen().await;
    restarted
        .initialize_core_security(Some(&config(Source::AdoptLegacy)), &fixture.dir)
        .await
        .unwrap();
    assert!(
        restarted
            .list_admin_passkeys("root")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        restarted
            .export_passkey_snapshot()
            .await
            .unwrap()
            .admin
            .len(),
        1,
        "stale legacy source is retained but never read as authority"
    );
    assert!(
        fixture
            .reopen()
            .await
            .initialize_core_security(Some(&config(Source::Fresh)), &fixture.dir)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn sqlite_core_handoff_recovers_each_committed_window() {
    interruption_contract(None).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_core_handoff_recovers_each_committed_window() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL gate required");
    interruption_contract(Some(&url)).await;
}

async fn interruption_contract(postgres: Option<&str>) {
    let fixture = Fixture::new(postgres).await;
    let config = config(Source::Fresh);
    let mut identity = None;
    for point in [
        HandoffPoint::Prepared,
        HandoffPoint::Marked,
        HandoffPoint::NamespacePrepared,
        HandoffPoint::NamespaceCommitted,
    ] {
        let store = fixture.reopen().await;
        let error = store
            .test_security_interruption(&config, &fixture.dir, point)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected"), "{error:#}");
        let (authority, phase) = store.security_authority().await.unwrap().unwrap();
        assert_eq!(phase, Phase::Prepared);
        assert!(identity.as_ref().is_none_or(|value| value == &authority));
        identity = Some(authority);
        assert!(store.require_legacy_security().await.is_err());
        let host = crate::plugin_host::PluginHostSpec::from_json(include_bytes!(
            "../../examples/authentication/passkey/host.json"
        ))
        .unwrap();
        assert!(
            store
                .plugin_storage(fixture.dir.clone())
                .migrate_plugin("passkey", &host.storage.unwrap())
                .await
                .is_err()
        );
    }
    let store = fixture.reopen().await;
    store
        .initialize_core_security(Some(&config), &fixture.dir)
        .await
        .unwrap();
    assert_eq!(
        store.security_authority().await.unwrap(),
        Some((identity.unwrap(), Phase::Ready))
    );
}

#[tokio::test]
async fn core_fresh_refuses_existing_namespaces_and_legacy_data() {
    let fixture = Fixture::new(None).await;
    fixture
        .store
        .insert_admin_passkey(&key("admin", "root"))
        .await
        .unwrap();
    assert!(
        fixture
            .store
            .initialize_core_security(Some(&config(Source::Fresh)), &fixture.dir)
            .await
            .is_err()
    );
    assert!(fixture.store.security_authority().await.unwrap().is_none());
    let _namespace = fixture.legacy().await;
    // A namespace without its completed import cannot be adopted either.
    assert!(
        fixture
            .store
            .initialize_core_security(Some(&config(Source::AdoptLegacy)), &fixture.dir)
            .await
            .is_err()
    );
    assert!(fixture.store.security_authority().await.unwrap().is_none());
    assert!(Authority::inspect(&fixture.dir).unwrap().is_none());
}

#[tokio::test]
async fn core_ready_missing_namespace_or_database_fails_without_recreation() {
    let fixture = Fixture::new(None).await;
    let config = config(Source::Fresh);
    fixture
        .store
        .initialize_core_security(Some(&config), &fixture.dir)
        .await
        .unwrap();
    let other = Store::connect(
        &format!(
            "sqlite://{}",
            fixture.root.path().join("wrong.sqlite").display()
        ),
        fixture.root.path().join("artifacts"),
    )
    .await
    .unwrap();
    other.migrate().await.unwrap();
    assert!(
        other
            .initialize_core_security(Some(&config), &fixture.dir)
            .await
            .is_err()
    );
    assert!(other.security_authority().await.unwrap().is_none());
    let path = namespace_path(&fixture.dir, &config.passkeys.namespace_id).unwrap();
    std::fs::rename(&path, path.with_extension("saved")).unwrap();
    let error = fixture
        .reopen()
        .await
        .initialize_core_security(Some(&config), &fixture.dir)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing"), "{error:#}");
    assert!(!path.exists());
}

#[tokio::test]
async fn core_symlinked_namespace_and_corrupt_marker_fail_closed() {
    let fixture = Fixture::new(None).await;
    let state = fixture.dir.ensure_state_dir("passkey").unwrap();
    let external = fixture.root.path().join("must-not-open");
    std::fs::write(&external, b"untouched").unwrap();
    symlink(&external, state.join("db.sqlite")).unwrap();
    assert!(
        fixture
            .store
            .initialize_core_security(Some(&config(Source::AdoptLegacy)), &fixture.dir)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&external).unwrap(), b"untouched");
    assert!(fixture.store.security_authority().await.unwrap().is_none());
    let marker = fixture.dir.root().join(MARKER);
    std::fs::write(&marker, b"{}").unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(Authority::inspect(&fixture.dir).is_err());
    assert!(
        fixture
            .store
            .initialize_core_security(None, &fixture.dir)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn sqlite_core_ready_commit_failure_retains_namespace_and_recovers() {
    ready_failure(None).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_core_ready_commit_failure_retains_namespace_and_recovers() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL gate required");
    ready_failure(Some(&url)).await;
}

async fn ready_failure(postgres: Option<&str>) {
    let fixture = Fixture::new(postgres).await;
    let (inject, recover) = if postgres.is_some() {
        (
            "CREATE FUNCTION core_security_test_fail_ready() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture ready commit failure'; END; $$; \
         CREATE TRIGGER core_security_test_ready BEFORE UPDATE ON core_security_authority FOR EACH ROW WHEN (NEW.phase = 'ready') EXECUTE FUNCTION core_security_test_fail_ready();",
            "DROP TRIGGER core_security_test_ready ON core_security_authority; DROP FUNCTION core_security_test_fail_ready();",
        )
    } else {
        (
            "CREATE TRIGGER core_security_test_ready BEFORE UPDATE ON core_security_authority WHEN NEW.phase = 'ready' BEGIN SELECT RAISE(ABORT, 'fixture ready commit failure'); END;",
            "DROP TRIGGER core_security_test_ready;",
        )
    };
    fixture.execute_core_fixture_sql(inject).await;
    let config = config(Source::Fresh);
    assert!(
        fixture
            .store
            .initialize_core_security(Some(&config), &fixture.dir)
            .await
            .is_err()
    );
    let (authority, phase) = fixture.store.security_authority().await.unwrap().unwrap();
    assert_eq!(phase, Phase::Prepared);
    fixture.execute_core_fixture_sql(recover).await;
    let restarted = fixture.reopen().await;
    restarted
        .initialize_core_security(Some(&config), &fixture.dir)
        .await
        .unwrap();
    assert_eq!(
        restarted.security_authority().await.unwrap(),
        Some((authority, Phase::Ready))
    );
}

#[tokio::test]
async fn core_ready_missing_namespace_token_does_not_reclaim_storage() {
    let fixture = Fixture::new(None).await;
    let config = config(Source::Fresh);
    fixture
        .store
        .initialize_core_security(Some(&config), &fixture.dir)
        .await
        .unwrap();
    let key = key("admin", "root");
    fixture.store.insert_admin_passkey(&key).await.unwrap();
    let path = namespace_path(&fixture.dir, &config.passkeys.namespace_id).unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(path))
        .await
        .unwrap();
    sqlx::query("DELETE FROM _cowboy_core_security_owner")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        fixture
            .reopen()
            .await
            .initialize_core_security(Some(&config), &fixture.dir)
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _cowboy_core_security_owner")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "ready startup must not manufacture a missing token"
    );
    assert_key(
        &fixture.store.list_admin_passkeys("root").await.unwrap()[0],
        &key,
    );
    pool.close().await;
}

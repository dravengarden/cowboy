use super::*;
use crate::plugin_dir::PluginDir;
use crate::plugin_host::PluginHostSpec;
use crate::plugin_runtime::PluginRuntime;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

fn unlock_fixture_directories(path: &Path) {
    if !std::fs::symlink_metadata(path).unwrap().is_dir() {
        return;
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    for entry in std::fs::read_dir(path).unwrap() {
        unlock_fixture_directories(&entry.unwrap().path());
    }
}

#[tokio::test]
async fn sqlite_passkey_plugin_round_trip_and_import() {
    assert_passkey_plugin_round_trip_and_import(None).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_passkey_plugin_round_trip_and_import() {
    let url =
        std::env::var("COWBOY_TEST_POSTGRES_URL").expect("run nix develop -c just test-postgres");
    assert_passkey_plugin_round_trip_and_import(Some(&url)).await;
}

async fn assert_passkey_plugin_round_trip_and_import(postgres_url: Option<&str>) {
    let root = std::env::temp_dir().join(format!(
        "cowboy-plugin-passkeys-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let sqlite_url = format!("sqlite://{}", root.join("core.sqlite3").display());
    let store = Store::connect(postgres_url.unwrap_or(&sqlite_url), root.join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let user = crate::store::ProductUser {
        id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
        username: "owner".to_owned(),
        password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
        password_hash: "hash".to_owned(),
        created_at_ms: 1,
        updated_at_ms: 1,
        disabled_at_ms: None,
    };
    store.insert_user(&user).await.unwrap();
    let core_passkey = UserPasskey {
        id: "pk1".to_owned(),
        user_id: user.id.clone(),
        credential_id: "cred-1".to_owned(),
        nickname: "laptop".to_owned(),
        passkey_json: "{\"ok\":true}".to_owned(),
        created_at_ms: 10,
        last_used_at_ms: None,
    };
    store.insert_user_passkey(&core_passkey).await.unwrap();
    let admin_passkey = UserPasskey {
        id: "admin-pk1".to_owned(),
        user_id: "root".to_owned(),
        credential_id: "admin-cred-1".to_owned(),
        nickname: "admin laptop".to_owned(),
        passkey_json: "{\"admin\":true}".to_owned(),
        created_at_ms: 20,
        last_used_at_ms: Some(30),
    };
    store.insert_admin_passkey(&admin_passkey).await.unwrap();

    let dir = PluginDir::open(&root).unwrap();
    let storage = store.plugin_storage(dir);
    let spec = PluginHostSpec::from_json(
        include_str!("../../examples/authentication/passkey/host.json").as_bytes(),
    )
    .unwrap();
    let namespace = storage
        .migrate_plugin("passkey", spec.storage.as_ref().unwrap())
        .await
        .unwrap();
    let ns = PasskeyStorage::open_legacy(&namespace, &store)
        .await
        .unwrap();
    import_from_core(&ns, &store).await.unwrap();
    import_from_core(&ns, &store).await.unwrap();
    let listed = list_user(&ns, &user.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].nickname, "laptop");
    assert_eq!(listed[0].passkey_json, core_passkey.passkey_json);
    assert_eq!(listed[0].created_at_ms, core_passkey.created_at_ms);
    assert_eq!(listed[0].last_used_at_ms, None);
    assert_eq!(count_user(&ns, &user.id).await.unwrap(), 1);
    let admins = list_admin(&ns, "root").await.unwrap();
    assert_eq!(admins.len(), 1);
    assert_eq!(admins[0].credential_id, admin_passkey.credential_id);
    assert_eq!(admins[0].passkey_json, admin_passkey.passkey_json);
    assert_eq!(admins[0].last_used_at_ms, Some(30));

    update_user(&ns, &user.id, "pk1", "{\"used\":true}", 40)
        .await
        .unwrap();
    update_admin(&ns, "root", "admin-pk1", "{\"admin_used\":true}", 50)
        .await
        .unwrap();
    assert_eq!(
        list_user(&ns, &user.id).await.unwrap()[0].last_used_at_ms,
        Some(40)
    );
    assert_eq!(
        list_admin(&ns, "root").await.unwrap()[0].last_used_at_ms,
        Some(50)
    );
    assert_eq!(delete_user(&ns, &user.id, "pk1").await.unwrap(), 1);
    assert_eq!(delete_admin(&ns, "root", "admin-pk1").await.unwrap(), 1);
    import_from_core(&ns, &store).await.unwrap();
    assert_eq!(count_user(&ns, &user.id).await.unwrap(), 0);
    assert_eq!(count_admin(&ns, "root").await.unwrap(), 0);
    assert_eq!(store.list_user_passkeys(&user.id).await.unwrap().len(), 1);
    assert_eq!(store.list_admin_passkeys("root").await.unwrap().len(), 1);

    assert_bootstrap_runtime_hosts(&store, &root).await;
    drop(store);
    unlock_fixture_directories(&root);
    std::fs::remove_dir_all(root).unwrap();
}

async fn assert_bootstrap_runtime_hosts(store: &Store, root: &Path) {
    let runtime_dir = PluginDir::open(&root.join("runtime")).unwrap();
    let runtime_storage = store.plugin_storage(runtime_dir);
    let runtime = PluginRuntime::activate(&runtime_storage, None)
        .await
        .unwrap();
    assert!(
        runtime
            .default_hosts()
            .iter()
            .any(|host| host.id == "password")
    );
    assert!(
        runtime
            .namespace_for_capability(STORAGE_CAPABILITY)
            .is_some()
    );
}

fn legacy_host() -> PluginHostSpec {
    PluginHostSpec::from_json(include_bytes!(
        "../../examples/authentication/passkey/host.json"
    ))
    .unwrap()
}

#[test]
fn core_owns_the_exact_passkey_schema_for_both_dialects() {
    validate_legacy_host(&legacy_host()).unwrap();
    for postgres in [true, false] {
        for mutation in 0..4 {
            let mut host = legacy_host();
            let storage = host.storage.as_mut().unwrap();
            let migrations = if postgres {
                &mut storage.postgres
            } else {
                &mut storage.sqlite
            };
            match mutation {
                0 => migrations.migrations[0].sql.push(' '),
                1 => migrations.migrations[0].version = "0002".to_owned(),
                2 => migrations.migrations.clear(),
                _ => migrations
                    .migrations
                    .push(crate::plugin_host::PluginMigration {
                        version: "0002".to_owned(),
                        sql: "CREATE TABLE future_credentials (id TEXT PRIMARY KEY);".to_owned(),
                    }),
            }
            assert!(validate_legacy_host(&host).is_err());
        }
    }
    let mut host = legacy_host();
    host.storage = None;
    assert!(validate_legacy_host(&host).is_err());
    host.native_capabilities.clear();
    validate_legacy_host(&host).expect("unrelated hosts do not acquire a security dependency");
}

#[tokio::test]
async fn sqlite_core_passkey_storage_failure_contract() {
    assert_core_storage_failure_contract(None).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_core_passkey_storage_failure_contract() {
    let url =
        std::env::var("COWBOY_TEST_POSTGRES_URL").expect("run nix develop -c just test-postgres");
    assert_core_storage_failure_contract(Some(&url)).await;
}

async fn namespace(storage: &crate::plugin_storage::PluginStorage, id: &str) -> PluginNamespace {
    storage
        .migrate_plugin(id, legacy_host().storage.as_ref().unwrap())
        .await
        .unwrap()
}

async fn assert_rows(
    namespace: &PluginNamespace,
    users: i64,
    admins: i64,
    ceremonies: i64,
    imports: i64,
) {
    for (table, expected) in [
        ("user_passkeys", users),
        ("admin_passkeys", admins),
        ("external_passkey_ceremonies", ceremonies),
        ("_cowboy_import", imports),
    ] {
        assert_eq!(
            namespace
                .fetch_i64(&format!("SELECT COUNT(*) FROM {table}"))
                .await
                .unwrap(),
            expected,
            "{table}"
        );
    }
}

async fn assert_core_storage_failure_contract(postgres_url: Option<&str>) {
    let root = tempfile::Builder::new()
        .prefix("cowboy-core-passkeys-")
        .tempdir()
        .unwrap();
    let sqlite_url = format!("sqlite://{}", root.path().join("core.sqlite3").display());
    let mut store = Store::connect(
        postgres_url.unwrap_or(&sqlite_url),
        root.path().join("artifacts"),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    let storage = store.plugin_storage(PluginDir::open(root.path()).unwrap());
    assert_ceremony_isolation(&store, &storage).await;
    seed_core_source(&store).await;
    assert_atomic_import(&store, &storage).await;
    assert_ambiguous_import_refused(&store, &storage).await;
    assert_shared_binding_and_reopen(&mut store, &storage).await;
}

async fn seed_core_source(store: &Store) {
    let now = chrono::Utc::now().timestamp_millis();
    store
        .insert_user(&crate::store::ProductUser {
            id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            username: "core-fixture".to_owned(),
            password_algo: crate::product_auth::PASSWORD_ALGO_ARGON2ID.to_owned(),
            password_hash: "fixture-only".to_owned(),
            created_at_ms: now,
            updated_at_ms: now,
            disabled_at_ms: None,
        })
        .await
        .unwrap();
    for admin in [false, true] {
        let passkey = UserPasskey {
            id: if admin { "admin-key" } else { "user-key" }.to_owned(),
            user_id: if admin {
                "root"
            } else {
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            }
            .to_owned(),
            credential_id: if admin {
                "admin-credential"
            } else {
                "user-credential"
            }
            .to_owned(),
            nickname: "fixture 🔑".to_owned(),
            // Deliberately non-canonical bytes: no re-encoding during import.
            passkey_json: "{ \"counter\": 41, \"publicKey\": \"fixture\" }".to_owned(),
            created_at_ms: now - 10_000,
            last_used_at_ms: Some(now - 1_000),
        };
        if admin {
            store.insert_admin_passkey(&passkey).await.unwrap();
        } else {
            store.insert_user_passkey(&passkey).await.unwrap();
        }
    }
    for index in 1..=2 {
        store
            .upsert_external_passkey_ceremony(&ExternalPasskeyCeremonyRecord {
                transaction_hash: format!("{index:064x}"),
                ceremony_json: format!("{{ \"fixture\": {index} }}"),
                created_at_ms: now + index,
                expires_at_ms: now + index * 60_000,
            })
            .await
            .unwrap();
    }
}

async fn assert_atomic_import(store: &Store, storage: &crate::plugin_storage::PluginStorage) {
    let ns = namespace(storage, "atomic-passkey").await;
    // Fault at the very last INSERT, after both credential tables and all
    // ceremonies. Only this disposable fixture has this rejecting index.
    ns.execute("INSERT INTO _cowboy_import (name, applied_at_ms) VALUES ('fault-injection', 0)")
        .await
        .unwrap();
    ns.execute("CREATE UNIQUE INDEX reject_import_receipt ON _cowboy_import ((1))")
        .await
        .unwrap();
    assert!(PasskeyStorage::open_legacy(&ns, store).await.is_err());
    assert_rows(&ns, 0, 0, 0, 1).await;
    ns.execute("DROP INDEX reject_import_receipt")
        .await
        .unwrap();
    ns.execute("DELETE FROM _cowboy_import WHERE name = 'fault-injection'")
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        PasskeyStorage::open_legacy(&ns, store),
        PasskeyStorage::open_legacy(&ns, store),
    );
    let imported = first.expect("retry after complete rollback");
    second.expect("concurrent initializer must see the committed receipt");
    assert_rows(&ns, 1, 1, 2, 1).await;
    let source = store.export_passkey_snapshot().await.unwrap();
    for (actual, expected) in [
        (
            list_user(&imported, &source.user[0].user_id).await.unwrap(),
            &source.user[0],
        ),
        (
            list_admin(&imported, &source.admin[0].user_id)
                .await
                .unwrap(),
            &source.admin[0],
        ),
    ] {
        assert_eq!(actual.len(), 1);
        let actual = &actual[0];
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.user_id, expected.user_id);
        assert_eq!(actual.credential_id, expected.credential_id);
        assert_eq!(actual.nickname, expected.nickname);
        assert_eq!(actual.passkey_json, expected.passkey_json);
        assert_eq!(actual.created_at_ms, expected.created_at_ms);
        assert_eq!(actual.last_used_at_ms, expected.last_used_at_ms);
    }
    for row in &source.ceremonies {
        let actual = ceremony(&imported, &row.transaction_hash, row.created_at_ms)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual.transaction_hash, row.transaction_hash);
        assert_eq!(actual.ceremony_json, row.ceremony_json);
        assert_eq!(actual.created_at_ms, row.created_at_ms);
        assert_eq!(actual.expires_at_ms, row.expires_at_ms);
    }
    // The old migration reader still accepts exactly the same ledger/SQL.
    storage
        .migrate_plugin("atomic-passkey", legacy_host().storage.as_ref().unwrap())
        .await
        .unwrap();
}

async fn assert_ambiguous_import_refused(
    store: &Store,
    storage: &crate::plugin_storage::PluginStorage,
) {
    for (id, sql, counts) in [
        (
            "partial-user",
            "INSERT INTO user_passkeys (id, user_id, credential_id, nickname, passkey_json, created_at_ms) VALUES ('partial', 'owner', 'cred', 'fixture', '{}', 1)",
            (1, 0, 0),
        ),
        (
            "partial-admin",
            "INSERT INTO admin_passkeys (id, account, credential_id, nickname, passkey_json, created_at_ms) VALUES ('partial', 'root', 'cred', 'fixture', '{}', 1)",
            (0, 1, 0),
        ),
        (
            "partial-ceremony",
            "INSERT INTO external_passkey_ceremonies (transaction_hash, ceremony_json, expires_at_ms, created_at_ms) VALUES ('partial', '{}', 2, 1)",
            (0, 0, 1),
        ),
    ] {
        let ns = namespace(storage, id).await;
        ns.execute(sql).await.unwrap();
        let error = PasskeyStorage::open_legacy(&ns, store).await.err().unwrap();
        assert!(
            error
                .to_string()
                .contains("refusing automatic reconciliation")
        );
        assert_rows(&ns, counts.0, counts.1, counts.2, 0).await;
    }
    let mut future = legacy_host().storage.unwrap();
    let addition = crate::plugin_host::PluginMigration {
        version: "0002".to_owned(),
        sql: "CREATE TABLE future_state (id TEXT PRIMARY KEY);".to_owned(),
    };
    future.postgres.migrations.push(addition.clone());
    future.sqlite.migrations.push(addition);
    let ns = storage
        .migrate_plugin("incompatible-passkey", &future)
        .await
        .unwrap();
    let error = PasskeyStorage::open_legacy(&ns, store).await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("incompatible Passkey migration ledger")
    );
    assert_rows(&ns, 0, 0, 0, 0).await;
    let tampered = namespace(storage, "tampered-passkey").await;
    // Corrupt only a disposable fixture to exercise checksum rejection.
    tampered
        .execute("UPDATE _cowboy_plugin_migrations SET checksum = 'invalid'")
        .await
        .unwrap();
    assert!(
        PasskeyStorage::open_legacy(&tampered, store)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("incompatible Passkey migration ledger")
    );
    assert_rows(&tampered, 0, 0, 0, 0).await;
}

async fn assert_shared_binding_and_reopen(
    store: &mut Store,
    storage: &crate::plugin_storage::PluginStorage,
) {
    let first_ns = namespace(storage, "bound-passkey").await;
    let competing_ns = namespace(storage, "competing-passkey").await;
    let mut cloned_before_binding = store.clone();
    let (first, competing) = tokio::join!(
        store.attach_passkey_storage(&first_ns),
        cloned_before_binding.attach_passkey_storage(&competing_ns),
    );
    assert_ne!(first.is_ok(), competing.is_ok());
    let ns = if first.is_ok() {
        assert_rows(&competing_ns, 0, 0, 0, 0).await;
        &first_ns
    } else {
        assert_rows(&first_ns, 0, 0, 0, 0).await;
        &competing_ns
    };
    assert!(
        store.attach_passkey_storage(ns).await.is_err(),
        "binding cannot be replaced"
    );
    let user = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    cloned_before_binding
        .update_user_passkey(
            user,
            "user-key",
            "{\"counter\":42}",
            chrono::Utc::now().timestamp_millis(),
        )
        .await
        .unwrap();
    assert_eq!(
        store.list_user_passkeys(user).await.unwrap()[0].passkey_json,
        "{\"counter\":42}"
    );
    assert!(
        store.export_passkey_snapshot().await.unwrap().user[0]
            .passkey_json
            .contains("41")
    );
    let reopened = PasskeyStorage::open_legacy(ns, store).await.unwrap();
    assert_eq!(
        list_user(&reopened, user).await.unwrap()[0].passkey_json,
        "{\"counter\":42}"
    );
    cloned_before_binding
        .delete_user_passkey(user, "user-key")
        .await
        .unwrap();
    cloned_before_binding
        .delete_admin_passkey("root", "admin-key")
        .await
        .unwrap();
    assert!(store.list_user_passkeys(user).await.unwrap().is_empty());
    assert_eq!(store.count_admin_passkeys("root").await.unwrap(), 0);
    let reopened = PasskeyStorage::open_legacy(ns, store).await.unwrap();
    assert_eq!(
        count_user(&reopened, user).await.unwrap(),
        0,
        "deleted credentials cannot be resurrected from stale core rows"
    );
    assert_eq!(count_admin(&reopened, "root").await.unwrap(), 0);
    assert_rows(ns, 0, 0, 2, 1).await;
}

async fn assert_ceremony_isolation(store: &Store, storage: &crate::plugin_storage::PluginStorage) {
    let ns = namespace(storage, "isolated-ceremonies").await;
    let owned = PasskeyStorage::open_legacy(&ns, store).await.unwrap();
    for (id, created, expires) in [("expired", 1, 2), ("earlier", 2, 1000), ("later", 3, 2000)] {
        upsert_ceremony_at(
            &owned,
            &ExternalPasskeyCeremonyRecord {
                transaction_hash: id.to_owned(),
                ceremony_json: format!("{{\"flow\":\"{id}\"}}"),
                created_at_ms: created,
                expires_at_ms: expires,
            },
            0,
        )
        .await
        .unwrap();
    }
    assert_rows(&ns, 0, 0, 3, 1).await;
    assert!(ceremony(&owned, "earlier", 2).await.unwrap().is_some());
    assert!(ceremony(&owned, "later", 2).await.unwrap().is_some());
    ns.execute(
        "CREATE UNIQUE INDEX reject_ceremony_write ON external_passkey_ceremonies (created_at_ms)",
    )
    .await
    .unwrap();
    let duplicate = ExternalPasskeyCeremonyRecord {
        transaction_hash: "failed".to_owned(),
        ceremony_json: "{}".to_owned(),
        created_at_ms: 2,
        expires_at_ms: 2000,
    };
    assert!(upsert_ceremony_at(&owned, &duplicate, 2).await.is_err());
    assert_rows(&ns, 0, 0, 3, 1).await; // GC also rolls back on failed insert.
    ns.execute("DROP INDEX reject_ceremony_write")
        .await
        .unwrap();
    let update = ExternalPasskeyCeremonyRecord {
        transaction_hash: "later".to_owned(),
        ceremony_json: "{\"completed\":true}".to_owned(),
        created_at_ms: 4,
        expires_at_ms: 2000,
    };
    upsert_ceremony_at(&owned, &update, 2).await.unwrap();
    assert_rows(&ns, 0, 0, 2, 1).await;
    let later = ceremony(&owned, "later", 2).await.unwrap().unwrap();
    assert_eq!(later.ceremony_json, update.ceremony_json);
    assert_eq!(
        later.created_at_ms, 3,
        "update preserves original creation time"
    );
    assert!(ceremony(&owned, "earlier", 2).await.unwrap().is_some());
    assert!(ceremony(&owned, "earlier", 1000).await.unwrap().is_none());
    assert!(ceremony(&owned, "later", 1999).await.unwrap().is_some());
    assert!(ceremony(&owned, "later", 2000).await.unwrap().is_none());
}

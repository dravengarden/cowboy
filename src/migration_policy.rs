//! Guard the consolidated startup-migration contract.

#![warn(clippy::pedantic)]

use sha2::Digest as _;

const PUBLISHED_POSTGRES_MIGRATIONS: &[(&str, &str)] = &[
    (
        "0034_baseline.sql",
        "b66deb3964b2bd6ef80c81407caa76463ba2a7173e344b6219986fcc5868c373",
    ),
    (
        "0035_passkey_session_refresh.sql",
        "be13646b8368d1e519faf6a7d241a7f4b815a814e92ffd403fb34d58b0b86170",
    ),
    (
        "0036_passkey_reauth_interval.sql",
        "facb5411f50a2ca2f56032d89bfc50e1fb5dea59800d1957ba45b11ec18d59f5",
    ),
    (
        "0037_user_device_sessions.sql",
        "bd1cfc10078f7927d9e802fd6b9ec7dfaaa1151a66e5176e66636f9d99e25b27",
    ),
    (
        "0038_session_auth_deadlines.sql",
        "863dab2d644db009484c5a374b9e428e797f17bb7b2e831eb9ace6a5230c0796",
    ),
    (
        "0039_passkey_verification_frequency.sql",
        "45993509a173e58c7f3fdddef62e4246093375f0de1cf0006bc4475e555ac4c7",
    ),
    (
        "0040_session_primary_auth_method.sql",
        "17f7251986496f5f87ad514aaf37b8c770690d540b02d8f9ad92d9822b133865",
    ),
];
const PUBLISHED_SQLITE_MIGRATIONS: &[(&str, &str)] = &[
    (
        "0008_baseline.sql",
        "07b0d27ab6eeb8b8ece01505b387abc4bbb28fc36ee977f92820589cce4f7da9",
    ),
    (
        "0009_passkey_session_refresh.sql",
        "17721aa588d1e06bcfc903b5773755584edf640858e5ec591c00c3eb0f90f104",
    ),
    (
        "0010_passkey_reauth_interval.sql",
        "6d1bf69955046a8fbe8a5115ad192a089019a7b663249b941847682d926b740c",
    ),
    (
        "0011_user_device_sessions.sql",
        "e678ec9e58f7266c615b33094636077e7b963b8098e1725e7314774d5028b7ca",
    ),
    (
        "0012_session_auth_deadlines.sql",
        "528c9e20def9be826e7176e16301a4c1dfcb7d1131bf22902738ce664254b740",
    ),
    (
        "0013_passkey_verification_frequency.sql",
        "4abb3409463377fa4422113a82601cb0080f7bff2b94ab8ef7a82e39dc1d86eb",
    ),
    (
        "0014_session_primary_auth_method.sql",
        "83171f8df96ec003b2447643cd67b698370cc06af338f64d1e2ac0e6587ceb31",
    ),
];

fn assert_published_migrations_are_immutable(
    directory: &std::path::Path,
    published: &[(&str, &str)],
) {
    let mut seen = 0;
    for entry in std::fs::read_dir(directory).expect("read migrations") {
        let entry = entry.expect("migration entry");
        if entry.path().extension() != Some(std::ffi::OsStr::new("sql")) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let expected = published
            .iter()
            .find_map(|(candidate, digest)| (*candidate == name).then_some(*digest))
            .unwrap_or_else(|| {
                panic!("{name} has no published checksum; register it before deploy")
            });
        let bytes = std::fs::read(entry.path()).expect("read migration");
        let actual = format!("{:x}", sha2::Sha256::digest(bytes));
        assert_eq!(actual, expected, "published migration {name} was modified");
        seen += 1;
    }
    assert_eq!(seen, published.len(), "a registered migration is missing");
}

#[test]
fn published_postgres_migrations_are_immutable() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    assert_published_migrations_are_immutable(&directory, PUBLISHED_POSTGRES_MIGRATIONS);
}

#[test]
fn published_sqlite_migrations_are_immutable() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/sqlite");
    assert_published_migrations_are_immutable(&directory, PUBLISHED_SQLITE_MIGRATIONS);
}

#[test]
fn sqlite_passkey_frequency_migration_tightens_retired_values() {
    let connection = rusqlite::Connection::open_in_memory().expect("open SQLite database");
    connection
        .execute_batch(
            "CREATE TABLE users (id TEXT PRIMARY KEY, passkey_reauth_interval_ms INTEGER NOT NULL);\
             INSERT INTO users VALUES\
             ('a', 14400000), ('b', 28800000), ('c', 43200000),\
             ('d', 86400000), ('e', 259200000), ('f', 604800000),\
             ('g', 1209600000);",
        )
        .expect("create predecessor Passkey schema");
    connection
        .execute_batch(include_str!(
            "../migrations/sqlite/0013_passkey_verification_frequency.sql"
        ))
        .expect("apply Passkey verification frequency migration");

    let mut statement = connection
        .prepare("SELECT passkey_verification_interval_ms FROM users ORDER BY id")
        .expect("prepare migrated interval query");
    let values = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("query migrated intervals")
        .collect::<Result<Vec<_>, _>>()
        .expect("read migrated intervals");
    assert_eq!(
        values,
        vec![
            14_400_000,
            21_600_000,
            43_200_000,
            86_400_000,
            259_200_000,
            86_400_000,
            86_400_000,
        ]
    );
    assert!(
        connection
            .execute(
                "UPDATE users SET passkey_verification_interval_ms = 604800000 WHERE id = 'a'",
                [],
            )
            .is_err(),
        "retired seven-day values must fail the new database constraint"
    );
}

#[test]
fn sqlite_session_auth_method_migration_preserves_legacy_sessions() {
    let connection = rusqlite::Connection::open_in_memory().expect("open SQLite database");
    connection
        .execute_batch(
            "CREATE TABLE user_sessions (token_hash TEXT PRIMARY KEY);\
             INSERT INTO user_sessions VALUES ('legacy');",
        )
        .expect("create predecessor session schema");
    connection
        .execute_batch(include_str!(
            "../migrations/sqlite/0014_session_primary_auth_method.sql"
        ))
        .expect("apply session authentication method migration");

    let legacy: Option<String> = connection
        .query_row(
            "SELECT primary_auth_method FROM user_sessions WHERE token_hash = 'legacy'",
            [],
            |row| row.get(0),
        )
        .expect("read legacy authentication method");
    assert_eq!(legacy, None);
    connection
        .execute(
            "UPDATE user_sessions SET primary_auth_method = 'cardea' WHERE token_hash = 'legacy'",
            [],
        )
        .expect("bind legacy session to a provider");
    assert!(
        connection
            .execute(
                "UPDATE user_sessions SET primary_auth_method = 'Cardea' WHERE token_hash = 'legacy'",
                [],
            )
            .is_err(),
        "provider IDs must retain the canonical login-method shape"
    );
}

#[test]
fn baselines_preserve_predecessor_rollback_ledgers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let postgres = std::fs::read_to_string(root.join("migrations/0034_baseline.sql"))
        .expect("read PostgreSQL baseline");
    let sqlite = std::fs::read_to_string(root.join("migrations/sqlite/0008_baseline.sql"))
        .expect("read SQLite baseline");
    assert!(postgres.contains("Preserve predecessor rollback compatibility"));
    assert!(sqlite.contains("Preserve predecessor rollback compatibility"));
    assert_eq!(postgres.matches("'legacy compatibility'").count(), 33);
    assert_eq!(sqlite.matches("'legacy compatibility'").count(), 7);
}

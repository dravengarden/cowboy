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

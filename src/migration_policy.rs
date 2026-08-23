//! Guard the consolidated startup-migration contract.

#![warn(clippy::pedantic)]

use sha2::Digest as _;

const PUBLISHED_POSTGRES_BASELINE: (&str, &str) = (
    "0034_baseline.sql",
    "b66deb3964b2bd6ef80c81407caa76463ba2a7173e344b6219986fcc5868c373",
);
const PUBLISHED_SQLITE_BASELINE: (&str, &str) = (
    "0008_baseline.sql",
    "07b0d27ab6eeb8b8ece01505b387abc4bbb28fc36ee977f92820589cce4f7da9",
);

fn assert_single_published_baseline(directory: &std::path::Path, published: (&str, &str)) {
    let migrations = std::fs::read_dir(directory)
        .expect("read migrations")
        .map(|entry| entry.expect("migration entry"))
        .filter(|entry| entry.path().extension() == Some(std::ffi::OsStr::new("sql")))
        .collect::<Vec<_>>();
    assert_eq!(
        migrations.len(),
        1,
        "{} must contain one baseline",
        directory.display()
    );
    let migration = &migrations[0];
    let name = migration.file_name().to_string_lossy().into_owned();
    assert_eq!(name, published.0);
    let bytes = std::fs::read(migration.path()).expect("read baseline");
    let actual = format!("{:x}", sha2::Sha256::digest(bytes));
    assert_eq!(
        actual, published.1,
        "published baseline {name} was modified"
    );
}

#[test]
fn postgres_has_one_immutable_baseline() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    assert_single_published_baseline(&directory, PUBLISHED_POSTGRES_BASELINE);
}

#[test]
fn sqlite_has_one_immutable_baseline() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations/sqlite");
    assert_single_published_baseline(&directory, PUBLISHED_SQLITE_BASELINE);
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

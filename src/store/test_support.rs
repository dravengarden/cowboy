//! Store fixtures for contracts that must hold on both backends. A contract
//! is an `async fn(&Store)`; each backend gets a thin test that opens its
//! fixture and runs it, and the PostgreSQL twin is named `postgres_*` so
//! `just test-postgres` runs it in an isolated empty database.

use super::Store;

/// A migrated store plus the directory that owns its artifacts. Dropping it
/// closes the pool before the directory is removed.
pub(crate) struct TestStore {
    pub(crate) store: Store,
    _root: tempfile::TempDir,
}

/// An in-memory SQLite store.
pub(crate) async fn sqlite() -> TestStore {
    open("sqlite::memory:").await
}

/// The isolated PostgreSQL database `just test-postgres` provisions for this
/// test. Panics instead of passing when the fixture is absent.
pub(crate) async fn postgres() -> TestStore {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    open(&url).await
}

async fn open(url: &str) -> TestStore {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect(url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    TestStore { store, _root: root }
}

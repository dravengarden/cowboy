//! The passkey Plugin round trip, finished against the Plugin runtime.

use std::path::{Path, PathBuf};

use cowboy_store::core_passkeys::STORAGE_CAPABILITY;
use cowboy_store::core_passkeys::tests::{
    assert_passkey_plugin_round_trip_and_import, unlock_fixture_directories,
};
use cowboy_store::plugin_dir::PluginDir;
use cowboy_store::store::Store;

use crate::plugin_runtime::PluginRuntime;

async fn bootstrap_hosts((store, root): (Store, PathBuf)) {
    assert_bootstrap_runtime_hosts(&store, &root).await;
    drop(store);
    unlock_fixture_directories(&root);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn sqlite_passkey_plugin_round_trip_and_import() {
    bootstrap_hosts(assert_passkey_plugin_round_trip_and_import(None).await).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_passkey_plugin_round_trip_and_import() {
    let url =
        std::env::var("COWBOY_TEST_POSTGRES_URL").expect("run nix develop -c just test-postgres");
    bootstrap_hosts(assert_passkey_plugin_round_trip_and_import(Some(&url)).await).await;
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

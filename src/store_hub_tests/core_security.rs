//! Core-security fixtures checked against Controller host preparation.

use cowboy_store::core_security::tests::{Fixture, fresh_round_trip, write_config};

/// Omitting both the core-security config and the database must not start an
/// in-memory Controller, and a database alone is not enough either.
fn preparation_refuses(fixture: Fixture) {
    let mut args = crate::serve_args::ServeArgs::test_plugin_check(fixture.root.path());
    assert!(
        crate::plugin_activation::prepare_controller_hosts(&args).is_err(),
        "omitting both config and DB must not start in-memory"
    );
    args.database_url = Some(fixture.url.clone());
    assert!(crate::plugin_activation::prepare_controller_hosts(&args).is_err());
}

#[test]
fn core_preflight_with_empty_catalog_needs_no_local_auth_plugin_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("core.json");
    write_config(&path, "fresh");
    let data = root.path().join("absent-data");
    let mut args = crate::serve_args::ServeArgs::test_plugin_check(&data);
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

#[tokio::test]
async fn sqlite_core_fresh_round_trip_without_plugins() {
    preparation_refuses(fresh_round_trip(None).await);
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_core_fresh_round_trip_without_plugins() {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL gate required");
    preparation_refuses(fresh_round_trip(Some(&url)).await);
}

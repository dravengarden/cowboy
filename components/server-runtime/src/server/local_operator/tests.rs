use super::*;
use std::io::Write as _;

#[tokio::test]
#[allow(clippy::used_underscore_binding)] // Inspect only the private ownership guard.
async fn endpoint_owner_releases_shared_description_without_unlocking_a_successor() {
    let root = tempfile::tempdir().unwrap();
    let directory = directory(root.path()).unwrap();
    let live = Arc::new(AtomicBool::new(true));
    let (listener, owner) = bind_socket(&directory, live.clone()).unwrap();
    // dup and pre-exec fork inheritance retain the same flock description.
    let inherited = owner._lock.0.try_clone().unwrap();
    assert!(bind_socket(&directory, Arc::new(AtomicBool::new(true))).is_err());
    drop(listener);
    drop(owner);
    assert!(!live.load(Ordering::Acquire));
    let (listener, owner) = bind_socket(&directory, Arc::new(AtomicBool::new(true))).unwrap();
    drop(inherited);
    assert!(bind_socket(&directory, Arc::new(AtomicBool::new(true))).is_err());
    drop(listener);
    drop(owner);
}

#[tokio::test]
async fn endpoint_ownership_prevents_unlinking_live_or_foreign_paths_and_revokes_on_exit() {
    let root = tempfile::tempdir().unwrap();
    let directory = directory(root.path()).unwrap();
    let path = directory.join(SOCKET_NAME);
    let live = Arc::new(AtomicBool::new(true));
    let (listener, owner) = bind_socket(&directory, live.clone()).unwrap();
    let inode = std::fs::symlink_metadata(&path).unwrap().ino();
    assert!(bind_socket(&directory, Arc::new(AtomicBool::new(true))).is_err());
    assert_eq!(std::fs::symlink_metadata(&path).unwrap().ino(), inode);
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    drop(listener);
    drop(owner);
    assert!(!live.load(Ordering::Acquire));
    assert!(!path.exists());
    // A crash can leave an owned socket; the next exclusive owner may replace it.
    drop(UnixListener::bind(&path).unwrap());
    let (listener, owner) = bind_socket(&directory, Arc::new(AtomicBool::new(true))).unwrap();
    // Cleanup must not remove a path that somebody replaced after our bind.
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"foreign file").unwrap();
    drop(listener);
    drop(owner);
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign file");
    assert!(bind_socket(&directory, Arc::new(AtomicBool::new(true))).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign file");
}

#[tokio::test]
async fn unix_identity_requires_a_private_grant_and_ignores_supplied_credentials() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root = tempfile::tempdir().unwrap();
    let directory = directory(root.path()).unwrap();
    let socket = directory.join("fixture.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let live = Arc::new(AtomicBool::new(true));
    let app = Router::new()
        .route(
            "/scope",
            get(|Extension(grant): Extension<Arc<Grant>>| async move {
                Json(serde_json::json!({"actor":grant.actor()}))
            }),
        )
        .layer(middleware::from_fn_with_state(
            Boundary {
                directory: directory.clone(),
                live: live.clone(),
            },
            authenticate,
        ));
    let server = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<Peer>())
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .unix_socket(socket)
        .no_proxy()
        .build()
        .unwrap();
    let request = || {
        client
            .get("http://localhost/scope")
            .header("x-cowboy-operator", "root")
            .header("authorization", "Bearer not-a-grant")
            .header("cookie", "cowboy_admin=not-a-grant")
    };
    assert_eq!(
        request().send().await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let path = directory.join("grant.json");
    let mut file = private_file(&path, true).unwrap();
    file.write_all(
        &serde_json::to_vec(&serde_json::json!({"schema":1,"generation":"a".repeat(64)})).unwrap(),
    )
    .unwrap();
    let response = request().send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["actor"]["account"],
        format!("unix-uid:{}", uid())
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        request().send().await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    live.store(false, Ordering::Release);
    assert_eq!(
        request().send().await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    server.abort();
    let _ = server.await;
}

use super::{
    is_admin_console_path, plugin_release_artifact_response, resolve_static_document,
    static_content_type,
};
use axum::http::{StatusCode, header};

#[tokio::test]
async fn successful_shell_reads_retain_lazy_chunks_after_the_next_web_release() {
    let dir = tempfile::tempdir().unwrap();
    let roots = ["a", "b", "c"].map(|name| dir.path().join(name));
    for root in &roots {
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(
            root.join("index.html"),
            root.file_name().unwrap().as_encoded_bytes(),
        )
        .unwrap();
    }
    std::fs::write(roots[1].join("assets/Steer-b.js"), b"previous shortcut").unwrap();
    let link = dir.path().join("web");
    std::os::unix::fs::symlink(&roots[0], &link).unwrap();
    let history = super::web_roots::WebRoots::new(link.clone());
    let misses_only = super::web_roots::WebRoots::new(link.clone());
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&roots[1], &link).unwrap();
    // B serves normally. No missing asset asks for fallback yet.
    assert_eq!(
        super::resolve_web_document(&history, "index.html")
            .await
            .unwrap()
            .1,
        b"b"
    );
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&roots[2], &link).unwrap();
    assert_eq!(
        super::resolve_web_document(&history, "index.html")
            .await
            .unwrap()
            .1,
        b"c"
    );
    assert!(
        super::web_roots::previous_release_asset(&misses_only, "assets/Steer-b.js")
            .await
            .is_none(),
        "observing only misses loses the successfully served intermediate release"
    );
    assert_eq!(
        super::resolve_web_document(&history, "assets/Steer-b.js")
            .await
            .unwrap()
            .1,
        b"previous shortcut"
    );
    assert_eq!(
        super::resolve_web_document(&history, "assets/unknown.js")
            .await
            .unwrap_err(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn published_artifacts_in_legacy_catalog_keep_streaming_and_cache_headers() {
    let root = std::env::temp_dir().join(format!("cowboy-artifact-http-{}", uuid::Uuid::new_v4()));
    let bytes = b"immutable published runtime";
    let digest = crate::admin::hex_sha256(bytes);
    let directory = root.join("plugin-catalog/artifacts").join(&digest);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("runtime.tar.gz"), bytes).unwrap();
    let catalog = crate::plugin_catalog::PluginCatalog::open(&root, None).unwrap();
    let response = plugin_release_artifact_response(&catalog, &digest, "runtime.tar.gz").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_LENGTH],
        bytes.len().to_string()
    );
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        response.headers()[header::ETAG],
        format!("\"sha256:{digest}\"")
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .as_ref(),
        bytes
    );
    for (digest, name) in [
        (digest.as_str(), "missing.tar.gz"),
        ("invalid", "runtime.tar.gz"),
        (digest.as_str(), "../secret"),
    ] {
        assert_eq!(
            plugin_release_artifact_response(&catalog, digest, name)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert!(!root.join("plugins/catalog/artifacts").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn admin_console_paths_are_distinct_from_the_session_spa() {
    assert!(is_admin_console_path("admin"));
    assert!(is_admin_console_path("admin/accounts"));
    assert!(!is_admin_console_path("admin.html"));
    assert!(!is_admin_console_path("index.html"));
    assert!(!is_admin_console_path("sessions"));
}

#[test]
fn apple_app_site_association_is_served_as_json() {
    assert_eq!(
        static_content_type(".well-known/apple-app-site-association"),
        "application/json"
    );
    assert_eq!(static_content_type("index.html"), "text/html");
}

#[tokio::test]
async fn admin_paths_serve_admin_html_not_the_session_spa() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-admin-static-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("index.html"), b"session-spa").unwrap();
    std::fs::write(root.join("admin.html"), b"admin-console").unwrap();

    let (name, body) = resolve_static_document(&root, "admin").await.unwrap();
    assert_eq!(name, "admin.html");
    assert_eq!(body, b"admin-console");

    let (name, body) = resolve_static_document(&root, "admin/accounts")
        .await
        .unwrap();
    assert_eq!(name, "admin.html");
    assert_eq!(body, b"admin-console");

    let (name, body) = resolve_static_document(&root, "index.html").await.unwrap();
    assert_eq!(name, "index.html");
    assert_eq!(body, b"session-spa");

    let (name, body) = resolve_static_document(&root, "sessions").await.unwrap();
    assert_eq!(name, "index.html");
    assert_eq!(body, b"session-spa");

    std::fs::remove_dir_all(&root).unwrap();
}

#[tokio::test]
async fn missing_admin_html_is_not_the_session_spa() {
    let root = std::env::temp_dir().join(format!(
        "cowboy-admin-static-missing-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("index.html"), b"session-spa").unwrap();

    let status = resolve_static_document(&root, "admin").await.unwrap_err();
    assert_eq!(status, StatusCode::NOT_FOUND);

    std::fs::remove_dir_all(&root).unwrap();
}

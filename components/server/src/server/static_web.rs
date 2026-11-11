//! Static SPA documents, content types and artifact downloads.

use super::*;

pub(super) async fn api_web_push_config(State(state): State<Arc<AppState>>) -> Response {
    Json(WebPushConfigResponse {
        application_server_key: state.web_push.public_key(),
    })
    .into_response()
}

/// Render the first 16 bytes of SHA256 as a compact content identifier. Used
/// by both static-asset ETags and `/version` so the two cannot drift.
pub(super) fn content_hash(content: &[u8]) -> String {
    let hash: [u8; 32] = Sha256::digest(content).into();
    format!(
        "{:016x}{:016x}",
        u64::from_be_bytes(hash[0..8].try_into().expect("8 bytes")),
        u64::from_be_bytes(hash[8..16].try_into().expect("8 bytes")),
    )
}

pub(super) async fn api_artifact(
    State(state): State<Arc<AppState>>,
    Extension(_authenticated): Extension<AuthenticatedProductRequest>,
    Path(name): Path<String>,
) -> Response {
    let Some(path) = state
        .store
        .as_ref()
        .and_then(|store| store.artifact_path(&name))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let content_type = mime_guess::from_path(&name)
                .first_or_octet_stream()
                .to_string();
            (
                [
                    (header::CONTENT_TYPE, content_type),
                    (
                        header::CACHE_CONTROL,
                        "private, max-age=31536000, immutable".to_owned(),
                    ),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        web_security::UNTRUSTED_BYTES_CSP.to_owned(),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(error) => {
            tracing::warn!(%error, artifact = %name, "reading artifact failed");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

pub(super) async fn plugin_release_artifact(
    State(state): State<Arc<AppState>>,
    Path((digest, name)): Path<(String, String)>,
) -> Response {
    plugin_release_artifact_response(&state.plugin_catalog, &digest, &name).await
}

pub(super) async fn plugin_release_artifact_response(
    catalog: &crate::plugin_catalog::PluginCatalog,
    digest: &str,
    name: &str,
) -> Response {
    let Some(path) = catalog.published_artifact_path(digest, name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) => {
            tracing::info!(%error, artifact = %path.display(), "Plugin artifact is unavailable");
            return StatusCode::NOT_FOUND.into_response();
        }
    };
    let metadata = match file.metadata().await {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(%error, artifact = %path.display(), "reading Plugin artifact metadata failed");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    let digest = digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .to_ascii_lowercase();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header("x-content-type-options", "nosniff")
        .header(header::CONTENT_LENGTH, metadata.len())
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(header::ETAG, format!("\"sha256:{digest}\""))
        .body(Body::from_stream(ReaderStream::new(file)))
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "building Plugin artifact response failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
}

/// Resolve the on-disk document for a static request.
///
/// `/admin` and `/admin/*` never fall back to the session PWA `index.html`.
/// When `admin.html` is present they return that document; otherwise 404.
pub(super) async fn resolve_web_document(
    roots: &web_roots::WebRoots,
    requested: &str,
) -> Result<(String, Vec<u8>), StatusCode> {
    // Pin this request to the observed target. A Web activation may retarget
    // the link while the asynchronous read is in flight.
    let document = match roots.observe() {
        Some(root) => resolve_static_document(&root, requested).await,
        None => Err(StatusCode::NOT_FOUND),
    };
    match document {
        Err(status) if requested.starts_with("assets/") => {
            match web_roots::previous_release_asset(roots, requested).await {
                Some(content) => {
                    tracing::info!(requested, "served a web asset from the previous release");
                    Ok((requested.to_owned(), content))
                }
                None => Err(status),
            }
        }
        document => document,
    }
}

pub(super) async fn resolve_static_document(
    web_root: &FsPath,
    requested: &str,
) -> Result<(String, Vec<u8>), StatusCode> {
    let requested_path = web_root.join(requested);
    match tokio::fs::read(&requested_path).await {
        Ok(bytes) => Ok((requested.to_owned(), bytes)),
        Err(_) if is_admin_console_path(requested) => {
            match tokio::fs::read(web_root.join("admin.html")).await {
                Ok(bytes) => Ok(("admin.html".to_owned(), bytes)),
                Err(_) => Err(StatusCode::NOT_FOUND),
            }
        }
        Err(_) if requested.starts_with("assets/") => Err(StatusCode::NOT_FOUND),
        Err(_) => match tokio::fs::read(web_root.join("index.html")).await {
            Ok(bytes) => Ok(("index.html".to_owned(), bytes)),
            Err(_) => Err(StatusCode::NOT_FOUND),
        },
    }
}

pub(super) fn static_content_type(name: &str) -> String {
    if name == ".well-known/apple-app-site-association" {
        "application/json".to_owned()
    } else {
        mime_guess::from_path(name)
            .first_or_octet_stream()
            .to_string()
    }
}

/// Serve a separately deployed asset by path, falling back to `index.html` so
/// the SPA owns client-side routing. Missing hashed assets never fall back: a
/// module request must receive a real 404 rather than HTML with a 200 status.
/// Missing `index.html` (UI not built) → 404.
/// `/admin` and `/admin/*` serve `admin.html` and never the session shell.
///
/// Caching: a per-file SHA256 is used as a content `ETag` (stable across
/// rollouts when the bytes are unchanged). The
/// cache policy is split by whether the filename is content-addressed:
///   - `/assets/*` — Vite emits content-hashed names, so the bytes behind a name
///     never change → `immutable` with a one-year max-age, never revalidated.
///   - everything else (index.html, admin.html, sw.js, manifest, favicon, icons) —
///     `no-cache`: the browser may store it but MUST revalidate via the `ETag` on
///     every use, so a redeploy is picked up immediately while unchanged files
///     cost only a 304. This is what stops a redeployed favicon/icon from being
///     pinned to a stale copy in the browser's HTTP cache.
pub(super) async fn static_handler(
    State(state): State<Arc<AppState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    if requested.starts_with("app-shell") {
        return app_shell(&state, requested, uri.query().is_some()).await;
    }
    if requested.starts_with("mcp-app/") || requested == "mcp-app" {
        return super::mcp_bindings::apps::serve(
            &state,
            requested,
            uri.query().is_some(),
            &headers,
        );
    }

    // Never allow a URI to escape the configured asset root. Percent-encoded
    // traversal remains a literal filename at this layer and is harmless.
    if !FsPath::new(requested)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Serve the asset if it exists; otherwise fall back to index.html so the
    // SPA handles the route. Admin routes never take that fallback. Vite's
    // /assets names are content-addressed files, never client-side routes.
    // Returning index.html for a missing old chunk makes browsers report the
    // opaque "Importing a module script failed" error because a JS import
    // received text/html.
    let requested_path = state.web_root.join(requested);
    let (name, content) = match resolve_web_document(&state.web_roots, requested).await {
        Ok(document) => document,
        Err(status) if is_admin_console_path(requested) => {
            tracing::info!(
                requested = %requested_path.display(),
                "admin console is not in the web root"
            );
            return status.into_response();
        }
        Err(status) if requested.starts_with("assets/") => {
            tracing::info!(
                requested = %requested_path.display(),
                "requested web asset is no longer deployed"
            );
            return status.into_response();
        }
        Err(status) => {
            tracing::warn!(
                requested = %requested_path.display(),
                "reading web asset failed"
            );
            return (status, "UI not built").into_response();
        }
    };

    // Content ETag uses the same hash function as `/version`.
    let etag = format!("\"{}\"", content_hash(&content));

    // Conditional request: the browser echoes our ETag in If-None-Match; if it
    // still matches, skip the body. `contains` (not strict equality) tolerates a
    // comma-list or a `W/` weak prefix some clients send.
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        && inm.contains(etag.as_str())
    {
        return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag.as_str())]).into_response();
    }

    let cache_control = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        // `no-store`, NOT `no-cache`. iOS WKWebView (the native Tauri shell) caches
        // the HTML shell + sw.js under NSURLCache and serves it stale even with
        // `no-cache`/ETag revalidation — so a redeploy never reached the app until
        // a manual cache wipe. `no-store` forbids storing it at all, so every cold
        // start re-fetches index.html → its new hashed asset url → the fresh
        // bundle. The files this covers are tiny (HTML, sw.js, manifest, icons).
        "no-store"
    };
    let content_type = static_content_type(&name);
    if name == "passkey.html" {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type.as_str())
            .header(header::CACHE_CONTROL, cache_control)
            .header(header::ETAG, etag.as_str())
            .header("referrer-policy", "no-referrer")
            .header("x-content-type-options", "nosniff")
            .header("x-frame-options", "DENY")
            .header(
                "content-security-policy",
                "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                 connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            )
            .header(
                "permissions-policy",
                "publickey-credentials-create=(self), publickey-credentials-get=(self)",
            )
            .body(Body::from(content))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }
    let document_csp = name.ends_with(".html").then(|| {
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok());
        web_security::app_document_csp(&content, host)
    });
    let mut response = (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache_control.to_owned()),
            (header::ETAG, etag),
        ],
        Body::from(content),
    )
        .into_response();
    web_security::harden_static_response(response.headers_mut(), document_csp.as_deref());
    response
}

/// `GET /app-shell.html`, exactly (docs/widget-apps-dom.md §3). Any other
/// path below it, any query string and a missing shell get an empty 404, never
/// the SPA document: a frame that navigated itself with data must find nothing.
async fn app_shell(state: &AppState, requested: &str, query: bool) -> Response {
    if requested != "app-shell.html" || query {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(root) = state.web_roots.observe() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(html) = tokio::fs::read(root.join("app-shell.html")).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Response::builder().status(StatusCode::OK);
    for (name, value) in web_security::app_shell_headers(&html) {
        response = response.header(name, value);
    }
    response
        .body(Body::from(html))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

//! Code plane handlers: repository, changes, diffs, files, search and
//! language intelligence for a session's workspace.

use super::*;

mod search;
pub(super) use search::{ContentSearchParams, content_search, routes as search_routes};

/// Return one filesystem directory page for the mobile review tree.
///
/// The root is resolved from the session rather than a client-provided root.
/// Relative paths are validated and cannot escape it. Gitignored children are
/// intentionally visible here because they may be independent repositories;
/// Git Changes remains scoped to the owning repository.
pub(super) async fn api_file_tree(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<FileTreeQuery>,
    headers: HeaderMap,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let limit = query.limit.clamp(20, 500);
        let path = query.path;
        let requested_path = path.clone();
        let remote_page = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::Directory {
                path: path.clone(),
                limit,
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Directory(page))) => Some(page),
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote directory unavailable").into_response();
            }
            Ok(None) => None,
        };
        if let Some(page) = remote_page {
            let entries = page
                .entries
                .into_iter()
                .map(|entry| FileTreeEntry {
                    name: entry.name,
                    path: entry.path,
                    kind: if entry.is_directory {
                        "directory"
                    } else {
                        "file"
                    },
                    ignored: entry.ignored,
                })
                .collect::<Vec<_>>();
            let revision = file_tree_revision(&requested_path, &entries, page.truncated);
            let body = serde_json::to_vec(&FileTreeResponse {
                api_version: 1,
                path: requested_path,
                revision: revision.clone(),
                entries,
                truncated: page.truncated,
            })
            .expect("file tree response serializes");
            return file_tree_http_response(&headers, &revision, body);
        }
        let cache = state.code_cache.clone();
        let cache_root = cwd.clone();
        let cache_path = requested_path.clone();
        let cached = tokio::task::spawn_blocking(move || {
            cache.get_directory(FsPath::new(&cache_root), &cache_path, limit)
        })
        .await;
        if let Ok(Ok(Some(cached))) = cached {
            return file_tree_http_response(&headers, &cached.revision, cached.bytes);
        }
        let scan_root = cwd.clone();
        let result = tokio::task::spawn_blocking(move || {
            crate::code_review::LocalCodeProvider::new(scan_root).directory(&path, limit)
        })
        .await;
        let Ok(Ok(page)) = result else {
            return (StatusCode::BAD_REQUEST, "invalid directory").into_response();
        };
        let entries = page
            .entries
            .into_iter()
            .map(|entry| FileTreeEntry {
                name: entry.name,
                path: entry.path,
                kind: if entry.is_directory {
                    "directory"
                } else {
                    "file"
                },
                ignored: entry.ignored,
            })
            .collect::<Vec<_>>();
        let truncated = page.truncated;
        let revision = file_tree_revision(&requested_path, &entries, truncated);
        let body = serde_json::to_vec(&FileTreeResponse {
            api_version: 1,
            path: requested_path.clone(),
            revision: revision.clone(),
            entries,
            truncated,
        })
        .expect("file tree response serializes");
        let cache = state.code_cache.clone();
        let cache_revision = revision.clone();
        let cache_body = body.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                cache.put_directory(
                    FsPath::new(&cwd),
                    &requested_path,
                    limit,
                    &cache_revision,
                    &cache_body,
                )
            })
            .await;
            if let Ok(Err(error)) = result {
                tracing::warn!(%error, "persisting lazy directory cache failed");
            } else if let Err(error) = result {
                tracing::warn!(%error, "lazy directory cache task failed");
            }
        });
        file_tree_http_response(&headers, &revision, body)
    })
    .await
}

pub(super) fn session_code_context(
    state: &AppState,
    session_id: &str,
) -> Option<SessionCodeContext> {
    let scope = state.hub.session_code_scope(session_id)?;
    Some(SessionCodeContext {
        machine_id: scope.machine_id().into(),
        cwd: scope.cwd().into(),
        scope,
    })
}

pub(super) async fn code_context_is_current(
    state: &AppState,
    id: &str,
    scope: &CodeReadScope,
) -> bool {
    match scope {
        CodeReadScope::Session(scope) => {
            code_reads::session::current(&state.hub, &state.machine_control, scope)
        }
        CodeReadScope::Workspace(workspace) => {
            resolve_code_context(state, id)
                .await
                .is_some_and(|current| &current.scope == scope)
                && state.machine_control.workspace_scope_is_current(workspace)
        }
    }
}

pub(super) async fn api_code_manifest(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd.clone();
        let language_ready = if matches!(context.scope, CodeReadScope::Session(_)) {
            match ensure_zed_worktree_for_session(&state, &context).await {
                Ok(ready) => ready,
                Err(error) => {
                    tracing::warn!(session = %session_id, %error, "Zed adapter unavailable");
                    false
                }
            }
        } else {
            false
        };
        let manifest = match remote_code_request(&state, &context.scope, CodeOperation::Manifest)
            .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Manifest(manifest))) => manifest,
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote worktree unavailable").into_response();
            }
            Ok(None) => {
                let result = tokio::task::spawn_blocking(move || {
                    crate::code_review::LocalCodeProvider::new(cwd).manifest()
                })
                .await;
                let Ok(Ok(manifest)) = result else {
                    return (StatusCode::UNPROCESSABLE_ENTITY, "worktree unavailable")
                        .into_response();
                };
                manifest
            }
        };
        let restricted = match &context.scope {
            CodeReadScope::Session(scope) => !state
                .hub
                .language_servers_allowed(scope.session().session_id()),
            _ => false,
        };
        let language_state = if language_ready {
            "ready"
        } else if restricted {
            "restricted"
        } else {
            "unavailable"
        };
        let buffer_mode = code_buffers::review_mode(&state.machine_control, &context.scope);
        // Capability fields are part of this cached representation. Bump the
        // contract tag whenever that shape grows so installed Mobile clients do
        // not retain an older 304-backed manifest after a deploy.
        let etag = format!(
            "\"code-manifest-v4-{}-{language_state}-{buffer_mode:?}\"",
            manifest.revision
        );
        const MANIFEST_CACHE_CONTROL: &str = "private, max-age=0, must-revalidate";
        if headers
            .get(header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains(etag.as_str()))
        {
            return (
                StatusCode::NOT_MODIFIED,
                [
                    (header::ETAG, etag.as_str()),
                    (header::CACHE_CONTROL, MANIFEST_CACHE_CONTROL),
                ],
            )
                .into_response();
        }
        let mut response = Json(CodeManifestResponse {
            api_version: 1,
            provider: manifest.provider,
            revision: manifest.revision,
            head: manifest.head,
            project: manifest.project,
            branch: manifest.branch,
            worktree: manifest.worktree,
            change_count: manifest.change_count,
            buffer_mode,
            language: CodeLanguageCapabilities {
                provider: if language_ready { "language" } else { "none" },
                state: language_state,
                diagnostics: language_ready,
                inlay_hints: language_ready,
                semantic_tokens: language_ready,
                hover: language_ready,
                navigation: language_ready,
                outline: language_ready,
            },
        })
        .into_response();
        response
            .headers_mut()
            .insert(header::ETAG, etag.parse().expect("SHA256 ETag is valid"));
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static(MANIFEST_CACHE_CONTROL),
        );
        response
    })
    .await
}

pub(super) async fn api_code_changes(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeChangesQuery>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let comparison = query.comparison;
        let mut fetch_available = false;
        let result = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::Changes {
                comparison: comparison.clone(),
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Changes(changes))) => changes,
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote git changes unavailable").into_response();
            }
            Ok(None) => {
                fetch_available = matches!(context.scope, CodeReadScope::Session(_));
                let result = tokio::task::spawn_blocking(move || {
                    crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                        .changes(comparison.as_deref())
                })
                .await;
                let Ok(Ok(changes)) = result else {
                    return (StatusCode::UNPROCESSABLE_ENTITY, "git changes unavailable")
                        .into_response();
                };
                changes
            }
        };
        Json(CodeChangesResponse {
            api_version: 1,
            head: result.head,
            revision: result.revision,
            changes: result
                .changes
                .into_iter()
                .map(|change| CodeChangeResponse {
                    path: change.path,
                    old_path: change.old_path,
                    staged: change.staged,
                    unstaged: change.unstaged,
                    fingerprint: change.fingerprint,
                    additions: change.additions,
                    deletions: change.deletions,
                    status: match change.status {
                        crate::code_review::ChangeStatus::Modified => "modified",
                        crate::code_review::ChangeStatus::Added => "added",
                        crate::code_review::ChangeStatus::Deleted => "deleted",
                        crate::code_review::ChangeStatus::Renamed => "renamed",
                        crate::code_review::ChangeStatus::Untracked => "untracked",
                        crate::code_review::ChangeStatus::Conflicted => "conflicted",
                    },
                })
                .collect(),
            truncated: result.truncated,
            comparison: result.comparison,
            default_comparison: result.default_comparison,
            comparisons: result.comparisons,
            committed_comparisons: result.committed_comparisons,
            fetched_at_ms: result.fetched_at_ms,
            fetch_available,
        })
        .into_response()
    })
    .await
}

pub(super) async fn api_code_repository(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeRepositoryQuery>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let after = query.after;
        let result = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::Repository {
                after: after.clone(),
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Repository(repository))) => {
                repository
            }
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote git history unavailable").into_response();
            }
            Ok(None) => {
                let result = tokio::task::spawn_blocking(move || {
                    crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                        .repository(after.as_deref())
                })
                .await;
                let Ok(Ok(repository)) = result else {
                    return (StatusCode::UNPROCESSABLE_ENTITY, "git history unavailable")
                        .into_response();
                };
                repository
            }
        };
        Json(CodeRepositoryResponse {
            api_version: 1,
            commits: result.commits,
            history_truncated: result.history_truncated,
            worktrees: result.worktrees,
            target: result.target,
        })
        .into_response()
    })
    .await
}

pub(super) async fn api_code_commit(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeCommitQuery>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let oid = query.oid;
        let result = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::Commit { oid: oid.clone() },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Commit(commit))) => commit,
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote commit unavailable").into_response();
            }
            Ok(None) => {
                let result = tokio::task::spawn_blocking(move || {
                    crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd)).commit(&oid)
                })
                .await;
                let Ok(Ok(commit)) = result else {
                    return (StatusCode::UNPROCESSABLE_ENTITY, "commit unavailable")
                        .into_response();
                };
                commit
            }
        };
        Json(CodeCommitResponse {
            api_version: 1,
            commit: result,
        })
        .into_response()
    })
    .await
}

pub(super) async fn api_code_commit_diff(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeCommitDiffQuery>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let oid = query.oid;
        let path = query.path;
        let result = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::CommitDiff {
                oid: oid.clone(),
                path: path.clone(),
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::CommitDiff(diff))) => diff,
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote commit diff unavailable").into_response();
            }
            Ok(None) => {
                let commit_oid = oid.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                        .commit_diff(&commit_oid, &path)
                })
                .await;
                let Ok(Ok(diff)) = result else {
                    return (StatusCode::UNPROCESSABLE_ENTITY, "commit diff unavailable")
                        .into_response();
                };
                diff
            }
        };
        Json(CodeDiffResponse {
            api_version: 1,
            path: result.path,
            revision: oid,
            text: result.text,
            added: result.added,
            removed: result.removed,
            truncated: result.truncated,
            next_cursor: None,
            limited: result.truncated,
        })
        .into_response()
    })
    .await
}

pub(super) async fn api_code_diff(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeDiffQuery>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let owner = context.scope;
        if let Some(cursor) = query.cursor.as_deref() {
            let page = state.diff_snapshots.next_page(&owner, cursor).await;
            return match page {
                Ok(page) => Json(CodeDiffResponse {
                    api_version: 1,
                    path: page.path,
                    revision: page.revision,
                    text: page.text,
                    added: page.added,
                    removed: page.removed,
                    truncated: page.next_cursor.is_some() || page.limited,
                    next_cursor: page.next_cursor,
                    limited: page.limited,
                })
                .into_response(),
                Err(error) if error == "diff snapshot expired" => {
                    (StatusCode::GONE, error).into_response()
                }
                Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
            };
        }
        let cwd = context.cwd;
        let Some(path) = query.path else {
            return (StatusCode::BAD_REQUEST, "path is required").into_response();
        };
        let scope = match query.scope {
            CodeDiffScope::Combined => crate::code_review::DiffScope::Combined,
            CodeDiffScope::Staged => crate::code_review::DiffScope::Staged,
            CodeDiffScope::Unstaged => crate::code_review::DiffScope::Unstaged,
        };
        let key = DiffSnapshotKey {
            owner,
            path: path.clone(),
            context: query.context,
            show_whitespace: query.show_whitespace,
            scope,
            comparison: query.comparison.clone(),
        };
        let remote_document = match remote_code_request(
            &state,
            &key.owner,
            CodeOperation::Diff {
                path: path.clone(),
                context: query.context,
                show_whitespace: query.show_whitespace,
                scope,
                comparison: query.comparison.clone(),
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Diff(document))) => Some(document),
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote diff unavailable").into_response();
            }
            Ok(None) => None,
        };
        let page = state
            .diff_snapshots
            .first_page(key, || async move {
                if let Some(document) = remote_document {
                    return Ok(document);
                }
                tokio::task::spawn_blocking(move || {
                    crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd)).diff_snapshot(
                        &path,
                        query.context,
                        query.show_whitespace,
                        scope,
                        query.comparison.as_deref(),
                    )
                })
                .await
                .map_err(|error| error.to_string())?
            })
            .await;
        let Ok(page) = page else {
            return (StatusCode::BAD_REQUEST, "diff unavailable").into_response();
        };
        Json(CodeDiffResponse {
            api_version: 1,
            path: page.path,
            revision: page.revision,
            text: page.text,
            added: page.added,
            removed: page.removed,
            truncated: page.next_cursor.is_some() || page.limited,
            next_cursor: page.next_cursor,
            limited: page.limited,
        })
        .into_response()
    })
    .await
}

pub(super) async fn api_code_file(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeFileQuery>,
    headers: HeaderMap,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        // Browser tokens bind the original context/path before local or remote I/O.
        let continuation =
            match state
                .file_cursors
                .resolve(&context.scope, &query.path, query.cursor.as_deref())
            {
                Ok(continuation) => continuation,
                Err(error) => return error.into_response(),
            };
        let requested_path = query.path.clone();
        let mut query = query;
        query.cursor = continuation
            .as_ref()
            .map(|cursor| cursor.native_cursor().to_owned());
        let cwd = context.cwd;
        let result = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::File {
                path: query.path.clone(),
                cursor: query.cursor.clone(),
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::File(file))) => Ok(file),
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote file unavailable").into_response();
            }
            Ok(None) => {
                let cache = state.code_cache.clone();
                let result = tokio::task::spawn_blocking(move || {
                    match cache.get_or_load(FsPath::new(&cwd), &query.path) {
                        Ok(Some(cached)) => {
                            debug_assert_eq!(cached.size, cached.bytes.len() as u64);
                            crate::code_review::cached_file_page(
                                &query.path,
                                cached.bytes,
                                cached.revision,
                                query.cursor.as_deref(),
                            )
                        }
                        // The cache is deliberately bounded to the physical session
                        // worktree. Registered aggregate projects are a read-only Code
                        // projection outside that root, so let the provider resolve and
                        // validate those paths instead of treating a cache miss as an
                        // authorization failure.
                        Ok(None) | Err(_) => {
                            crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                                .file_page(&query.path, query.cursor.as_deref())
                        }
                    }
                })
                .await;
                let Ok(result) = result else {
                    return (StatusCode::BAD_REQUEST, "file unavailable").into_response();
                };
                result
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(error) if error == "file snapshot changed" => {
                return (StatusCode::CONFLICT, error).into_response();
            }
            Err(error) => return code_file_error_response(&error),
        };
        match state.file_cursors.project(
            &context.scope,
            &requested_path,
            continuation.as_ref(),
            result,
        ) {
            Ok(page) => code_reads::file_pages::response(&headers, page),
            Err(error) => error.into_response(),
        }
    })
    .await
}

pub(super) async fn api_code_file_raw(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeFileQuery>,
    headers: HeaderMap,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let result = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::FileRaw {
                path: query.path.clone(),
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::FileRaw(file))) => Ok(file),
            Ok(Some(_)) => {
                return (StatusCode::BAD_GATEWAY, "remote file unavailable").into_response();
            }
            Err(_) => return (StatusCode::BAD_GATEWAY, "remote file unavailable").into_response(),
            Ok(None) => {
                let cache = state.code_cache.clone();
                let path = query.path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let media_type = crate::code_review::preview_media_type(&path)
                        .ok_or_else(|| "file is not a previewable media type".to_owned())?;
                    match cache.get_or_load(FsPath::new(&cwd), &path) {
                        Ok(Some(cached)) => Ok(crate::code_review::RawFileDocument {
                            path: path.replace('\\', "/"),
                            revision: cached.revision,
                            media_type: media_type.to_owned(),
                            bytes: cached.bytes,
                            size: cached.size,
                        }),
                        Ok(None) | Err(_) => {
                            crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
                                .file_raw(&path)
                        }
                    }
                })
                .await;
                let Ok(result) = result else {
                    return (StatusCode::BAD_REQUEST, "file unavailable").into_response();
                };
                result
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(error) if error == "file snapshot changed" => {
                return (StatusCode::CONFLICT, error).into_response();
            }
            Err(error) => return code_file_error_response(&error),
        };
        let etag = format!("\"{}\"", result.revision);
        const FILE_CACHE_CONTROL: &str = "private, max-age=0, must-revalidate";
        if headers
            .get(header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains(etag.as_str()))
        {
            return (
                StatusCode::NOT_MODIFIED,
                [
                    (header::ETAG, etag.as_str()),
                    (header::CACHE_CONTROL, FILE_CACHE_CONTROL),
                ],
            )
                .into_response();
        }
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, result.media_type),
                (header::ETAG, etag),
                (header::CACHE_CONTROL, FILE_CACHE_CONTROL.to_owned()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
                (
                    header::CONTENT_SECURITY_POLICY,
                    web_security::UNTRUSTED_BYTES_CSP.to_owned(),
                ),
            ],
            result.bytes,
        )
            .into_response()
    })
    .await
}

pub(super) fn code_file_error_response(error: &str) -> Response {
    match error {
        "file not found" => (StatusCode::NOT_FOUND, "file not found").into_response(),
        "binary file" | "file is not UTF-8" | "file is not a previewable media type" => (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "file is not previewable text",
        )
            .into_response(),
        _ => (StatusCode::BAD_REQUEST, "file unavailable").into_response(),
    }
}

pub(super) async fn api_code_buffer_open(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<CodeBufferLeaseRequest>,
) -> Response {
    api_code_buffer_lease(state, session_id, request, true).await
}

pub(super) async fn api_code_language(
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeLanguageQuery>,
) -> Response {
    code_reads::language::read(
        authority,
        &session_id,
        &query.path,
        code_reads::language::Query::Language,
    )
    .await
}

pub(super) async fn api_code_hover(
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeHoverQuery>,
) -> Response {
    code_reads::language::read(
        authority,
        &session_id,
        &query.path,
        code_reads::language::Query::Hover {
            row: query.row,
            column: query.column,
        },
    )
    .await
}

pub(super) async fn api_code_navigation(
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeNavigationQuery>,
) -> Response {
    code_reads::language::read(
        authority,
        &session_id,
        &query.path,
        code_reads::language::Query::Navigation {
            row: query.row,
            column: query.column,
            kind: query.kind,
        },
    )
    .await
}

pub(super) async fn api_code_outline(
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<CodeLanguageQuery>,
) -> Response {
    code_reads::language::read(
        authority,
        &session_id,
        &query.path,
        code_reads::language::Query::Outline,
    )
    .await
}

pub(super) async fn api_code_buffer_close(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<CodeBufferLeaseRequest>,
) -> Response {
    api_code_buffer_lease(state, session_id, request, false).await
}

pub(super) async fn api_code_buffer_lease(
    state: Arc<AppState>,
    session_id: String,
    request: CodeBufferLeaseRequest,
    open: bool,
) -> Response {
    let Some(context) = session_code_context(&state, &session_id) else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if request.path.is_empty()
        || request.lease_id.is_empty()
        || request.lease_id.len() > 128
        || request.lease_id.chars().any(char::is_whitespace)
    {
        return (StatusCode::BAD_REQUEST, "invalid buffer lease").into_response();
    }
    let Some((worktree, path)) =
        zed_language_target(&context.machine_id, &context.cwd, &request.path)
    else {
        // Missing files, path escapes, and unregistered aggregate projections
        // are deterministic lease misses, not adapter outages. Registered
        // `projects/<name>/...` files lease against that checkout so hover can
        // run rust-analyzer there.
        return (StatusCode::UNPROCESSABLE_ENTITY, "buffer lease unavailable").into_response();
    };
    let response = zed_session::buffer_request(
        &state.hub,
        &state.machine_control,
        state.zed_adapter_socket.as_deref(),
        &context.scope,
        zed_session::BufferRequest {
            worktree: &worktree,
            path: &path,
            lease_id: &request.lease_id,
            open,
        },
    )
    .await;
    match response {
        Ok(ZedAdapterResponse::Buffer { path, leases, .. }) => Json(CodeBufferLeaseResponse {
            api_version: 1,
            path,
            leases,
        })
        .into_response(),
        Ok(_) => (
            StatusCode::BAD_GATEWAY,
            "unexpected language service response",
        )
            .into_response(),
        Err(zed_session::BufferError::Unavailable(error)) => {
            tracing::warn!(session = %session_id, %error, "Zed worktree unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "language service unavailable",
            )
                .into_response()
        }
        Err(zed_session::BufferError::Request(error)) => {
            tracing::warn!(
                session = %session_id,
                operation = if open { "open" } else { "close" },
                %error,
                "Zed buffer lease failed"
            );
            (
                if open {
                    StatusCode::UNPROCESSABLE_ENTITY
                } else {
                    StatusCode::CONFLICT
                },
                "buffer lease unavailable",
            )
                .into_response()
        }
    }
}

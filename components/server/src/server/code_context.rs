//! Workspaces, worktrees and code context resolution, and the Zed adapter
//! requests behind the code plane.

use super::*;

mod diagnostics;
pub(super) use diagnostics::{CodeDiagnostic, CodePoint};

pub(super) fn projected_work_items(columbus: &std::path::Path) -> Vec<WorkspaceWorkItem> {
    let output = std::process::Command::new("harness-cli")
        .args([
            "--root",
            &columbus.display().to_string(),
            "work-item",
            "list",
            "--format=json",
        ])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    serde_json::from_slice(&output.stdout).unwrap_or_default()
}

/// Resolve a registered project's current stable checkout through Columbus,
/// with a registry-backed fallback when the harness CLI is not on `PATH`.
/// Returns `None` for a registered project whose checkout has not been cloned.
pub(super) fn project_worktree(
    columbus: &std::path::Path,
    name: &str,
) -> Option<std::path::PathBuf> {
    crate::workspace::current_project_checkout(columbus, name)
}

/// `GET /api/workspaces` — the selectable session roots for the New Session
/// dialog: Columbus plus one entry per
/// columbus-managed project, read from `<workspace-root>/columbus/project-defs/*`
/// (the registry is the source of truth for which projects exist) and resolved
/// to each project's stable checkout. The selected Machine prepares an isolated
/// session worktree before the worker starts. The frontend keeps a fallback for when
/// this is unreachable.
pub(super) async fn api_workspaces(State(state): State<Arc<AppState>>) -> Response {
    let columbus = state.supervisor.workspace_root().join("columbus");
    let work_items = projected_work_items(&columbus);
    let mut out = vec![Workspace {
        value: "columbus".to_owned(),
        label: "columbus".to_owned(),
        help: columbus.display().to_string(),
        project: None,
        active_work_items: Vec::new(),
    }];
    if let Ok(entries) = std::fs::read_dir(columbus.join("project-defs")) {
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "schema" && n != "cue.mod")
            .collect();
        names.sort();
        for name in names {
            if let Some(dir) = project_worktree(&columbus, &name) {
                let path = dir.display().to_string();
                out.push(Workspace {
                    value: path.clone(),
                    label: name.clone(),
                    help: path,
                    project: Some(name.clone()),
                    active_work_items: work_items
                        .iter()
                        .filter(|item| item.projects.contains(&name))
                        .cloned()
                        .collect(),
                });
            }
        }
    }
    Json(out).into_response()
}

/// Whether this Machine's Code reads may execute on the Controller's own
/// filesystem. Both halves are required and neither is sufficient: a Machine
/// must ask by declaring local mode, and an operator must have permitted it.
/// Naming a Machine does not make a remote one local, and declaring local mode
/// does not make an unpermitted Machine readable here.
pub(super) fn colocated_execution_permitted(
    declared_local: bool,
    machine_id: &str,
    permitted: &std::collections::BTreeSet<String>,
) -> bool {
    declared_local && permitted.contains(machine_id)
}

/// Machine-owned root identities travel with the workspace list they describe.
/// A component-only refresh preserves both; a workspace observation replaces
/// both, so an old identity can never outlive the roots it was minted for.
pub(super) fn apply_workspace_inventory(
    current: &mut Vec<crate::machine_protocol::MachineWorkspace>,
    current_identities: &mut Vec<crate::machine_protocol::WorkspaceRootIdentity>,
    current_revision: &mut Option<String>,
    workspaces: Option<Vec<crate::machine_protocol::MachineWorkspace>>,
    identities: Option<Vec<crate::machine_protocol::WorkspaceRootIdentity>>,
    revision: Option<String>,
) {
    if let Some(workspaces) = workspaces {
        *current = workspaces;
        *current_identities = identities.unwrap_or_default();
        *current_revision = revision;
    }
}

pub(super) fn default_machine_id() -> String {
    "local".to_owned()
}

pub(super) fn web_session_is_missing_machine(machine_id: &str, origin: &SessionOrigin) -> bool {
    machine_id == "local" && matches!(origin, SessionOrigin::Web)
}

pub(super) async fn api_reconcile_project_sessions(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ReconcileProjectSessionsRequest>,
) -> Response {
    let project = req.project.trim();
    if project.is_empty() {
        return (StatusCode::BAD_REQUEST, "project cannot be empty").into_response();
    }
    match state
        .supervisor
        .reconcile_project_sessions(project, req.dry_run)
    {
        Ok(session_ids) => {
            let native_thread_ids = state
                .hub
                .session_list()
                .into_iter()
                .filter(|meta| session_ids.contains(&meta.id))
                .filter_map(|meta| meta.agent_session_id.map(|thread| (meta.id, thread)))
                .collect();
            Json(ReconcileProjectSessionsResponse {
                session_ids,
                native_thread_ids,
            })
            .into_response()
        }
        Err(message) => (StatusCode::CONFLICT, message).into_response(),
    }
}

pub(crate) fn resolve_machine_workspace<'a>(
    workspaces: &'a [crate::machine_protocol::MachineWorkspace],
    requested_id: Option<&str>,
) -> Result<&'a crate::machine_protocol::MachineWorkspace, String> {
    let requested_id = requested_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "remote session requires a trusted workspace id".to_owned())?;
    workspaces
        .iter()
        .find(|workspace| workspace.id == requested_id)
        .ok_or_else(|| format!("unknown trusted workspace {requested_id:?}"))
}

pub(super) fn default_file_limit() -> usize {
    20
}

pub(super) fn default_file_tree_limit() -> usize {
    200
}

pub(super) fn file_tree_revision(path: &str, entries: &[FileTreeEntry], truncated: bool) -> String {
    let mut digest = Sha256::new();
    digest.update(path.as_bytes());
    digest.update([u8::from(truncated)]);
    for entry in entries {
        digest.update(entry.name.as_bytes());
        digest.update([0]);
        digest.update(entry.path.as_bytes());
        digest.update([0]);
        digest.update(entry.kind.as_bytes());
        digest.update([0]);
        digest.update([u8::from(entry.ignored)]);
    }
    format!("{:x}", digest.finalize())
}

pub(super) fn file_tree_http_response(
    headers: &HeaderMap,
    revision: &str,
    bytes: Vec<u8>,
) -> Response {
    const TREE_CACHE_CONTROL: &str = "private, max-age=15, stale-while-revalidate=120";
    let etag = format!("\"{revision}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains(etag.as_str()))
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag.as_str()),
                (header::CACHE_CONTROL, TREE_CACHE_CONTROL),
            ],
        )
            .into_response();
    }
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(TREE_CACHE_CONTROL),
    );
    if let Ok(value) = etag.parse() {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

/// Which document version a language server's diagnostics were computed
/// against, relative to the text they are reported on. Never project-wide.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AnalysisState {
    DocumentCurrent,
    Stale,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ServerAnalysis {
    pub(super) server_id: u64,
    pub(super) name: String,
    pub(super) state: AnalysisState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(super) enum ZedAdapterResponse {
    AgentBufferBudget {
        api_version: u8,
        state: String,
    },
    Worktree {
        api_version: u8,
        state: String,
    },
    Buffer {
        api_version: u8,
        path: String,
        leases: usize,
    },
    BufferLanguage {
        api_version: u8,
        path: String,
        version: Vec<CodeBufferVersion>,
        diagnostics: Vec<CodeDiagnostic>,
        inlay_hints: Vec<CodeInlayHint>,
        semantic_tokens: Vec<u32>,
        /// Per-server analysis evidence; absent from engines without it.
        #[serde(default)]
        analysis: Option<Vec<ServerAnalysis>>,
    },
    BufferHover {
        api_version: u8,
        path: String,
        contents: Vec<CodeHoverBlock>,
    },
    BufferNavigation {
        api_version: u8,
        path: String,
        locations: Vec<CodeLocation>,
    },
    BufferSymbols {
        api_version: u8,
        path: String,
        symbols: Vec<CodeDocumentSymbol>,
    },
    Error {
        message: String,
    },
    #[serde(other)]
    Other,
}

#[cfg(test)]
pub(super) async fn zed_adapter_request(
    socket: &FsPath,
    request: serde_json::Value,
) -> anyhow::Result<ZedAdapterResponse> {
    zed_session::local_request(socket, request).await
}

pub(super) fn validate_zed_adapter_response(
    response: ZedAdapterResponse,
) -> anyhow::Result<ZedAdapterResponse> {
    match response {
        response @ (ZedAdapterResponse::Worktree { api_version: 1, .. }
        | ZedAdapterResponse::Buffer { api_version: 1, .. }
        | ZedAdapterResponse::BufferLanguage { api_version: 1, .. }
        | ZedAdapterResponse::BufferHover { api_version: 1, .. }
        | ZedAdapterResponse::BufferNavigation { api_version: 1, .. }
        | ZedAdapterResponse::BufferSymbols { api_version: 1, .. }) => Ok(response),
        ZedAdapterResponse::AgentBufferBudget {
            api_version: 1,
            state,
        } if matches!(state.as_str(), "saturated" | "unavailable") => {
            Ok(ZedAdapterResponse::AgentBufferBudget {
                api_version: 1,
                state,
            })
        }
        ZedAdapterResponse::AgentBufferBudget { .. } => {
            anyhow::bail!("invalid Code resource admission receipt")
        }
        ZedAdapterResponse::Worktree { api_version, .. }
        | ZedAdapterResponse::Buffer { api_version, .. }
        | ZedAdapterResponse::BufferLanguage { api_version, .. }
        | ZedAdapterResponse::BufferHover { api_version, .. }
        | ZedAdapterResponse::BufferNavigation { api_version, .. }
        | ZedAdapterResponse::BufferSymbols { api_version, .. } => {
            anyhow::bail!("unsupported Zed adapter API version {api_version}")
        }
        ZedAdapterResponse::Error { message } => anyhow::bail!("{message}"),
        ZedAdapterResponse::Other => anyhow::bail!("unexpected Zed adapter response"),
    }
}

pub(super) async fn zed_request_in_scope(
    hub: &Hub,
    control: &MachineControl,
    local_socket: Option<&FsPath>,
    scope: &crate::core::SessionCodeScope,
    request: serde_json::Value,
) -> anyhow::Result<ZedAdapterResponse> {
    zed_session::Operation::connect(hub, control, local_socket, scope)
        .await?
        .request(request)
        .await
}

pub(super) async fn remote_code_request(
    state: &AppState,
    scope: &CodeReadScope,
    operation: CodeOperation,
) -> anyhow::Result<Option<crate::code_adapter::CodeAdapterResponse>> {
    let value = match scope {
        CodeReadScope::Workspace(workspace) => {
            if state
                .machine_control
                .workspace_scope_is_colocated(workspace)
                .map_err(anyhow::Error::msg)?
            {
                return Ok(None);
            }
            Some(
                state
                    .machine_control
                    .code_request_in_workspace(workspace, operation)
                    .await
                    .map_err(anyhow::Error::msg)?,
            )
        }
        CodeReadScope::Session(session) => {
            anyhow::ensure!(
                code_reads::session::current(&state.hub, &state.machine_control, session),
                "code context changed"
            );
            state
                .machine_control
                .code_request_in_session(session, operation)
                .await
                .map_err(anyhow::Error::msg)?
        }
    };
    value
        .map(serde_json::from_value)
        .transpose()
        .map_err(Into::into)
}

pub(super) async fn ensure_zed_worktree_for_session(
    state: &AppState,
    context: &ResolvedCodeContext,
) -> anyhow::Result<bool> {
    let CodeReadScope::Session(scope) = &context.scope else {
        anyhow::bail!("language request requires a Session scope");
    };
    code_reads::session::worktree_ready(
        &state.hub,
        &state.machine_control,
        state.zed_adapter_socket.as_deref(),
        scope,
    )
    .await
}

#[cfg(test)]
pub(super) async fn ensure_zed_worktree(socket: &FsPath, cwd: &str) -> anyhow::Result<bool> {
    match zed_adapter_request(
        socket,
        serde_json::json!({
            "type": "ensureWorktree",
            "path": cwd,
            "trusted": true,
        }),
    )
    .await?
    {
        ZedAdapterResponse::Worktree { state, .. } => Ok(state == "ready"),
        _ => anyhow::bail!("unexpected Zed adapter response"),
    }
}

pub(super) fn default_code_context() -> usize {
    6
}

/// Rank files under a session's working directory for the `@` reference picker.
///
/// The cwd comes from the session itself (never from the client) so a browser
/// can't walk arbitrary paths. The walk + fuzzy match is blocking, so it runs
/// on a blocking thread; a missing session is `404`, an empty tree is `200`
/// with `[]`.
pub(super) async fn api_search_files(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
    Query(query): Query<FileSearchQuery>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let cwd = context.cwd;
        let limit = query.limit.clamp(1, 100);
        let files = match remote_code_request(
            &state,
            &context.scope,
            CodeOperation::Search {
                query: query.q.clone(),
                limit,
            },
        )
        .await
        {
            Ok(Some(crate::code_adapter::CodeAdapterResponse::Search(files))) => files,
            Ok(Some(_)) | Err(_) => {
                return (StatusCode::BAD_GATEWAY, "remote code search unavailable").into_response();
            }
            Ok(None) => tokio::task::spawn_blocking(move || {
                crate::files::search(std::path::Path::new(&cwd), &query.q, limit)
            })
            .await
            .unwrap_or_default(),
        };
        Json(FileSearchResponse { files }).into_response()
    })
    .await
}

pub(super) fn parse_workspace_code_context(value: &str) -> Option<(&str, &str)> {
    let value = value.strip_prefix(WORKSPACE_CODE_CONTEXT_PREFIX)?;
    let (machine_id, workspace_id) = value.split_once("::")?;
    (!machine_id.is_empty() && !workspace_id.is_empty() && !workspace_id.contains("::"))
        .then_some((machine_id, workspace_id))
}

/// `worktree::<machine>::<workspace>::<hex path>`, so a path is one URL segment.
pub(super) fn parse_worktree_code_context(value: &str) -> Option<(&str, &str, String)> {
    let mut parts = value.strip_prefix("worktree::")?.split("::");
    let (machine_id, workspace_id, encoded) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || machine_id.is_empty()
        || workspace_id.is_empty()
        || encoded.len() > 8192
    {
        return None;
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(encoded.get(index..index + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let path = String::from_utf8(bytes).ok()?;
    (path.starts_with('/') && !path.contains('\0')).then_some((machine_id, workspace_id, path))
}

/// A session's worktree is read as that session, under its owner's visibility;
/// the workspace policy must not become a second way in.
pub(super) fn worktree_belongs_to_a_session(state: &AppState, path: &str) -> bool {
    state.hub.session_list().iter().any(|session| {
        session.cwd == path
            || state
                .hub
                .session_code_scope(&session.id)
                .is_some_and(|scope| scope.cwd() == path)
    })
}

pub(super) async fn resolve_code_context(
    state: &AppState,
    id: &str,
) -> Option<ResolvedCodeContext> {
    if let Some((machine_id, workspace_id, path)) = parse_worktree_code_context(id) {
        if worktree_belongs_to_a_session(state, &path) {
            return None;
        }
        let store = state.store.as_ref()?;
        let scope = code_reads::workspace::resolve_worktree(
            store,
            &state.machine_control,
            &state.service_id,
            machine_id,
            workspace_id,
            &path,
        )
        .await?;
        return Some(ResolvedCodeContext {
            cwd: scope.cwd().into(),
            scope,
        });
    }
    if let Some((machine_id, workspace_id)) = parse_workspace_code_context(id) {
        let store = state.store.as_ref()?;
        let scope = code_reads::workspace::resolve(
            store,
            &state.machine_control,
            &state.service_id,
            machine_id,
            workspace_id,
        )
        .await?;
        return Some(ResolvedCodeContext {
            cwd: scope.cwd().into(),
            scope,
        });
    }
    let scope =
        code_reads::session::resolve(&state.hub, &state.machine_control, &state.service_id, id)?;
    Some(ResolvedCodeContext {
        cwd: scope.cwd().to_owned(),
        scope,
    })
}

/// Fetch the compared branch into Review's own refs. This is a mutation, so it
/// sits under the session's write authorization, not the read-only Code guard.
pub(super) async fn api_review_fetch(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    Json(request): Json<CodeFetchRequest>,
) -> Response {
    let Some(context) = resolve_code_context(&state, &session_id).await else {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    };
    if !matches!(context.scope, CodeReadScope::Session(_)) {
        return (
            StatusCode::CONFLICT,
            "only a session worktree can be fetched",
        )
            .into_response();
    }
    match remote_code_request(&state, &context.scope, CodeOperation::Manifest).await {
        Ok(None) => {}
        Ok(Some(_)) => {
            return (
                StatusCode::CONFLICT,
                "this Machine cannot fetch from Review yet",
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                "the session's Machine is unavailable",
            )
                .into_response();
        }
    }
    let cwd = context.cwd.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::code_review::LocalCodeProvider::new(FsPath::new(&cwd))
            .fetch_comparison(&request.comparison)
    })
    .await;
    match result {
        Ok(Ok(fetched)) => {
            state.diff_snapshots.forget(&context.scope).await;
            Json(fetched).into_response()
        }
        Ok(Err(error)) => (StatusCode::CONFLICT, error).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "fetch failed").into_response(),
    }
}

/// Resolve a browser buffer path on the host that owns the session.
///
/// Local sessions can be canonicalized here, including Columbus aggregate
/// projections. A remote Machine path does not exist on the Controller, so the
/// Controller only applies the portable relative-path boundary and leaves the
/// authoritative canonical/trusted-root check to that Machine's Zed adapter.
pub(super) fn zed_language_target(
    machine_id: &str,
    cwd: &str,
    relative: &str,
) -> Option<(String, String)> {
    if machine_id != "local" {
        if relative.is_empty()
            || relative.starts_with('/')
            || relative.contains(['\\', '\0'])
            || relative
                .split('/')
                .any(|component| matches!(component, "" | "." | ".."))
        {
            return None;
        }
        return Some((cwd.to_owned(), relative.to_owned()));
    }
    crate::code_review::LocalCodeProvider::new(std::path::Path::new(cwd))
        .language_buffer_key(relative)
        .map(|(worktree, path)| {
            (
                worktree.to_string_lossy().into_owned(),
                path.to_string_lossy().replace('\\', "/"),
            )
        })
}

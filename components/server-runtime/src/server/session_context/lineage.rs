//! Durable lineage recording, child-scoped ancestor chain inspection, and
//! bounded immutable ancestor history paging.

use std::collections::HashSet;

use super::*;
use crate::store::LineageEdge;

/// Maximum ancestor hops returned for inline transcript display.
const MAX_DISPLAY_HOPS: usize = 16;
/// Maximum sibling/child branches listed for the branch switcher.
const MAX_BRANCHES: usize = 50;

#[derive(Debug, Serialize)]
pub struct LineageHopResponse {
    pub depth: usize,
    pub session_id: String,
    pub child_session_id: String,
    pub kind: String,
    pub before_seq: Option<u64>,
    pub title: String,
    pub provider: String,
    /// `"live" | "trashed" | "purged"`
    pub parent_state: &'static str,
    /// `"live" | "archived" | "legacy_unknown" | "unavailable"`
    pub history_state: &'static str,
    pub edge_version: u64,
    pub segment_id: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct LineageBranchResponse {
    pub session_id: String,
    pub parent_session_id: String,
    pub relation: &'static str,
    pub kind: String,
    pub title: String,
    pub provider: String,
    pub status: crate::agent_model::Status,
    pub trashed: bool,
    pub created_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct LineageResponse {
    session_id: String,
    hops: Vec<LineageHopResponse>,
    branches: Vec<LineageBranchResponse>,
}

#[derive(Debug, Deserialize)]
pub struct LineageHistoryQuery {
    #[serde(default = "default_before_seq")]
    pub before_seq: u64,
    #[serde(default)]
    pub edge_version: Option<u64>,
}

fn default_before_seq() -> u64 {
    u64::MAX
}

#[derive(Debug, Serialize)]
struct LineageHistoryResponse {
    session_id: String,
    events: Vec<crate::core::Envelope>,
    next_before_seq: Option<u64>,
    reached_start: bool,
    edge_version: u64,
    is_sealed: bool,
}

/// Record a durable lineage edge, update Hub's auth-parent index, and freeze
/// the parent's `< before_seq` prefix when displayed.
pub(super) async fn record_edge(
    state: &AppState,
    child_session_id: &str,
    source: &crate::core::SessionMeta,
    kind: &str,
    before_seq: u64,
    display_detached: bool,
) -> Result<(), String> {
    let effective_before = if display_detached { 0 } else { before_seq };
    let edge = LineageEdge {
        child_session_id: child_session_id.to_owned(),
        parent_session_id: source.id.clone(),
        owner_user_id: source.owner_user_id.clone().unwrap_or_default(),
        kind: kind.to_owned(),
        before_seq: effective_before,
        source_title: source.title.clone(),
        source_provider: source.provider.clone(),
        segment_id: None,
        display_detached,
        edge_version: 1,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    if let Some(store) = state.store.as_ref() {
        store
            .upsert_lineage_edge(&edge)
            .await
            .map_err(|error| format!("Lineage storage unavailable ({error})"))?;
    }
    state.hub.record_auth_parent(child_session_id, &source.id);
    if !display_detached && effective_before > 0 {
        state.hub.freeze_lineage_prefix(&source.id, effective_before);
    }
    Ok(())
}


/// Record the durable lineage edge for a Replace handoff (`target_id != id`).
pub(super) async fn record_replace_edge(
    state: &AppState,
    target_id: &str,
    source: &crate::core::SessionMeta,
    target_context: &crate::session_context::Context,
) {
    let self_source = target_context
        .injection
        .as_ref()
        .and_then(|inj| inj.sources.iter().find(|src| src.session_id == source.id));
    let before_seq = self_source.map_or(0, |src| src.before);
    let display_detached = self_source.is_none();
    if let Err(error) = record_edge(
        state,
        target_id,
        source,
        "replace",
        before_seq,
        display_detached,
    )
    .await
    {
        tracing::warn!(session = %target_id, source = %source.id, %error, "replace lineage edge not recorded");
    }
}

/// Remove a rolled-back lineage edge (Cancel Switch).
pub(super) async fn delete_edge(state: &AppState, child_session_id: &str) {
    state.hub.remove_auth_parent(child_session_id);
    if let Some(store) = state.store.as_ref()
        && let Err(error) = store.delete_lineage_edge(child_session_id).await
    {
        tracing::warn!(session = %child_session_id, %error, "failed to delete cancelled lineage edge");
    }
}

fn edge_owner_matches(state: &AppState, principal: &ProductPrincipal, edge: &LineageEdge) -> bool {
    !state.product_auth_enabled || edge.owner_user_id == principal.user_id
}

fn owner_can_see_session(
    state: &AppState,
    principal: &ProductPrincipal,
    id: &str,
) -> Result<SessionInfo, StatusCode> {
    let info = state.hub.session_info(id).ok_or(StatusCode::NOT_FOUND)?;
    if info.meta.system || info.meta.closing || state.hub.is_managed_child(id) {
        return Err(StatusCode::NOT_FOUND);
    }
    if state.product_auth_enabled
        && info.meta.owner_user_id.as_deref() != Some(principal.user_id.as_str())
    {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(info)
}

async fn walk_display_edges(
    state: &AppState,
    principal: &ProductPrincipal,
    root_id: &str,
) -> Result<Vec<LineageEdge>, StatusCode> {
    let Some(store) = state.store.as_ref() else {
        return Ok(Vec::new());
    };
    let mut edges = Vec::new();
    let mut visited = HashSet::with_capacity(8);
    visited.insert(root_id.to_owned());
    let mut cursor = root_id.to_owned();
    for _ in 0..MAX_DISPLAY_HOPS {
        let edge = store
            .lineage_edge(&cursor)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let Some(edge) = edge else {
            break;
        };
        if edge.display_detached {
            break;
        }
        if !edge_owner_matches(state, principal, &edge) {
            return Err(StatusCode::FORBIDDEN);
        }
        if !visited.insert(edge.parent_session_id.clone()) {
            break;
        }
        cursor = edge.parent_session_id.clone();
        edges.push(edge);
    }
    Ok(edges)
}

fn legacy_hop(
    state: &AppState,
    principal: &ProductPrincipal,
    info: &SessionInfo,
) -> Option<LineageHopResponse> {
    let (kind, parent_id) = if let Some(src) = info.meta.forked_from.as_deref() {
        ("native_fork", src)
    } else {
        let src = info.meta.continued_from.as_deref()?;
        ("replace", src)
    };
    let parent_info = state.hub.session_info(parent_id).filter(|parent| {
        !state.product_auth_enabled
            || parent.meta.owner_user_id.as_deref() == Some(principal.user_id.as_str())
    });
    let (title, provider, parent_state) = match parent_info {
        Some(parent) => (
            parent.meta.title,
            parent.meta.provider,
            if parent.meta.trashed_at_ms.is_some() {
                "trashed"
            } else {
                "live"
            },
        ),
        None => (String::new(), String::new(), "purged"),
    };
    Some(LineageHopResponse {
        depth: 1,
        session_id: parent_id.to_owned(),
        child_session_id: info.meta.id.clone(),
        kind: kind.to_owned(),
        before_seq: None,
        title,
        provider,
        parent_state,
        history_state: "legacy_unknown",
        edge_version: 0,
        segment_id: None,
        created_at_ms: 0,
    })
}

async fn collect_branches(
    state: &AppState,
    principal: &ProductPrincipal,
    session_id: &str,
    immediate_parent: Option<&str>,
) -> Vec<LineageBranchResponse> {
    let Some(store) = state.store.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut push_edges = |edges: Vec<LineageEdge>, relation: &'static str| {
        for edge in edges {
            if out.len() >= MAX_BRANCHES
                || edge.display_detached
                || edge.child_session_id == session_id
                || !edge_owner_matches(state, principal, &edge)
            {
                continue;
            }
            let Some(info) = state.hub.session_info(&edge.child_session_id) else {
                continue;
            };
            if info.meta.closing || info.meta.system {
                continue;
            }
            out.push(LineageBranchResponse {
                session_id: info.meta.id,
                parent_session_id: edge.parent_session_id,
                relation,
                kind: edge.kind,
                title: info.meta.title,
                provider: info.meta.provider,
                status: info.meta.status,
                trashed: info.meta.trashed_at_ms.is_some(),
                created_at_ms: edge.created_at_ms,
            });
        }
    };
    if let Ok(children) = store.lineage_children(session_id).await {
        push_edges(children, "child");
    }
    if let Some(parent_id) = immediate_parent
        && let Ok(siblings) = store.lineage_children(parent_id).await
    {
        push_edges(siblings, "sibling");
    }
    out
}

/// `GET /api/sessions/{id}/lineage` — ordered ancestor hops and sibling/child
/// branches authorized through child `{id}`.
pub async fn get(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
) -> Response {
    let info = match owner_can_see_session(&state, &auth.principal, &id) {
        Ok(info) => info,
        Err(status) => return status.into_response(),
    };
    let edges = match walk_display_edges(&state, &auth.principal, &id).await {
        Ok(edges) => edges,
        Err(status) => return status.into_response(),
    };
    let immediate_parent = edges
        .first()
        .map(|edge| edge.parent_session_id.clone())
        .or_else(|| info.meta.forked_from.clone())
        .or_else(|| info.meta.continued_from.clone());

    let mut hops = Vec::with_capacity(edges.len());
    for (idx, edge) in edges.into_iter().enumerate() {
        let live_parent = state.hub.session_info(&edge.parent_session_id);
        let (title, provider, parent_state) = match live_parent {
            Some(parent) => (
                parent.meta.title,
                parent.meta.provider,
                if parent.meta.trashed_at_ms.is_some() {
                    "trashed"
                } else {
                    "live"
                },
            ),
            None => (
                edge.source_title.clone(),
                edge.source_provider.clone(),
                "purged",
            ),
        };
        let history_state = if edge.segment_id.is_some() {
            "archived"
        } else if parent_state != "purged" {
            "live"
        } else {
            "unavailable"
        };
        hops.push(LineageHopResponse {
            depth: idx + 1,
            session_id: edge.parent_session_id,
            child_session_id: edge.child_session_id,
            kind: edge.kind,
            before_seq: Some(edge.before_seq),
            title,
            provider,
            parent_state,
            history_state,
            edge_version: edge.edge_version,
            segment_id: edge.segment_id,
            created_at_ms: edge.created_at_ms,
        });
    }

    if hops.is_empty()
        && let Some(legacy) = legacy_hop(&state, &auth.principal, &info)
    {
        hops.push(legacy);
    }

    let branches = collect_branches(
        &state,
        &auth.principal,
        &id,
        immediate_parent.as_deref(),
    )
    .await;

    (
        [(header::CACHE_CONTROL.as_str(), "private, no-store")],
        Json(LineageResponse {
            session_id: id,
            hops,
            branches,
        }),
    )
        .into_response()
}

/// `GET /api/sessions/{id}/lineage/{hop_id}/history` — child-scoped immutable
/// ancestor history page, server-clamped to `edge.before_seq`.
pub async fn history(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((id, hop_id)): Path<(String, String)>,
    Query(query): Query<LineageHistoryQuery>,
) -> Response {
    if let Err(status) = owner_can_see_session(&state, &auth.principal, &id) {
        return status.into_response();
    }
    let edges = match walk_display_edges(&state, &auth.principal, &id).await {
        Ok(edges) => edges,
        Err(status) => return status.into_response(),
    };
    let Some(edge) = edges.into_iter().find(|edge| edge.parent_session_id == hop_id) else {
        return (StatusCode::NOT_FOUND, "ancestor not in session lineage").into_response();
    };
    if let Some(expected) = query.edge_version
        && expected != edge.edge_version
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "lineage_edge_changed",
                "edge_version": edge.edge_version,
            })),
        )
            .into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "lineage store unavailable").into_response();
    };
    let page = match store
        .lineage_history_page(&edge, query.before_seq, crate::core::HISTORY_PAGE)
        .await
    {
        Ok(page) => page,
        Err(error) => {
            tracing::warn!(session = %id, hop = %hop_id, %error, "lineage history query failed");
            return (StatusCode::SERVICE_UNAVAILABLE, "lineage history unavailable").into_response();
        }
    };
    let body = Json(LineageHistoryResponse {
        session_id: hop_id,
        events: page.events,
        next_before_seq: page.next_before_seq,
        reached_start: page.reached_start,
        edge_version: edge.edge_version,
        is_sealed: page.is_sealed,
    });
    if page.is_sealed {
        (
            [
                (header::CACHE_CONTROL.as_str(), "private, no-store"),
                ("x-cowboy-history", "complete"),
            ],
            body,
        )
            .into_response()
    } else {
        ([(header::CACHE_CONTROL.as_str(), "private, no-store")], body).into_response()
    }
}

/// `GET /api/sessions/{id}/lineage/{hop_id}/question-pages` — child-scoped
/// Explore question index strictly clamped to `edge.before_seq`.
pub async fn question_pages(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((id, hop_id)): Path<(String, String)>,
    Query(query): Query<QuestionPagesQuery>,
) -> Response {
    if let Err(status) = owner_can_see_session(&state, &auth.principal, &id) {
        return status.into_response();
    }
    let edges = match walk_display_edges(&state, &auth.principal, &id).await {
        Ok(edges) => edges,
        Err(status) => return status.into_response(),
    };
    let Some(edge) = edges.into_iter().find(|edge| edge.parent_session_id == hop_id) else {
        return (StatusCode::NOT_FOUND, "ancestor not in session lineage").into_response();
    };
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "lineage store unavailable").into_response();
    };
    let limit = query.limit.unwrap_or(64).clamp(1, 100);
    let clamped_before = query.before.map(|before| before.min(edge.before_seq));
    match store.lineage_question_pages(&edge, clamped_before, limit).await {
        Ok((pages, next_before_seq, total)) => (
            [(header::CACHE_CONTROL.as_str(), "private, no-store")],
            Json(QuestionPagesResponse {
                total,
                exact: true,
                pages,
                next_before_seq,
            }),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!(session = %id, hop = %hop_id, %error, "lineage question pages query failed");
            (StatusCode::SERVICE_UNAVAILABLE, "lineage question pages unavailable").into_response()
        }
    }
}

/// `GET /api/sessions/{id}/lineage/{hop_id}/question-pages/{page_id}` —
/// child-scoped Explore question page detail strictly `< edge.before_seq`.
pub async fn question_page(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((id, hop_id, page_id)): Path<(String, String, u64)>,
) -> Response {
    if let Err(status) = owner_can_see_session(&state, &auth.principal, &id) {
        return status.into_response();
    }
    let edges = match walk_display_edges(&state, &auth.principal, &id).await {
        Ok(edges) => edges,
        Err(status) => return status.into_response(),
    };
    let Some(edge) = edges.into_iter().find(|edge| edge.parent_session_id == hop_id) else {
        return (StatusCode::NOT_FOUND, "ancestor not in session lineage").into_response();
    };
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "lineage store unavailable").into_response();
    };
    match store.lineage_question_page_at(&edge, page_id).await {
        Ok(Some(events)) => (
            [(header::CACHE_CONTROL.as_str(), "private, no-store")],
            Json(HistoryResponse {
                events,
                next_before_seq: None,
                reached_start: false,
            }),
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(session = %id, hop = %hop_id, page_id, %error, "lineage question page query failed");
            (StatusCode::SERVICE_UNAVAILABLE, "lineage question page unavailable").into_response()
        }
    }
}

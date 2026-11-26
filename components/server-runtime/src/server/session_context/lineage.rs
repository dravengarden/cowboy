//! Durable lineage recording, child-scoped ancestor chain inspection, and
//! bounded immutable ancestor history paging.

use super::*;
use crate::core::{AuthParentStatus, LineageCaptureGuard};
use crate::store::{LineageEdge, LineageHistoryOutcome};

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
    /// `true` when the session has a durable edge that was explicitly detached
    /// (Clear or excluded Replace), suppressing legacy provenance fallback.
    display_detached: bool,
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

/// Wait for all previously queued FIFO store writes (such as `InsertSession`
/// and `PutSetting`) to become durable before marking a receipt ready.
pub(super) async fn flush_store_barrier(state: &AppState) -> Result<(), String> {
    let Some(rx) = state.hub.store_barrier() else {
        return Ok(());
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
        Ok(Ok(Ok(true))) => Ok(()),
        Ok(Ok(Err(reason))) => Err(reason),
        Ok(Ok(Ok(false))) => Err("Durable store barrier was not acknowledged".into()),
        Ok(Err(_)) => Err("Durable store writer stopped before barrier completed".into()),
        Err(_) => Err("Timed out waiting for durable session persistence".into()),
    }
}

/// Pin `source_id` against concurrent Clear, freeze `< before_seq` on both
/// reducers in FIFO event order, and wait for the store writer to flush any
/// buffered pre-cutoff events before returning the RAII pin guard.
pub(super) async fn pin_and_freeze_source(
    state: &AppState,
    source_id: &str,
    requested_before: Option<u64>,
) -> Result<(u64, LineageCaptureGuard), String> {
    let (before_seq, ack_rx, guard) = state
        .hub
        .pin_and_freeze_source(source_id, requested_before)?;
    if let Some(rx) = ack_rx {
        match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
            Ok(Ok(Ok(true))) => {}
            Ok(Ok(Err(reason))) => return Err(reason),
            Ok(Ok(Ok(false))) => {
                return Err("Lineage freeze was not acknowledged by durable storage".into());
            }
            Ok(Err(_)) => {
                return Err("Durable store writer stopped before freezing lineage prefix".into());
            }
            Err(_) => {
                return Err("Timed out waiting for durable lineage prefix freeze".into());
            }
        }
    }
    Ok((before_seq, guard))
}

/// Record a durable lineage edge (`hop_order = 1`, materializing ancestor
/// display hops in storage) and update Hub's auth-parent index. Returns `Err`
/// on durable failure so callers never admit an edge-less destination.
pub(super) async fn record_edge(
    state: &AppState,
    child_session_id: &str,
    source: &crate::core::SessionMeta,
    kind: &str,
    before_seq: u64,
    display_detached: bool,
) -> Result<(), String> {
    let effective_before = if display_detached { 0 } else { before_seq };
    let parent_is_root = matches!(
        state.hub.auth_parent_status(&source.id),
        AuthParentStatus::KnownRoot
    );
    let edge = LineageEdge {
        child_session_id: child_session_id.to_owned(),
        hop_order: 1,
        parent_session_id: source.id.clone(),
        parent_is_root,
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
    state
        .hub
        .record_auth_parent(child_session_id, &source.id, parent_is_root);
    Ok(())
}

/// Record the mandatory durable lineage edge for a Replace handoff.
pub(super) async fn record_replace_edge(
    state: &AppState,
    target_id: &str,
    source: &crate::core::SessionMeta,
    target_context: &crate::session_context::Context,
) -> Result<(), String> {
    let self_source = target_context
        .injection
        .as_ref()
        .and_then(|inj| inj.sources.iter().find(|src| src.session_id == source.id));
    let before_seq = self_source.map_or(0, |src| src.before);
    let display_detached = self_source.is_none();
    record_edge(
        state,
        target_id,
        source,
        "replace",
        before_seq,
        display_detached,
    )
    .await
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

async fn load_child_display_edges(
    state: &AppState,
    principal: &ProductPrincipal,
    root_id: &str,
) -> Result<(bool, Vec<LineageEdge>), StatusCode> {
    let Some(store) = state.store.as_ref() else {
        return Ok((false, Vec::new()));
    };
    let lineage = store
        .child_display_lineage(root_id)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    for edge in &lineage.hops {
        if !edge_owner_matches(state, principal, edge) {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok((lineage.has_durable_edge, lineage.hops))
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
    let (has_durable_edge, edges) =
        match load_child_display_edges(&state, &auth.principal, &id).await {
            Ok(result) => result,
            Err(status) => return status.into_response(),
        };
    let display_detached = has_durable_edge && edges.is_empty();
    let immediate_parent = edges
        .first()
        .map(|edge| edge.parent_session_id.clone())
        .or_else(|| {
            (!display_detached)
                .then(|| {
                    info.meta
                        .forked_from
                        .clone()
                        .or(info.meta.continued_from.clone())
                })
                .flatten()
        });

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
        } else if let Some(store) = state.store.as_ref()
            && store
                .session_has_live_events_before(&edge.parent_session_id, edge.before_seq)
                .await
                .unwrap_or(false)
        {
            // Logically deleted in Trash during the 3-day purge grace window:
            // events are still durably readable through the child-scoped route.
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

    // Only emit legacy fallback when NO durable edge exists at all; an
    // explicitly detached session (`display_detached == true`) must never fall
    // back to an unbounded parent link.
    if !has_durable_edge
        && hops.is_empty()
        && let Some(legacy) = legacy_hop(&state, &auth.principal, &info)
    {
        hops.push(legacy);
    }

    let branches =
        collect_branches(&state, &auth.principal, &id, immediate_parent.as_deref()).await;

    (
        [(header::CACHE_CONTROL.as_str(), "private, no-store")],
        Json(LineageResponse {
            session_id: id,
            display_detached,
            hops,
            branches,
        }),
    )
        .into_response()
}

/// `GET /api/sessions/{id}/lineage/{hop_id}/history` — child-scoped immutable
/// ancestor history page, read with its edge version and seal watermark in one
/// consistent database snapshot.
pub async fn history(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((id, hop_id)): Path<(String, String)>,
    Query(query): Query<LineageHistoryQuery>,
) -> Response {
    if let Err(status) = owner_can_see_session(&state, &auth.principal, &id) {
        return status.into_response();
    }
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "lineage store unavailable").into_response();
    };
    let owner_filter = state
        .product_auth_enabled
        .then_some(auth.principal.user_id.as_str());
    let outcome = match store
        .lineage_history_page_checked(
            &id,
            &hop_id,
            owner_filter,
            query.edge_version,
            query.before_seq,
            crate::core::HISTORY_PAGE,
        )
        .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(session = %id, hop = %hop_id, %error, "lineage history query failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "lineage history unavailable",
            )
                .into_response();
        }
    };
    let page = match outcome {
        LineageHistoryOutcome::NotFound => {
            return (StatusCode::NOT_FOUND, "ancestor not in session lineage").into_response();
        }
        LineageHistoryOutcome::VersionConflict { current_version } => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "lineage_edge_changed",
                    "edge_version": current_version,
                })),
            )
                .into_response();
        }
        LineageHistoryOutcome::Page(page) => page,
    };
    let body = Json(LineageHistoryResponse {
        session_id: hop_id,
        events: page.events,
        next_before_seq: page.next_before_seq,
        reached_start: page.reached_start,
        edge_version: page.edge_version,
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
        (
            [(header::CACHE_CONTROL.as_str(), "private, no-store")],
            body,
        )
            .into_response()
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
    let (_, edges) = match load_child_display_edges(&state, &auth.principal, &id).await {
        Ok(result) => result,
        Err(status) => return status.into_response(),
    };
    let Some(edge) = edges
        .into_iter()
        .find(|edge| edge.parent_session_id == hop_id)
    else {
        return (StatusCode::NOT_FOUND, "ancestor not in session lineage").into_response();
    };
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "lineage store unavailable").into_response();
    };
    let limit = query.limit.unwrap_or(64).clamp(1, 100);
    let clamped_before = query.before.map(|before| before.min(edge.before_seq));
    match store
        .lineage_question_pages(&edge, clamped_before, limit)
        .await
    {
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
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "lineage question pages unavailable",
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct LineageQuestionPageQuery {
    #[serde(default)]
    pub after_seq: Option<u64>,
}

#[derive(Debug, Serialize)]
struct LineageQuestionPageResponse {
    events: Vec<crate::core::Envelope>,
    next_before_seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_after_seq: Option<u64>,
    reached_start: bool,
}

/// `GET /api/sessions/{id}/lineage/{hop_id}/question-pages/{page_id}` —
/// child-scoped Explore question page detail strictly `< edge.before_seq`.
pub async fn question_page(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((id, hop_id, page_id)): Path<(String, String, u64)>,
    Query(query): Query<LineageQuestionPageQuery>,
) -> Response {
    if let Err(status) = owner_can_see_session(&state, &auth.principal, &id) {
        return status.into_response();
    }
    let (_, edges) = match load_child_display_edges(&state, &auth.principal, &id).await {
        Ok(result) => result,
        Err(status) => return status.into_response(),
    };
    let Some(edge) = edges
        .into_iter()
        .find(|edge| edge.parent_session_id == hop_id)
    else {
        return (StatusCode::NOT_FOUND, "ancestor not in session lineage").into_response();
    };
    let Some(store) = state.store.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "lineage store unavailable").into_response();
    };
    match store
        .lineage_question_page_at(&edge, page_id, query.after_seq)
        .await
    {
        Ok(Some(detail)) => (
            [(header::CACHE_CONTROL.as_str(), "private, no-store")],
            Json(LineageQuestionPageResponse {
                events: detail.events,
                next_before_seq: None,
                next_after_seq: detail.next_after_seq,
                reached_start: detail.reached_start,
            }),
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(session = %id, hop = %hop_id, page_id, %error, "lineage question page query failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "lineage question page unavailable",
            )
                .into_response()
        }
    }
}

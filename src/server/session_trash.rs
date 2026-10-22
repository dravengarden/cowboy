//! Sessions Trash commands and retention (`docs/sessions-folders.md#trash`).
//!
//! Trash is a Controller-only state: moving a session there hibernates its
//! idle worker and keeps everything else. Permanent deletion reuses the one
//! session-deletion lifecycle, whether requested or due to retention.
use super::*;
use crate::session_folders::FolderActor;
use std::collections::HashSet;

pub(super) const TRASHED_REFUSAL: &str = crate::core::TRASHED_SESSION_REFUSAL;

/// How often the retention sweep looks for expired Trash and for requested
/// folder deletions still waiting on a remote session stop. One in-memory scan.
const RETENTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Trashed folders whose permanent deletion was requested while a session in
/// them was still stopping. Permission was checked at request time.
static PENDING_FOLDER_PURGES: std::sync::LazyLock<parking_lot::Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);
/// Let Machines reconnect first: a remote session is deleted only after its
/// execution environment confirms the stop.
const RETENTION_DELAY: std::time::Duration = std::time::Duration::from_secs(2 * 60);

fn wall_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn mutable_sessions(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    session_ids: &[String],
) -> Result<(), String> {
    for session_id in session_ids {
        let owner = state
            .hub
            .session_info(session_id)
            .ok_or("Session no longer exists")?
            .meta
            .owner_user_id;
        if !principal.can_mutate(owner.as_deref()) {
            return Err("not allowed to change this session".into());
        }
    }
    Ok(())
}

fn trash_actor(state: &Arc<AppState>, principal: &ProductPrincipal) -> Result<FolderActor, String> {
    if !principal.can_reorder() {
        return Err("viewers cannot delete sessions".into());
    }
    Ok(folder_actor(state, principal))
}

/// Move sessions and folders to Trash, then release the idle workers.
pub(super) fn trash(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    folder_ids: &[String],
    session_ids: &[String],
) -> Result<(), String> {
    let actor = trash_actor(state, principal)?;
    mutable_sessions(state, principal, session_ids)?;
    let trashed = state
        .hub
        .trash_items(&actor, folder_ids, session_ids, wall_ms())?;
    for session_id in &trashed.sessions {
        // Best effort: a refusal only means the worker stays idle until the
        // permanent deletion stops it; nothing can wake it meanwhile.
        if state.supervisor.has_live_worker(session_id)
            && let Err(reason) = state.supervisor.hibernate_session(session_id)
        {
            tracing::info!(session = %session_id, %reason, "trashed session kept its idle worker");
        }
    }
    tracing::info!(
        sessions = trashed.sessions.len(),
        folders = trashed.folders.len(),
        "moved items to Trash"
    );
    Ok(())
}

pub(super) fn restore(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    folder_ids: &[String],
    session_ids: &[String],
) -> Result<(), String> {
    let actor = trash_actor(state, principal)?;
    mutable_sessions(state, principal, session_ids)?;
    let restored = state.hub.restore_items(&actor, folder_ids, session_ids)?;
    tracing::info!(
        sessions = restored.sessions.len(),
        folders = restored.folders.len(),
        notes = ?restored.notes,
        "restored items from Trash"
    );
    Ok(())
}

/// Delete trashed items now. Folders go once their sessions are gone; a
/// remote session whose stop is unconfirmed keeps its folder for a retry.
pub(super) fn purge(
    state: &Arc<AppState>,
    principal: &ProductPrincipal,
    folder_ids: &[String],
    session_ids: &[String],
) -> Result<(), String> {
    let actor = trash_actor(state, principal)?;
    mutable_sessions(state, principal, session_ids)?;
    // Ownership of every folder, and of everything below it, is checked
    // before anything is deleted or queued for a retry.
    state.hub.check_trashed_folders(&actor, folder_ids)?;
    let (trashed_sessions, trashed_folders) = state.hub.trash_contents();
    let mut doomed: Vec<String> = Vec::new();
    for session_id in session_ids {
        if !trashed_sessions.iter().any(|(id, _, _)| id == session_id) {
            return Err("Only items in Trash can be deleted permanently".into());
        }
        doomed.push(session_id.clone());
    }
    let subtree: Vec<String> = folder_ids
        .iter()
        .flat_map(|folder| {
            trashed_folders
                .iter()
                .filter(|candidate| {
                    candidate.id == *folder
                        || folder_is_below(&trashed_folders, &candidate.id, folder)
                })
                .map(|candidate| candidate.id.clone())
                .collect::<Vec<_>>()
        })
        .collect();
    for (session_id, _, _) in &trashed_sessions {
        if !doomed.contains(session_id)
            && state
                .hub
                .session_folder_placement(session_id)
                .is_some_and(|folder| subtree.contains(&folder))
        {
            doomed.push(session_id.clone());
        }
    }
    mutable_sessions(state, principal, &doomed)?;
    let mut first_error = None;
    for session_id in &doomed {
        if let Err(error) = delete_claimed(state, session_id, None) {
            first_error.get_or_insert(error);
        }
    }
    let removed = state.hub.purge_trashed_folders(&actor, folder_ids);
    PENDING_FOLDER_PURGES.lock().extend(
        folder_ids
            .iter()
            .filter(|folder| !removed.contains(folder))
            .cloned(),
    );
    first_error.map_or(Ok(()), Err)
}

fn folder_is_below(
    folders: &[crate::session_folders::SessionFolder],
    candidate: &str,
    ancestor: &str,
) -> bool {
    let mut cursor = folders
        .iter()
        .find(|folder| folder.id == candidate)
        .and_then(|folder| folder.parent.clone());
    for _ in 0..=folders.len() {
        let Some(id) = cursor else {
            return false;
        };
        if id == ancestor {
            return true;
        }
        cursor = folders
            .iter()
            .find(|folder| folder.id == id)
            .and_then(|folder| folder.parent.clone());
    }
    false
}

/// Permanently delete a trashed session once it is claimed: still in Trash
/// (since `expected_trashed_at_ms`, when given) and not already closing. The
/// claim marks it closing atomically, so a concurrent restore is refused. A
/// session that was restored or is already being deleted is skipped.
fn delete_claimed(
    state: &Arc<AppState>,
    session_id: &str,
    expected_trashed_at_ms: Option<i64>,
) -> Result<(), String> {
    if !state
        .hub
        .claim_trash_deletion(session_id, expected_trashed_at_ms)
    {
        return Ok(());
    }
    let result = delete_now(state, session_id);
    if result.is_err() {
        state.hub.set_closing(session_id, false);
    }
    result
}

/// The one permanent session deletion: stop the agent first (so it pushes no
/// more events into a soon-to-be-gone Hub session), then drop Hub state and
/// broadcast. A remote environment confirms its stop before the row goes.
pub(super) fn delete_now(state: &Arc<AppState>, session_id: &str) -> Result<(), String> {
    let binding = state
        .hub
        .session_info(session_id)
        .and_then(|info| info.meta.execution_binding);
    if binding
        .as_ref()
        .is_some_and(|binding| binding.managed().is_some())
    {
        managed_calls::delete_child(state, session_id)
    } else if binding.is_some() {
        let result = execution::sessions::delete(state, session_id);
        managed_calls::parent_deleted(state, session_id);
        result
    } else {
        state.supervisor.delete_session(session_id);
        state.hub.delete_session(session_id);
        managed_calls::parent_deleted(state, session_id);
        Ok(())
    }
}

/// Delete Trash past each owner's retention, then the folders it emptied.
pub(super) async fn run_retention(state: Arc<AppState>) {
    tokio::time::sleep(RETENTION_DELAY).await;
    let mut tick = tokio::time::interval(RETENTION_INTERVAL);
    loop {
        tick.tick().await;
        retention_pass(&state, wall_ms()).await;
    }
}

/// The owner's retention, or `None` when it cannot be read now: a failed
/// read must never shorten retention to the default and delete early.
async fn retention_days(
    state: &Arc<AppState>,
    cache: &mut HashMap<Option<String>, Option<i64>>,
    owner: Option<&String>,
) -> Option<i64> {
    if let Some(days) = cache.get(&owner.cloned()) {
        return *days;
    }
    let days = match (owner, state.store.as_ref()) {
        (Some(owner), Some(store)) => match store.preferences(owner, "trash").await {
            Ok(preferences) => Some(crate::store::preferences::trash_retention_days(
                preferences.as_ref(),
            )),
            Err(error) => {
                tracing::warn!(%error, "Trash retention skipped an owner: preferences unavailable");
                None
            }
        },
        _ => Some(crate::store::preferences::trash_retention_days(None)),
    };
    cache.insert(owner.cloned(), days);
    days
}

/// One retention pass. Every owner's retention is read first; the decisions
/// then use one fresh, synchronous snapshot, and each deletion is claimed
/// against the exact Trash time it was judged by, so an item restored (or
/// trashed again) meanwhile is never deleted.
pub(super) async fn retention_pass(state: &Arc<AppState>, now_ms: i64) {
    const DAY_MS: i64 = 86_400_000;
    let (sessions, folders) = state.hub.trash_contents();
    let mut cache = HashMap::new();
    for owner in sessions
        .iter()
        .map(|(_, owner, _)| owner.clone())
        .chain(folders.iter().map(|folder| folder.owner_user_id.clone()))
    {
        retention_days(state, &mut cache, owner.as_ref()).await;
    }
    let expired = |owner: &Option<String>, at: i64| {
        cache
            .get(owner)
            .copied()
            .flatten()
            .is_some_and(|days| at.saturating_add(days.saturating_mul(DAY_MS)) <= now_ms)
    };
    let (sessions, folders) = state.hub.trash_contents();
    for (session_id, owner, trashed_at_ms) in sessions {
        if !expired(&owner, trashed_at_ms) {
            continue;
        }
        match delete_claimed(state, &session_id, Some(trashed_at_ms)) {
            Ok(()) => {
                tracing::info!(session = %session_id, "deleted session after Trash retention")
            }
            Err(error) => {
                tracing::warn!(session = %session_id, %error, "Trash retention could not delete session");
            }
        }
    }
    let mut purge: Vec<String> = PENDING_FOLDER_PURGES.lock().drain().collect();
    for folder in folders {
        if folder
            .trashed_at_ms
            .is_some_and(|at| expired(&folder.owner_user_id, at))
            && !purge.contains(&folder.id)
        {
            purge.push(folder.id);
        }
    }
    if purge.is_empty() {
        return;
    }
    let removed = state
        .hub
        .purge_trashed_folders(&FolderActor::local(), &purge);
    if !removed.is_empty() {
        tracing::info!(folders = removed.len(), "deleted folders from Trash");
    }
    // Keep requested deletions that are still waiting for a session stop.
    let (_, remaining) = state.hub.trash_contents();
    PENDING_FOLDER_PURGES.lock().extend(
        purge
            .into_iter()
            .filter(|id| !removed.contains(id) && remaining.iter().any(|folder| folder.id == *id)),
    );
}

//! Warm restore of the Hub from durable state at Controller startup.

use super::*;

/// Warm the Hub from durable state before any client connects: settings,
/// Draft directories, workspace orders, session folders, every session with
/// its recent events, transcript anchors and subagent marks, continuations,
/// forks and artifacts. Persisted Busy turns stay guarded until their Machine
/// runtime has had one bounded reconnect window.
pub(super) async fn restore_hub_from_store(store: &Store, hub: &Hub) -> anyhow::Result<()> {
    let settings = store
        .load_settings()
        .await
        .context("loading persisted auth settings")?;
    hub.load_settings(settings);
    store
        .integrate_draft_folders()
        .await
        .context("integrating Draft directories")?;
    hub.restore_workspace_orders(store.load_workspace_orders().await?);
    let folders = store
        .load_session_folders()
        .await
        .context("loading session folders")?;
    hub.restore_session_folders(folders);
    // Warm restore — sessions + events come back exactly as the daemon
    // left them, so on a fresh process every WS client's first snapshot
    // is correct.
    let loaded = store.load_all().await.context("loading persisted state")?;
    let restored: Vec<_> = loaded
        .into_iter()
        .map(|ls| RestoredSession {
            meta: ls.meta,
            log: ls.events,
            event_count: ls.event_count,
            reached_start: ls.reached_start,
            next_seq: ls.next_seq,
            queue: ls.queue,
            drafts: ls.drafts,
            config_options: ls.config_options,
            config_preferences: ls.config_preferences,
            mobile_review_state: ls.mobile_review_state,
            folder_id: ls.folder_id,
            last_event_at_ms: ls.last_event_at_ms,
        })
        .collect();
    let restored_count = restored.len();
    // Adopt durable anchors before any reducer sees a new event, so a
    // restarted Controller never starts a second subagent/Call row.
    match store
        .recent_transcript_anchors(crate::transcript_anchors::HUB_WARM_ANCHORS)
        .await
    {
        Ok(anchors) => hub.warm_transcript_anchors(anchors),
        Err(error) => tracing::warn!(%error, "warming transcript anchors failed"),
    }
    match store
        .recent_subagent_snapshots(crate::transcript_anchors::WARM_SNAPSHOT_MARKS)
        .await
    {
        Ok(snapshots) => hub.warm_subagent_marks(snapshots),
        Err(error) => tracing::warn!(%error, "warming subagent marks failed"),
    }
    // Detached workers outlive this control-plane process. Keep
    // persisted Busy turns guarded until their Machine runtime has had
    // one bounded reconnect window to prove ownership; only then may a
    // missing worker become Interrupted.
    hub.restore_reconciling_runtime(restored);
    crate::session_context::restore_announcements(hub);
    hub.restore_continuations();
    hub.restore_forks();
    hub.set_artifacts(store.artifacts());
    tracing::info!(restored = restored_count, "persistence wired",);
    Ok(())
}

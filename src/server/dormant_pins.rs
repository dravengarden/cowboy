//! Release old Provider generations pinned only by long-dormant sessions
//! (`plugins.repin_dormant_sessions`).
//!
//! Generation retention keeps every generation a recoverable session pins, so
//! one forgotten session can hold an old release forever. A session that has
//! stayed exited without a worker for `plugins.repin_dormant_after` is moved
//! to its Device's installed release, but only through the same compatibility
//! gate as an explicit Reload (unchanged authentication and native session
//! contract, saved native session). Nothing is started: the next open resumes
//! the native session on the new release. The following retention pass can
//! then retire the old generation.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use super::*;

/// One persisted setting: session id -> first epoch ms it was seen dormant.
const DORMANT_SINCE: &str = "session_dormant_since";

/// Keep the first-seen time of sessions still dormant, start the clock for
/// newly dormant ones, and forget every other session.
fn next_dormancy(
    previous: &BTreeMap<String, i64>,
    dormant: &BTreeSet<String>,
    now_ms: i64,
) -> BTreeMap<String, i64> {
    dormant
        .iter()
        .map(|id| (id.clone(), previous.get(id).copied().unwrap_or(now_ms)))
        .collect()
}

fn due(since_ms: i64, now_ms: i64, after: Duration) -> bool {
    u64::try_from(now_ms.saturating_sub(since_ms))
        .is_ok_and(|elapsed| elapsed >= u64::try_from(after.as_millis()).unwrap_or(u64::MAX))
}

fn load(state: &AppState) -> BTreeMap<String, i64> {
    state
        .hub
        .settings_snapshot()
        .get(DORMANT_SINCE)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Run before a retention pass. Records dormancy every time; re-pins only when
/// the setting is enabled.
pub(super) async fn repin(state: &Arc<AppState>) {
    let sessions = state.hub.session_list();
    let dormant: BTreeSet<String> = sessions
        .iter()
        .filter(|session| {
            session.status == Status::Exited
                && !session.system
                && !holds_worker_slot(&state.runtime_router, session)
                && !state.hub.session_has_in_flight_prompt(&session.id)
        })
        .map(|session| session.id.clone())
        .collect();
    let previous = load(state);
    let now = now_ms();
    let next = next_dormancy(&previous, &dormant, now);
    if next != previous {
        state
            .hub
            .set_setting(DORMANT_SINCE.to_owned(), serde_json::json!(next));
    }
    if !state
        .service_config
        .get(&crate::config::schema::PLUGIN_REPIN_DORMANT_SESSIONS)
    {
        return;
    }
    let after = state
        .service_config
        .get(&crate::config::schema::PLUGIN_REPIN_DORMANT_AFTER);
    for meta in sessions {
        let Some(since) = next.get(&meta.id) else {
            continue;
        };
        if !due(*since, now, after) {
            continue;
        }
        let Ok(_fence) = ProviderReloadFence::acquire(
            &state.plugin_lifecycle_fences,
            (meta.machine_id.clone(), meta.provider.clone()),
        ) else {
            continue;
        };
        // Same gate as an explicit Reload: connected Device, saved native
        // session, installed release, unchanged auth and native contract.
        let target = match session_reload_target(state, &meta).await {
            Ok(target) => target,
            Err(error) => {
                tracing::debug!(session = %meta.id, %error, "dormant session keeps its Provider generation");
                continue;
            }
        };
        if target.digest == meta.provider_generation_digest {
            continue;
        }
        match state.hub.repin_dormant_provider(
            &meta,
            &target.version,
            &target.digest,
            &target.behavior,
        ) {
            Ok(()) => tracing::info!(
                session = %meta.id,
                machine = %meta.machine_id,
                from = %meta.provider_version,
                to = %target.version,
                "re-pinned a dormant session to the installed Provider release"
            ),
            Err(error) => {
                tracing::debug!(session = %meta.id, %error, "dormant session re-pin deferred");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn dormancy_keeps_first_seen_time_and_forgets_revived_or_deleted_sessions() {
        let first = next_dormancy(&BTreeMap::new(), &ids(&["a", "b"]), 100);
        assert_eq!(
            first,
            BTreeMap::from([("a".into(), 100), ("b".into(), 100)])
        );
        // "a" stays dormant (keeps 100), "b" was opened, "c" newly dormant.
        let second = next_dormancy(&first, &ids(&["a", "c"]), 500);
        assert_eq!(
            second,
            BTreeMap::from([("a".into(), 100), ("c".into(), 500)])
        );
    }

    #[test]
    fn a_session_is_due_only_after_the_full_dormant_period() {
        let week = Duration::from_hours(7 * 24);
        let week_ms = 7 * 24 * 3_600_000;
        assert!(!due(0, week_ms - 1, week));
        assert!(due(0, week_ms, week));
        // A clock step backwards never makes a session due.
        assert!(!due(1_000, 0, Duration::from_hours(1)));
    }
}

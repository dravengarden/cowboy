//! Provider update offers and idle-only adoption of the Machine's installed
//! Provider release.
//!
//! Every pass tells clients which sessions have a newer compatible release
//! waiting, then applies it where policy allows: a running session reloads
//! through the explicit Reload gate once it is idle, and a dormant (exited,
//! workerless) session is only re-pinned so its next open starts the new
//! release. Nothing is ever interrupted.
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::*;
use crate::core::ProviderUpdateOffer;

/// Full passes resolve every session's installed release.
const PASS_INTERVAL: Duration = Duration::from_secs(30);
/// How quickly a requested "update when idle" starts after the turn ends.
const REQUEST_POLL: Duration = Duration::from_secs(5);

fn key(session: &str) -> String {
    format!(
        "{}{session}",
        crate::core::settings_keys::SESSION_PROVIDER_AUTO_UPDATE_PREFIX
    )
}

pub(super) fn enabled(hub: &Hub, session: &str) -> bool {
    hub.settings_snapshot()
        .get(&key(session))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// `enabled` keeps a session on every future release once idle; `when_idle`
/// asks for the currently offered release once, after the running work.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    when_idle: Option<bool>,
}

pub(super) async fn configure(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    Json(policy): Json<Policy>,
) -> Response {
    if state.hub.session_info(&session).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    if policy.enabled.is_none() && policy.when_idle.is_none() {
        return (StatusCode::BAD_REQUEST, "set enabled or when_idle").into_response();
    }
    if let Some(enabled) = policy.enabled {
        state
            .hub
            .set_setting(key(&session), serde_json::json!(enabled));
    }
    if let Some(when_idle) = policy.when_idle {
        state.hub.set_provider_update_when_idle(&session, when_idle);
    }
    Json(serde_json::json!({
        "enabled": enabled(&state.hub, &session),
        "when_idle": state.hub.provider_update_when_idle(&session),
    }))
    .into_response()
}

fn newer(current: &str, target: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(target),
    ) {
        (Ok(current), Ok(target)) => target.pre.is_empty() && target > current,
        _ => false,
    }
}

/// A session adopts the installed release when it (or a one-shot request)
/// opted in, or when `plugins.auto_update_idle_sessions` is on and it has had
/// no event for `plugins.auto_update_idle_after`. System sessions only update
/// on opt-in.
fn wants_update(
    opted_in: bool,
    fleet_policy: bool,
    system: bool,
    idle_for: Option<Duration>,
    idle_after: Duration,
) -> bool {
    opted_in || (fleet_policy && !system && idle_for.is_some_and(|idle| idle >= idle_after))
}

/// Idle period after which policy applies the update unattended, if any.
fn automatic_after(
    opted_in: bool,
    fleet_policy: bool,
    system: bool,
    idle_after: Duration,
) -> Option<Duration> {
    if opted_in {
        Some(Duration::ZERO)
    } else if fleet_policy && !system {
        Some(idle_after)
    } else {
        None
    }
}

/// Machines that may start one automatic update in this pass: none of their
/// sessions is still starting. Updating a whole idle fleet at once relaunches
/// every worker together and can overrun the Machine's command queue.
fn machines_ready_for_update<'a>(
    sessions: impl IntoIterator<Item = (&'a str, Status)>,
) -> HashSet<String> {
    let mut ready = HashSet::new();
    let mut starting = HashSet::new();
    for (machine, status) in sessions {
        if status == Status::Starting {
            starting.insert(machine);
        } else {
            ready.insert(machine.to_owned());
        }
    }
    ready.retain(|machine| !starting.contains(machine.as_str()));
    ready
}

/// A requested update whose session just became idle should not wait for the
/// next full pass.
fn requested_update_ready(state: &AppState) -> bool {
    state.hub.session_list().iter().any(|meta| {
        meta.provider_update_available
            .as_ref()
            .is_some_and(|offer| offer.when_idle)
            && matches!(meta.status, Status::Running | Status::Exited)
            && !state.hub.session_has_in_flight_prompt(&meta.id)
    })
}

pub(super) async fn run(state: Arc<AppState>) {
    let mut shutdown = state.shutdown.clone();
    let mut tick = tokio::time::interval(REQUEST_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pass: Option<Instant> = None;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => {},
        }
        if last_pass.is_some_and(|at| at.elapsed() < PASS_INTERVAL)
            && !requested_update_ready(&state)
        {
            continue;
        }
        last_pass = Some(Instant::now());
        pass(&state, &shutdown).await;
    }
}

type TargetKey = (String, String, String, String);

async fn pass(state: &Arc<AppState>, shutdown: &tokio::sync::watch::Receiver<bool>) {
    let sessions = state.hub.session_list();
    let mut ready = machines_ready_for_update(
        sessions
            .iter()
            .map(|meta| (meta.machine_id.as_str(), meta.status)),
    );
    let fleet_policy = state
        .service_config
        .get(&crate::config::schema::PLUGIN_AUTO_UPDATE_IDLE_SESSIONS);
    let idle_after = state
        .service_config
        .get(&crate::config::schema::PLUGIN_AUTO_UPDATE_IDLE_AFTER);
    let mut targets: HashMap<TargetKey, Option<ResolvedProviderGeneration>> = HashMap::new();
    let mut offers = HashMap::new();
    for meta in sessions {
        if meta.status == Status::Starting || meta.closing {
            continue;
        }
        if !state.runtime_router.connected(&meta.machine_id)
            || !state.hub.provider_version_changeable(&meta.id)
        {
            continue;
        }
        let target_key = (
            meta.machine_id.clone(),
            meta.provider.clone(),
            meta.provider_version.clone(),
            meta.provider_generation_digest.clone(),
        );
        let target = match targets.get(&target_key) {
            Some(target) => target.clone(),
            None => {
                let target = installed_upgrade_target(
                    state,
                    &meta.machine_id,
                    &meta.provider,
                    &meta.provider_version,
                    &meta.provider_generation_digest,
                )
                .await
                .ok();
                targets.insert(target_key, target.clone());
                target
            }
        };
        let when_idle = state.hub.provider_update_when_idle(&meta.id);
        let Some(target) = target else {
            continue;
        };
        if !newer(&meta.provider_version, &target.version) {
            // Nothing newer: a stale one-shot request must not fire later.
            if when_idle {
                state.hub.set_provider_update_when_idle(&meta.id, false);
            }
            continue;
        }
        let opted_in = enabled(&state.hub, &meta.id);
        if !meta.system {
            offers.insert(
                meta.id.clone(),
                ProviderUpdateOffer {
                    version: target.version.clone(),
                    digest: target.digest.clone(),
                    when_idle,
                    automatic_after: automatic_after(
                        opted_in,
                        fleet_policy,
                        meta.system,
                        idle_after,
                    ),
                },
            );
        }
        let still_wanted = || {
            !*shutdown.borrow()
                && wants_update(
                    opted_in || when_idle,
                    fleet_policy,
                    meta.system,
                    state.hub.session_idle_for(&meta.id),
                    idle_after,
                )
        };
        if !still_wanted() {
            continue;
        }
        let applied = match meta.status {
            Status::Running if ready.contains(&meta.machine_id) => {
                let applied = update_running(state, &meta, &still_wanted).await;
                if applied {
                    ready.remove(&meta.machine_id);
                }
                applied
            }
            Status::Exited
                if !holds_worker_slot(&state.runtime_router, &meta)
                    && !state.hub.session_has_in_flight_prompt(&meta.id) =>
            {
                repin_dormant(state, &meta, &target)
            }
            _ => false,
        };
        if applied {
            offers.remove(&meta.id);
            state.hub.settle_provider_update_offer(&meta.id);
        }
    }
    state.hub.publish_provider_update_offers(offers);
}

/// Reload one idle running session through the shared explicit-Reload path,
/// which rechecks the exact old binding and idleness under the Hub lock,
/// preserves native identity, and fences racing prompts.
async fn update_running(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
    still_wanted: &(dyn Fn() -> bool + Sync),
) -> bool {
    let Ok(_fence) = ProviderReloadFence::acquire(
        &state.plugin_lifecycle_fences,
        (meta.machine_id.clone(), meta.provider.clone()),
    ) else {
        return false;
    };
    let target = match session_reload_target(state, meta).await {
        Ok(target) => target,
        Err(error) => {
            tracing::debug!(session = %meta.id, %error, "automatic Provider update deferred");
            return false;
        }
    };
    // Resolution awaited: never downgrade, and let a prompt that arrived
    // meanwhile keep its session.
    if !newer(&meta.provider_version, &target.version) || !still_wanted() {
        return false;
    }
    match apply_session_provider_reload(state, meta, &target, true) {
        Ok(()) => {
            tracing::info!(session = %meta.id, version = %target.version, "automatic Provider update started");
            true
        }
        Err(error) => {
            tracing::debug!(session = %meta.id, %error, "automatic Provider update deferred");
            false
        }
    }
}

/// Move a dormant session's binding without starting it; the next open
/// resumes the same native session on the new release.
fn repin_dormant(
    state: &Arc<AppState>,
    meta: &crate::core::SessionMeta,
    target: &ResolvedProviderGeneration,
) -> bool {
    let Ok(_fence) = ProviderReloadFence::acquire(
        &state.plugin_lifecycle_fences,
        (meta.machine_id.clone(), meta.provider.clone()),
    ) else {
        return false;
    };
    match state
        .hub
        .repin_dormant_provider(meta, &target.version, &target.digest, &target.behavior)
    {
        Ok(()) => {
            tracing::info!(
                session = %meta.id,
                from = %meta.provider_version,
                to = %target.version,
                "dormant session will open on the installed Provider release"
            );
            true
        }
        Err(error) => {
            tracing::debug!(session = %meta.id, %error, "dormant Provider re-pin deferred");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_machine_updates_one_session_at_a_time() {
        let ready = machines_ready_for_update([
            ("ovh", Status::Running),
            ("ovh", Status::Starting),
            ("hawk", Status::Running),
            ("hawk", Status::Exited),
            ("falcon", Status::Starting),
        ]);
        assert_eq!(ready, HashSet::from(["hawk".to_owned()]));
    }

    #[test]
    fn policy_is_opt_in_session_scoped_and_restored() {
        let hub = Hub::new();
        assert!(!enabled(&hub, "one"));
        hub.set_setting(key("one"), serde_json::json!(true));
        assert!(enabled(&hub, "one"));
        assert!(!enabled(&hub, "two"));
        let restored = Hub::new();
        restored.load_settings(hub.settings_snapshot().into_iter().collect());
        assert!(enabled(&restored, "one"));
        restored.set_setting(key("one"), serde_json::json!(false));
        assert!(!enabled(&restored, "one"));
        assert!(serde_json::from_value::<Policy>(serde_json::json!({"enabled": "yes"})).is_err());
        assert!(
            serde_json::from_value::<Policy>(
                serde_json::json!({"enabled": true, "interrupt": true})
            )
            .is_err()
        );
    }

    #[test]
    fn fleet_policy_updates_only_long_idle_non_system_sessions() {
        let hour = std::time::Duration::from_hours(1);
        let idle = |minutes| Some(std::time::Duration::from_mins(minutes));
        // Opt-in alone keeps its previous meaning, regardless of idleness.
        assert!(wants_update(true, false, false, idle(0), hour));
        assert!(wants_update(true, false, true, None, hour));
        // Fleet policy needs the full idle period.
        assert!(!wants_update(false, true, false, idle(59), hour));
        assert!(wants_update(false, true, false, idle(60), hour));
        assert!(!wants_update(false, true, false, None, hour));
        // System sessions and a disabled policy never update without opt-in.
        assert!(!wants_update(false, true, true, idle(600), hour));
        assert!(!wants_update(false, false, false, idle(600), hour));
    }

    #[test]
    fn policy_accepts_the_one_shot_request_alone() {
        let policy: Policy =
            serde_json::from_value(serde_json::json!({"when_idle": true})).unwrap();
        assert_eq!((policy.enabled, policy.when_idle), (None, Some(true)));
        assert!(serde_json::from_value::<Policy>(serde_json::json!({"when_idle": 1})).is_err());
    }

    #[test]
    fn offers_report_when_policy_will_apply_them() {
        let hour = Duration::from_hours(1);
        assert_eq!(
            automatic_after(true, false, true, hour),
            Some(Duration::ZERO)
        );
        assert_eq!(automatic_after(false, true, false, hour), Some(hour));
        assert_eq!(automatic_after(false, true, true, hour), None);
        assert_eq!(automatic_after(false, false, false, hour), None);
    }

    #[test]
    fn automatic_updates_never_downgrade_or_guess_a_version() {
        assert!(newer("3.1.27", "3.1.32"));
        assert!(!newer("3.1.32", "3.1.27"));
        assert!(!newer("3.1.32", "3.1.32"));
        assert!(!newer("legacy", "3.1.32"));
        assert!(!newer("3.1.32", "3.1.33-rc.1"));
    }
}

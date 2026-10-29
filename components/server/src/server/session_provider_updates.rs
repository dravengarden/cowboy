//! Provider update offers and idle-only adoption of the Machine's installed
//! Provider release.
//!
//! Every pass tells clients which sessions have a newer compatible release
//! waiting, then applies it where policy allows: a running session reloads
//! through the explicit Reload gate once it is idle, and a dormant (exited,
//! workerless) session is only re-pinned so its next open starts the new
//! release. Nothing is ever interrupted: a session with background work waits,
//! and a release whose unattended update just crashed a session is held back
//! on that Machine for [`FAILURE_HOLD`]. A worker nobody has used for
//! `plugins.auto_update_idle_limit` is not restarted for every release; it
//! adopts the installed release when its session is next opened.
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::*;
use crate::core::ProviderUpdateOffer;

/// Full passes resolve every session's installed release.
const PASS_INTERVAL: Duration = Duration::from_secs(30);
/// How quickly a requested "update when idle" starts after the turn ends.
const REQUEST_POLL: Duration = Duration::from_secs(5);
/// How long unattended updates to a release pause on a Machine after one of
/// them left a session crashed. An explicit request still goes through.
const FAILURE_HOLD: Duration = Duration::from_mins(30);
/// A session starting for longer than this no longer paces its Machine. Every
/// start phase gives up well inside it (the longest, a native resume, at 240s).
const STARTING_STALL: Duration = Duration::from_mins(10);

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

/// How long an exited session must stay quiet before it counts as dormant.
/// A worker generation cutover exits the old worker at the end of a turn and
/// the Machine starts its replacement on the same release seconds later;
/// re-pinning in that gap would leave the record ahead of the live worker.
const DORMANT_SETTLE: Duration = Duration::from_secs(60);

fn dormant_settled(idle_for: Option<Duration>) -> bool {
    idle_for.is_some_and(|idle| idle >= DORMANT_SETTLE)
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

/// Whether the fleet policy still restarts this session's worker for new
/// releases. Past `idle_limit` nobody is using the session, so it is updated
/// when next opened instead of resuming its native session on every release.
fn keeps_current(idle_for: Option<Duration>, idle_limit: Duration) -> bool {
    idle_for.is_none_or(|idle| idle < idle_limit)
}

/// Whether opening a session should adopt its offered release now: its worker
/// is idle past the prompt-cache window, nothing is running or requested, and
/// it is not a view-only system session.
fn opens_into_update(
    system: bool,
    status: Status,
    in_flight: bool,
    background_tasks: u32,
    offer_requested: Option<bool>,
    idle_for: Option<Duration>,
    idle_after: Duration,
) -> bool {
    !system
        && status == Status::Running
        && !in_flight
        && background_tasks == 0
        && offer_requested == Some(false)
        && idle_for.is_some_and(|idle| idle >= idle_after)
}

/// A client selected `meta`'s session. Under the fleet policy an idle session
/// with a newer release waiting records the same one-shot request as "update
/// when idle", so it is current within seconds, before the first prompt.
pub(super) fn request_on_open(state: &AppState, meta: &crate::core::SessionMeta) {
    if super::session_maintenance::policy(&state.hub, &state.service_config, &meta.machine_id)
        .auto_update_idle_sessions
        != Some(true)
    {
        return;
    }
    let Some(offer) = meta.provider_update_available.as_ref() else {
        return;
    };
    if !opens_into_update(
        meta.system,
        meta.status,
        state.hub.session_has_in_flight_prompt(&meta.id),
        meta.background_tasks,
        Some(offer.when_idle),
        state.hub.session_idle_for(&meta.id),
        state
            .service_config
            .get(&crate::config::schema::PLUGIN_AUTO_UPDATE_IDLE_AFTER),
    ) || state.hub.automatic_update_failed_within(
        &meta.machine_id,
        &meta.provider,
        &offer.digest,
        FAILURE_HOLD,
    ) {
        return;
    }
    state.hub.set_provider_update_when_idle(&meta.id, true);
    tracing::info!(
        session = %meta.id,
        version = %offer.version,
        "idle session opened; adopting the installed Provider release"
    );
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

/// Sessions that have been starting since before `now - STARTING_STALL`. A
/// worker start is bounded by the Machine's own timeouts, so one that never
/// settles is wedged rather than loading the Machine, and must not keep every
/// other session on its Machine from updating. Reports each once.
fn stalled_starts<'a>(
    starting_since: &mut HashMap<String, (Instant, bool)>,
    sessions: impl IntoIterator<Item = (&'a str, Status)>,
    now: Instant,
) -> HashSet<String> {
    let starting: HashSet<&str> = sessions
        .into_iter()
        .filter_map(|(id, status)| (status == Status::Starting).then_some(id))
        .collect();
    starting_since.retain(|id, _| starting.contains(id.as_str()));
    let mut stalled = HashSet::new();
    for id in starting {
        let (since, reported) = starting_since.entry(id.to_owned()).or_insert((now, false));
        if now.saturating_duration_since(*since) < STARTING_STALL {
            continue;
        }
        if !std::mem::replace(reported, true) {
            tracing::warn!(
                session = %id,
                "session has not finished starting; automatic Provider updates no longer wait for it"
            );
        }
        stalled.insert(id.to_owned());
    }
    stalled
}

/// A requested update whose session just became idle, or a session whose
/// Machine just refused its pinned generation, should not wait for the next
/// full pass.
fn requested_update_ready(state: &AppState) -> bool {
    !state.hub.provider_generation_missing_sessions().is_empty()
        || state.hub.session_list().iter().any(|meta| {
            meta.provider_update_available
                .as_ref()
                .is_some_and(|offer| offer.when_idle)
                && matches!(meta.status, Status::Running | Status::Exited)
                && meta.background_tasks == 0
                && !state.hub.session_has_in_flight_prompt(&meta.id)
        })
}

/// Sessions a Machine refused to launch because their pinned generation is
/// gone there (retired under a surviving worker, or never installed on that
/// Machine) are moved to the installed release through the explicit-Reload
/// gate, whatever the update policy says: no open can ever bring them back
/// on their pin. A queued prompt is then delivered by reviving the session,
/// as its open would.
async fn repin_missing(state: &Arc<AppState>) {
    for session_id in state.hub.provider_generation_missing_sessions() {
        let Some(info) = state.hub.session_info(&session_id) else {
            continue;
        };
        let meta = info.meta;
        // Keep the note while something transient blocks the move.
        if meta.closing
            || !matches!(meta.status, Status::Exited | Status::Crashed)
            || !state.runtime_router.connected(&meta.machine_id)
            || state.hub.session_has_in_flight_prompt(&meta.id)
        {
            continue;
        }
        let repinned = {
            let Ok(_decision) =
                super::session_reclaim::decisions(&meta.machine_id).try_lock_owned()
            else {
                continue;
            };
            let Ok(_fence) = ProviderReloadFence::acquire(
                &state.plugin_lifecycle_fences,
                (meta.machine_id.clone(), meta.provider.clone()),
            ) else {
                continue;
            };
            let target = match session_reload_target(state, &meta).await {
                Ok(target) => target,
                Err(error) => {
                    // Nothing automatic can move it; a Reset or Reload must.
                    state.hub.clear_provider_generation_missing(&meta.id);
                    tracing::warn!(
                        session = %meta.id,
                        machine = %meta.machine_id,
                        version = %meta.provider_version,
                        %error,
                        "the session's Provider generation is not installed and it cannot move to the installed release"
                    );
                    continue;
                }
            };
            if target.digest == meta.provider_generation_digest {
                // The installed release itself was refused: not a retired pin.
                state.hub.clear_provider_generation_missing(&meta.id);
                tracing::warn!(
                    session = %meta.id,
                    machine = %meta.machine_id,
                    version = %meta.provider_version,
                    "Machine refused the installed Provider generation it reports; leaving the session on it"
                );
                continue;
            }
            match state.hub.repin_missing_provider(
                &meta,
                &target.version,
                &target.digest,
                &target.behavior,
            ) {
                Ok(()) => {
                    tracing::info!(
                        session = %meta.id,
                        machine = %meta.machine_id,
                        from = %meta.provider_version,
                        to = %target.version,
                        "re-pinned a session whose Provider generation is no longer installed"
                    );
                    true
                }
                Err(error) => {
                    tracing::debug!(session = %meta.id, %error, "missing-generation re-pin deferred");
                    false
                }
            }
        };
        if repinned && info.queue_count > 0 {
            // Cold admission and pause/edit/cancel remain owned by dispatch.
            state.hub.retry_resource_wait(&meta.id);
        }
    }
}

pub(super) async fn run(state: Arc<AppState>) {
    let mut shutdown = state.shutdown.clone();
    let mut tick = tokio::time::interval(REQUEST_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pass: Option<Instant> = None;
    let mut starting_since = HashMap::new();
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
        pass(&state, &shutdown, &mut starting_since).await;
    }
}

type TargetKey = (String, String, String, String);

async fn pass(
    state: &Arc<AppState>,
    shutdown: &tokio::sync::watch::Receiver<bool>,
    starting_since: &mut HashMap<String, (Instant, bool)>,
) {
    repin_missing(state).await;
    let sessions = state.hub.session_list();
    let stalled = stalled_starts(
        starting_since,
        sessions.iter().map(|meta| (meta.id.as_str(), meta.status)),
        Instant::now(),
    );
    let mut ready = machines_ready_for_update(
        sessions
            .iter()
            .filter(|meta| !stalled.contains(&meta.id))
            .map(|meta| (meta.machine_id.as_str(), meta.status)),
    );
    let idle_after = state
        .service_config
        .get(&crate::config::schema::PLUGIN_AUTO_UPDATE_IDLE_AFTER);
    let idle_limit = state
        .service_config
        .get(&crate::config::schema::PLUGIN_AUTO_UPDATE_IDLE_LIMIT);
    // One read of the Machines serves every session's target resolution.
    let machines = match state.store.as_ref() {
        Some(store) => store.list_machines().await.ok(),
        None => None,
    };
    let mut targets: HashMap<TargetKey, Option<ResolvedProviderGeneration>> = HashMap::new();
    let mut offers = HashMap::new();
    for meta in sessions {
        let fleet_policy =
            super::session_maintenance::policy(&state.hub, &state.service_config, &meta.machine_id)
                .auto_update_idle_sessions
                == Some(true);
        // A trashed session is never updated: an update reloads its worker.
        if meta.status == Status::Starting || meta.closing || meta.trashed_at_ms.is_some() {
            continue;
        }
        // A named subscription's session changes release only through a
        // verified login transition; the Machine keeps each named worker on
        // its exact release. Offering Update here would only ever fail.
        if !meta.provider_account_id.is_default() {
            continue;
        }
        // A managed child serves one call and is retired after it; no Update.
        if state.hub.is_managed_child(&meta.id) {
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
                let target = machines
                    .as_deref()
                    .and_then(|machines| {
                        active_machine_plugin(machines, &meta.machine_id, &meta.provider).ok()
                    })
                    .and_then(|installed| {
                        installed_upgrade_target_from(
                            state,
                            installed,
                            &meta.provider,
                            &meta.provider_version,
                            &meta.provider_generation_digest,
                        )
                        .ok()
                    });
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
        // A surviving worker makes an exited session not dormant: re-pinning
        // would be reconciled back to the worker's release on every snapshot.
        // Reload it like a running session so the Machine replaces the worker.
        let live_worker = state
            .runtime_router
            .runtime(&meta.machine_id)
            .is_some_and(|runtime| runtime.has_worker(&meta.id));
        let has_worker = meta.status == Status::Running || live_worker;
        // Only the fleet policy's restart of a live worker waits for the next
        // open: past the idle limit, or after its own unattended update
        // crashed. A request, an opt-in and a free re-pin do not.
        let unattended = !(opted_in || when_idle);
        let on_open = unattended
            && fleet_policy
            && !meta.system
            && has_worker
            && (!keeps_current(state.hub.session_idle_for(&meta.id), idle_limit)
                || state.hub.automatic_update_failed_since_activity(&meta.id));
        if !meta.system {
            offers.insert(
                meta.id.clone(),
                ProviderUpdateOffer {
                    version: target.version.clone(),
                    digest: target.digest.clone(),
                    when_idle,
                    automatic_after: if on_open {
                        None
                    } else {
                        automatic_after(opted_in, fleet_policy, meta.system, idle_after)
                    },
                    on_open,
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
        if !still_wanted() || meta.background_tasks > 0 {
            continue;
        }
        if !when_idle
            && state.hub.automatic_update_failed_within(
                &meta.machine_id,
                &meta.provider,
                &target.digest,
                FAILURE_HOLD,
            )
        {
            continue;
        }
        let applied = match meta.status {
            Status::Running | Status::Exited if has_worker && ready.contains(&meta.machine_id) => {
                if on_open {
                    false
                } else {
                    let applied = update_running(state, &meta, &still_wanted).await;
                    if applied {
                        ready.remove(&meta.machine_id);
                    }
                    applied
                }
            }
            Status::Exited
                if !live_worker
                    && dormant_settled(state.hub.session_idle_for(&meta.id))
                    && !holds_worker_slot(&state.runtime_router, &meta)
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
    // Foreground wakes and memory reclamation own the same decision boundary.
    if super::session_maintenance::has_waiters(state, &meta.machine_id)
        || super::session_maintenance::memory_wait(state, &meta.machine_id).is_some()
    {
        return false;
    }
    let Ok(_decision) = super::session_reclaim::decisions(&meta.machine_id).try_lock_owned() else {
        return false;
    };
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
    let Ok(_decision) = super::session_reclaim::decisions(&meta.machine_id).try_lock_owned() else {
        return false;
    };
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
    fn a_session_that_just_exited_is_not_yet_dormant() {
        assert!(!dormant_settled(None));
        assert!(!dormant_settled(Some(Duration::from_secs(10))));
        assert!(dormant_settled(Some(DORMANT_SETTLE)));
    }

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
    fn a_wedged_start_stops_pacing_its_machine() {
        let start = Instant::now();
        let mut since = HashMap::new();
        let sessions = [("wedged", Status::Starting), ("idle", Status::Running)];
        assert!(stalled_starts(&mut since, sessions, start).is_empty());
        let late = start + STARTING_STALL;
        // A session that began starting later has its own clock.
        let later = [("wedged", Status::Starting), ("fresh", Status::Starting)];
        assert_eq!(
            stalled_starts(&mut since, later, late),
            HashSet::from(["wedged".to_owned()])
        );
        // Settling forgets the clock, so the next start is measured afresh.
        assert!(stalled_starts(&mut since, [("wedged", Status::Running)], late).is_empty());
        assert!(stalled_starts(&mut since, [("wedged", Status::Starting)], late).is_empty());
        assert_eq!(since.len(), 1);
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
    fn a_worker_nobody_uses_waits_for_its_next_open() {
        let limit = Duration::from_hours(12);
        assert!(keeps_current(Some(Duration::from_hours(11)), limit));
        assert!(!keeps_current(Some(limit), limit));
        assert!(keeps_current(None, limit));
        let hour = Duration::from_hours(1);
        let opens = |status, in_flight, background, requested, idle| {
            opens_into_update(false, status, in_flight, background, requested, idle, hour)
        };
        let day = Some(Duration::from_hours(24));
        assert!(opens(Status::Running, false, 0, Some(false), day));
        assert!(opens(Status::Running, false, 0, Some(false), Some(hour)));
        // Inside the prompt-cache window, busy, working, already requested,
        // unoffered, or not a live worker: nothing to do on open.
        assert!(!opens(
            Status::Running,
            false,
            0,
            Some(false),
            Some(Duration::from_mins(59))
        ));
        assert!(!opens(Status::Running, true, 0, Some(false), day));
        assert!(!opens(Status::Busy, false, 0, Some(false), day));
        assert!(!opens(Status::Running, false, 1, Some(false), day));
        assert!(!opens(Status::Running, false, 0, Some(true), day));
        assert!(!opens(Status::Running, false, 0, None, day));
        assert!(!opens(Status::Exited, false, 0, Some(false), day));
        assert!(!opens(Status::Crashed, false, 0, Some(false), day));
        assert!(!opens_into_update(
            true,
            Status::Running,
            false,
            0,
            Some(false),
            day,
            hour
        ));
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

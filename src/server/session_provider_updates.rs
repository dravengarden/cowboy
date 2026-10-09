//! Opt-in, idle-only adoption of the Machine's installed Provider release.
use std::collections::HashSet;

use super::*;

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    enabled: bool,
}

pub(super) async fn configure(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    Json(policy): Json<Policy>,
) -> Response {
    if state.hub.session_info(&session).is_none() {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    state
        .hub
        .set_setting(key(&session), serde_json::json!(policy.enabled));
    Json(serde_json::json!({"enabled": policy.enabled})).into_response()
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

/// A session adopts the installed release when it opted in itself, or when
/// `plugins.auto_update_idle_sessions` is on and it has had no event for
/// `plugins.auto_update_idle_after`. System sessions only update on opt-in.
fn wants_update(
    opted_in: bool,
    fleet_policy: bool,
    system: bool,
    idle_for: Option<std::time::Duration>,
    idle_after: std::time::Duration,
) -> bool {
    opted_in || (fleet_policy && !system && idle_for.is_some_and(|idle| idle >= idle_after))
}

fn eligible(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    wants_update(
        enabled(&state.hub, &meta.id),
        state
            .service_config
            .get(&crate::config::schema::PLUGIN_AUTO_UPDATE_IDLE_SESSIONS),
        meta.system,
        state.hub.session_idle_for(&meta.id),
        state
            .service_config
            .get(&crate::config::schema::PLUGIN_AUTO_UPDATE_IDLE_AFTER),
    )
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

pub(super) async fn run(state: Arc<AppState>) {
    let mut shutdown = state.shutdown.clone();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => {},
        }
        let sessions = state.hub.session_list();
        let mut ready = machines_ready_for_update(
            sessions
                .iter()
                .map(|meta| (meta.machine_id.as_str(), meta.status)),
        );
        for meta in sessions {
            // Do not revive stopped/crashed sessions, or retry a failed native resume.
            if meta.status != Status::Running
                || !ready.contains(&meta.machine_id)
                || !eligible(&state, &meta)
            {
                continue;
            }
            let Ok(_fence) = ProviderReloadFence::acquire(
                &state.plugin_lifecycle_fences,
                (meta.machine_id.clone(), meta.provider.clone()),
            ) else {
                continue;
            };
            let Ok(target) = session_reload_target(&state, &meta).await else {
                continue;
            };
            if !newer(&meta.provider_version, &target.version)
                || !eligible(&state, &meta)
                || *shutdown.borrow()
            {
                continue;
            }
            // The shared path rechecks the exact old binding and idle state under
            // the Hub lock, preserves native identity, and fences racing prompts.
            match apply_session_provider_reload(&state, &meta, &target, true) {
                Ok(()) => {
                    ready.remove(&meta.machine_id);
                    tracing::info!(session = %meta.id, version = %target.version, "automatic Provider update started")
                }
                Err(error) => {
                    tracing::debug!(session = %meta.id, %error, "automatic Provider update deferred")
                }
            }
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
    fn automatic_updates_never_downgrade_or_guess_a_version() {
        assert!(newer("3.1.27", "3.1.32"));
        assert!(!newer("3.1.32", "3.1.27"));
        assert!(!newer("3.1.32", "3.1.32"));
        assert!(!newer("legacy", "3.1.32"));
        assert!(!newer("3.1.32", "3.1.33-rc.1"));
    }
}

//! Opt-in, idle-only adoption of the Machine's installed Provider release.
use super::*;

fn key(session: &str) -> String {
    format!("session_provider_auto_update:{session}")
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

pub(super) async fn run(state: Arc<AppState>) {
    let mut shutdown = state.shutdown.clone();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => {},
        }
        for meta in state.hub.session_list() {
            // Do not revive stopped/crashed sessions, or retry a failed native resume.
            if meta.status != Status::Running || !enabled(&state.hub, &meta.id) {
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
                || !enabled(&state.hub, &meta.id)
                || *shutdown.borrow()
            {
                continue;
            }
            // The shared path rechecks the exact old binding and idle state under
            // the Hub lock, preserves native identity, and fences racing prompts.
            match apply_session_provider_reload(&state, &meta, &target) {
                Ok(()) => {
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
    fn automatic_updates_never_downgrade_or_guess_a_version() {
        assert!(newer("3.1.27", "3.1.32"));
        assert!(!newer("3.1.32", "3.1.27"));
        assert!(!newer("3.1.32", "3.1.32"));
        assert!(!newer("legacy", "3.1.32"));
        assert!(!newer("3.1.32", "3.1.33-rc.1"));
    }
}

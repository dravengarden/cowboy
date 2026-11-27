//! Back for in-place agent changes. A Settings change to model, reasoning or
//! another live option records the value it replaced, once per option, until
//! the next message is sent; restoring re-applies those values through the
//! ordinary configuration path. See `docs/session-context.md`.

use super::*;
use crate::core::settings_keys::SESSION_CONFIG_REVERT_PREFIX;

/// Serializes read-modify-write of one record across Settings changes,
/// restores and dispatch clears.
static REVERT_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct Revert {
    /// Monotonic per session, so every change is announced to other devices.
    revision: u64,
    /// Option id -> the value it had before the first change since the last send.
    values: serde_json::Map<String, serde_json::Value>,
    /// True only when a recorded `deepseek_cache_protection = true` came from
    /// verified interactive owner consent (never a legacy unverified default).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    paid_keepalive_owner_verified: bool,
}

fn key(id: &str) -> String {
    format!("{SESSION_CONFIG_REVERT_PREFIX}{id}")
}

fn read(hub: &crate::core::Hub, id: &str) -> Revert {
    hub.setting(&key(id))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn write(hub: &crate::core::Hub, id: &str, revert: &Revert) {
    hub.set_setting(
        key(id),
        serde_json::to_value(revert).expect("config revert serializes"),
    );
    if let Some(owner) = hub
        .session_info(id)
        .and_then(|info| info.meta.owner_user_id)
    {
        hub.announce_preferences(
            &owner,
            &format!("session_config_revert:{id}"),
            i64::try_from(revert.revision).unwrap_or(i64::MAX),
        );
    }
}

/// Fold one change into the record before it is applied: keep the value it
/// replaces the first time, and forget the option once it is set back.
fn fold(
    revert: &mut Revert,
    config_id: &str,
    previous: Option<serde_json::Value>,
    next: &serde_json::Value,
    paid_keepalive_owner_verified: bool,
) -> bool {
    let is_cache_protection = config_id == crate::managed_config::CACHE_PROTECTION_ID;
    match revert.values.get(config_id) {
        Some(original) if original == next => {
            revert.values.remove(config_id);
            if is_cache_protection {
                revert.paid_keepalive_owner_verified = false;
            }
            true
        }
        Some(_) => false,
        None => match previous {
            Some(previous) if &previous != next => {
                if is_cache_protection {
                    revert.paid_keepalive_owner_verified =
                        previous == serde_json::Value::Bool(true) && paid_keepalive_owner_verified;
                }
                revert.values.insert(config_id.to_owned(), previous);
                true
            }
            _ => false,
        },
    }
}

fn record(
    hub: &crate::core::Hub,
    id: &str,
    config_id: &str,
    previous: Option<serde_json::Value>,
    next: &serde_json::Value,
    paid_keepalive_owner_verified: bool,
) {
    let _guard = REVERT_LOCK.lock();
    let mut revert = read(hub, id);
    if fold(
        &mut revert,
        config_id,
        previous,
        next,
        paid_keepalive_owner_verified,
    ) {
        revert.revision += 1;
        write(hub, id, &revert);
    }
}

/// A message was dispatched: the change is now part of the conversation.
pub fn clear(hub: &crate::core::Hub, id: &str) {
    let _guard = REVERT_LOCK.lock();
    let mut revert = read(hub, id);
    if revert.values.is_empty() && !revert.paid_keepalive_owner_verified {
        return;
    }
    revert.values.clear();
    revert.paid_keepalive_owner_verified = false;
    revert.revision += 1;
    write(hub, id, &revert);
}

pub const MANAGED_CHILD_REFUSAL: &str = "managed child configuration is owned by its call";

/// Apply one live option change: the Web, Desktop and Agent API path.
pub fn apply_for_caller(
    state: &AppState,
    session_id: &str,
    config_id: String,
    value: serde_json::Value,
    remember: bool,
    caller: &crate::prompt_origin::RequestCaller,
) -> Result<(), String> {
    apply(
        state,
        session_id,
        config_id,
        value,
        remember,
        matches!(caller, crate::prompt_origin::RequestCaller::Person),
    )
}

pub fn apply(
    state: &AppState,
    session_id: &str,
    config_id: String,
    value: serde_json::Value,
    remember: bool,
    owner_authorized: bool,
) -> Result<(), String> {
    // The call runner alone configures a managed child: its receipt records
    // the preset that passed admission, so another writer would falsify it.
    if state.hub.is_managed_child(session_id) {
        return Err(MANAGED_CHILD_REFUSAL.to_owned());
    }
    session_configuration::prepare(state, session_id);
    // Read what the change replaces before applying it; record it only once
    // the change is accepted, so a refused value leaves nothing to go back to.
    // For paid keepalive, record only the effective verified consent (never a
    // raw legacy `true` default) so Back cannot promote unverified billing.
    let (previous, previous_paid_verified) = if !remember {
        (None, false)
    } else if config_id == crate::managed_config::CACHE_PROTECTION_ID {
        let prefs = state
            .hub
            .config_preferences(session_id)
            .unwrap_or(serde_json::Value::Null);
        let effective_on =
            crate::managed_config::effective_cache_protection_consent(&prefs) == Some(true);
        (Some(serde_json::Value::Bool(effective_on)), effective_on)
    } else {
        (
            state.hub.current_config_value(session_id, &config_id),
            false,
        )
    };
    let recorded = (config_id.clone(), value.clone());
    let revoking_cache_protection = config_id == crate::managed_config::CACHE_PROTECTION_ID
        && value == serde_json::Value::Bool(false);
    let applied = match state.supervisor.set_managed_config_option_for_caller(
        session_id,
        &config_id,
        value.clone(),
        owner_authorized,
    ) {
        Ok(true) => Ok(()),
        Ok(false) => state
            .hub
            .set_config_preference(session_id, config_id.clone(), value.clone())
            .and_then(|()| {
                state.supervisor.send(
                    session_id,
                    AgentCommand::SetConfigOption { config_id, value },
                )
            }),
        Err(error) => Err(error),
    };
    if applied.is_ok() {
        if revoking_cache_protection
            && let Some(meta) = state.hub.session_info(session_id).map(|info| info.meta)
        {
            let configuration = meta.provider_behavior.as_ref().map_or_else(
                || crate::provider::legacy_behavior(&meta.provider).configuration,
                |behavior| behavior.configuration.clone(),
            );
            if crate::deepseek_cache::supported_behavior(&configuration) {
                let machine_control = Arc::clone(&state.machine_control);
                let machine_id = meta.machine_id;
                let sid = meta.id;
                tokio::spawn(async move {
                    if let Err(error) = machine_control
                        .adapter_request(
                            &machine_id,
                            "provider-cache-revoke",
                            serde_json::json!({
                                "configuration": configuration,
                                "sessionId": sid,
                            }),
                        )
                        .await
                    {
                        tracing::debug!(
                            session = %sid,
                            machine = %machine_id,
                            %error,
                            "best-effort gateway cache-protection revoke did not complete"
                        );
                    }
                });
            }
        }
        if remember {
            record(
                &state.hub,
                session_id,
                &recorded.0,
                previous,
                &recorded.1,
                previous_paid_verified,
            );
        }
    }
    applied
}

pub async fn get(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    if state.hub.session_info(&id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    Json(read(&state.hub, &id)).into_response()
}

pub async fn restore(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
) -> Response {
    if state.hub.session_info(&id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // The same admission as a live option change from Settings: never on a
    // trashed session, never across a Provider lifecycle change.
    if state.hub.session_is_trashed(&id) {
        return (
            StatusCode::CONFLICT,
            "Restore this session from Trash first",
        )
            .into_response();
    }
    let fences = state.plugin_lifecycle_fences.read();
    if let Some(refusal) = provider_fence_key_for_session(&state.hub, &id)
        .as_ref()
        .and_then(|key| fences.get(key))
        .and_then(|fence| provider_fence_refusal(*fence))
    {
        return (StatusCode::CONFLICT, refusal).into_response();
    }
    let (values, paid_keepalive_owner_verified) = {
        let _guard = REVERT_LOCK.lock();
        let mut revert = read(&state.hub, &id);
        if revert.values.is_empty() {
            return (
                StatusCode::CONFLICT,
                "Nothing to go back to: a message was sent since the change",
            )
                .into_response();
        }
        let values = std::mem::take(&mut revert.values);
        let verified = std::mem::take(&mut revert.paid_keepalive_owner_verified);
        revert.revision += 1;
        write(&state.hub, &id, &revert);
        (values, verified)
    };
    // The agent's own option order (model before reasoning), so a value that
    // depends on the earlier one is applied after it.
    let order = state
        .hub
        .persisted_config_options(&id)
        .and_then(|options| options.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|option| {
            option
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    let mut pending = values.into_iter().collect::<Vec<_>>();
    pending.sort_by_key(|(config_id, _)| {
        order
            .iter()
            .position(|known| known == config_id)
            .unwrap_or(usize::MAX)
    });
    let mut remaining = pending.into_iter();
    while let Some((config_id, value)) = remaining.next() {
        let is_paid_enable = config_id == crate::managed_config::CACHE_PROTECTION_ID
            && value == serde_json::Value::Bool(true);
        let owner_authorized = matches!(auth.caller, crate::prompt_origin::RequestCaller::Person)
            && (!is_paid_enable || paid_keepalive_owner_verified);
        if let Err(error) = apply(
            &state,
            &id,
            config_id.clone(),
            value.clone(),
            false,
            owner_authorized,
        ) {
            // Keep Back for what was not applied.
            let _guard = REVERT_LOCK.lock();
            let mut revert = read(&state.hub, &id);
            for (config_id, value) in std::iter::once((config_id, value)).chain(remaining) {
                if config_id == crate::managed_config::CACHE_PROTECTION_ID
                    && value == serde_json::Value::Bool(true)
                    && paid_keepalive_owner_verified
                {
                    revert.paid_keepalive_owner_verified = true;
                }
                revert.values.entry(config_id).or_insert(value);
            }
            revert.revision += 1;
            write(&state.hub, &id, &revert);
            return (StatusCode::CONFLICT, error).into_response();
        }
    }
    drop(fences);
    Json(read(&state.hub, &id)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn back_keeps_the_first_value_and_forgets_a_change_set_back() {
        let mut revert = Revert::default();
        // Opus -> Sonnet records Opus.
        assert!(fold(
            &mut revert,
            "model",
            Some(json!("opus")),
            &json!("sonnet"),
            false,
        ));
        // Sonnet -> Haiku keeps the original Opus.
        assert!(!fold(
            &mut revert,
            "model",
            Some(json!("sonnet")),
            &json!("haiku"),
            false,
        ));
        assert_eq!(revert.values.get("model"), Some(&json!("opus")));
        // Choosing Opus again leaves nothing to go back to.
        assert!(fold(
            &mut revert,
            "model",
            Some(json!("haiku")),
            &json!("opus"),
            false,
        ));
        assert!(revert.values.is_empty());
        // Re-selecting the current value, or an unknown previous one, records nothing.
        assert!(!fold(
            &mut revert,
            "effort",
            Some(json!("high")),
            &json!("high"),
            false,
        ));
        assert!(!fold(&mut revert, "effort", None, &json!("high"), false));
        assert!(revert.values.is_empty());
    }

    #[test]
    fn back_requires_verified_owner_provenance_for_paid_keepalive() {
        let mut revert = Revert::default();
        let id = crate::managed_config::CACHE_PROTECTION_ID;
        // Effective value for a legacy unverified default is false, so explicit Off records nothing.
        assert!(!fold(
            &mut revert,
            id,
            Some(json!(false)),
            &json!(false),
            false
        ));
        assert!(revert.values.is_empty());
        assert!(!revert.paid_keepalive_owner_verified);

        // Verified owner On -> Off records true WITH provenance, and toggling back clears it.
        assert!(fold(
            &mut revert,
            id,
            Some(json!(true)),
            &json!(false),
            true
        ));
        assert_eq!(revert.values.get(id), Some(&json!(true)));
        assert!(revert.paid_keepalive_owner_verified);
        assert!(fold(
            &mut revert,
            id,
            Some(json!(false)),
            &json!(true),
            false
        ));
        assert!(revert.values.is_empty());
        assert!(!revert.paid_keepalive_owner_verified);

        // Legacy persisted Back records without provenance deserialize unverified.
        let legacy: Revert = serde_json::from_value(json!({
            "revision": 2,
            "values": { id: true }
        }))
        .unwrap();
        assert!(!legacy.paid_keepalive_owner_verified);
    }
}

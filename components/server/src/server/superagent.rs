//! Project supervision over ordinary sessions. Configuration is durable owner
//! state; model work is event-driven, bounded and attributed to Cowboy.
use super::*;
use crate::core::SessionMeta;
use crate::superagent::{Mode, Settings};
use serde_json::{Value, json};
use std::sync::LazyLock;

mod actions;
mod dispatch;
pub(super) use dispatch::dispatch;
mod scope;
#[cfg(test)]
use scope::evidence;
use scope::{Evidence, live, same_machine, target, visible};
#[cfg(test)]
mod tests;
mod wake;

pub(super) use actions::execute;
pub(super) use wake::run;
/// Serializes configuration admission with the last policy check and dispatch.
/// No model/tool turn or remote command holds this lock.
pub(super) static WRITES: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

pub(super) async fn settings(
    state: &AppState,
    owner: &str,
) -> Result<(i64, Settings), &'static str> {
    let prefs = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .preferences(owner, crate::superagent::SCOPE)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("unavailable")?;
    let config = serde_json::from_value(prefs.values["configuration"].clone())
        .map_err(|_| "invalid_superagent_settings")?;
    Ok((prefs.revision, config))
}

/// Even off coordinators retain the restricted role until the owner unbinds
/// them. Otherwise Off would accidentally restore broad owner-level tools.
pub(super) async fn restricted(state: &AppState, meta: &SessionMeta) -> Result<bool, &'static str> {
    let Some(owner) = meta.owner_user_id.as_deref() else {
        return Ok(false);
    };
    let projects = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .superagent_projects(owner)
        .await
        .map_err(|_| "unavailable")?;
    let projects = projects.as_object().ok_or("invalid_superagent_bindings")?;
    if projects
        .values()
        .any(|p| p["coordinator"].as_str().is_none())
    {
        return Err("invalid_superagent_bindings");
    }
    Ok(projects
        .values()
        .any(|p| p["coordinator"].as_str() == Some(&meta.id)))
}

pub(super) async fn validate_change(
    state: &AppState,
    owner: &str,
    value: &Value,
) -> Result<(), &'static str> {
    let (old, config) = editing_settings(state, owner, value).await?;
    for project in config.projects.values() {
        if state.hub.is_managed_child(&project.coordinator) {
            return Err("managed_child_not_eligible");
        }
        if project.mode == Mode::Off
            && old.coordinator(&project.coordinator).is_some()
            && !config.sessions.keys().any(|id| {
                config
                    .effective(id)
                    .is_some_and(|(_, p, m)| p.coordinator == project.coordinator && m != Mode::Off)
            })
        {
            continue;
        }
        let coordinator = visible(&state.hub, owner, &project.coordinator)?;
        if old.coordinator(&project.coordinator).is_none()
            && (state.hub.session_has_in_flight_prompt(&coordinator.id)
                || state
                    .hub
                    .session_info(&coordinator.id)
                    .is_some_and(|i| i.queue_count != 0))
        {
            return Err("coordinator_must_be_idle");
        }
        if project.mode != Mode::Off && old.projects.values().all(|p| p != project) {
            model_ready(state, &coordinator, project.preset)?;
        }
    }
    for (id, participation) in &config.sessions {
        if state.hub.is_managed_child(id) {
            return Err("managed_child_not_eligible");
        }
        if config
            .effective(id)
            .is_some_and(|(_, _, mode)| mode == Mode::Off)
        {
            continue;
        }
        let worker = visible(&state.hub, owner, id)?;
        let coordinator = visible(
            &state.hub,
            owner,
            &config.projects[&participation.project].coordinator,
        )?;
        // MVP does not widen the existing execution-Machine boundary.
        if !same_machine(&worker, &coordinator) {
            return Err("machine_scope_denied");
        }
    }
    for project in old.projects.values() {
        if config.coordinator(&project.coordinator).is_none()
            && state.hub.session_has_in_flight_prompt(&project.coordinator)
        {
            return Err("coordinator_must_be_idle");
        }
    }
    Ok(())
}

pub(super) async fn validate_editor(
    state: &AppState,
    owner: &str,
    actor: &SessionMeta,
    value: &Value,
) -> Result<(), &'static str> {
    let (before, after) = editing_settings(state, owner, value).await?;
    scope::editable(
        &state.hub,
        &state.colocated_machines,
        actor,
        &before,
        &after,
    )
}

async fn editing_settings(
    state: &AppState,
    owner: &str,
    changes: &Value,
) -> Result<(Settings, Settings), &'static str> {
    let prefs = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .preferences(owner, crate::superagent::SCOPE)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("unavailable")?;
    let current = &prefs.values["configuration"];
    let after = Settings::patch_value(current, changes)?;
    let mut bindings = current.clone();
    // Keep every old binding for permission and busy-unbind checks, even when
    // source decoding is broken. A replacement still touches all coordinators.
    let before = match serde_json::from_value::<Settings>(bindings.clone()) {
        Ok(before) => before,
        Err(_) => {
            bindings["sources"] = json!([]);
            serde_json::from_value(bindings).map_err(|_| "invalid_superagent_bindings")?
        }
    };
    Ok((before, after))
}

fn model_ready(
    state: &AppState,
    meta: &SessionMeta,
    preset: crate::superagent::Preset,
) -> Result<(), &'static str> {
    if state.hub.config_changes_pending(&meta.id) {
        return Err("coordinator_configuration_pending");
    }
    use crate::superagent::Preset;
    let (provider, preset) = match preset {
        Preset::AstraMax => ("codex", "astra-max"),
        Preset::FableMax => ("claude-code", "fable-max"),
    };
    let package = state
        .provider_catalog
        .package(
            provider,
            &meta.provider_version,
            &meta.provider_generation_digest,
        )
        .ok_or("coordinator_provider_unavailable")?;
    let values = &package
        .manifest
        .configuration
        .presets
        .iter()
        .find(|p| p.id == preset)
        .ok_or("coordinator_preset_unavailable")?
        .values;
    if meta.provider != provider
        || values.is_empty()
        || !super::native_configuration::matches(
            state.supervisor.observed_configuration(&meta.id).as_ref(),
            &json!(values),
        )
    {
        return Err("superagent_requires_selected_max_model");
    }
    Ok(())
}

pub(super) async fn read(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    op: &str,
    args: Value,
) -> Result<Value, String> {
    let owner = meta.owner_user_id.as_deref().ok_or("permission_denied")?;
    let _guard = WRITES.lock().await;
    let (revision, config) = settings(state, owner).await?;
    let (project_id, project) = config.coordinator(&meta.id).ok_or("coordinator_required")?;
    let library = crate::superagent::patterns::Library::compile(&config.sources)?;
    let catalog = library.resolve(&project.profile)?;
    let value = match op {
        "superagent.catalog" => {
            json!({"revision":revision,"project":project_id,"mode":project.mode,
            "learning":project.learning,"max_followups":project.max_followups,"catalog":catalog.discovery(),
            "sessions":config.sessions.keys().filter_map(|id| target(&state.hub,owner,meta,&config,id).ok()
                .map(|(_,_,mode)| json!({"id":id,"mode":mode}))).collect::<Vec<_>>() })
        }
        "superagent.document" => {
            let id = args["id"].as_str().ok_or("invalid_argument")?;
            let body = catalog
                .patterns
                .get(id)
                .map(|p| &p.body)
                .or_else(|| catalog.guidance.get(id))
                .ok_or("not_found")?;
            page(body, &args)?
        }
        "superagent.context" => {
            let id = args["session_id"].as_str().ok_or("invalid_argument")?;
            let (participation, _, mode) = target(&state.hub, owner, meta, &config, id)?;
            let evidence = scope::current_evidence(state, id).await?;
            target(&state.hub, owner, meta, &config, id)?;
            json!({"session_id":id,"evidence":evidence.marker,"turn_seq":evidence.seq,
                "mode":mode,"mandate":participation.mandate,"catalog_digest":catalog.digest,
                "transcript":page(&evidence.text,&args)?})
        }
        _ => return Err("unknown_operation".into()),
    };
    Ok(value)
}

fn page(text: &str, args: &Value) -> Result<Value, &'static str> {
    let offset = args
        .get("offset")
        .map_or(Some(0), Value::as_u64)
        .ok_or("invalid_offset")? as usize;
    let chars = args
        .get("chars")
        .map_or(Some(2000), Value::as_u64)
        .filter(|n| *n <= 4000)
        .ok_or("invalid_chars")? as usize;
    let length = text.chars().count();
    let part: String = text.chars().skip(offset).take(chars).collect();
    let end = offset.saturating_add(part.chars().count());
    Ok(
        json!({"text":part,"range":{"offset":offset,"next":(end < length).then_some(end),"total":length}}),
    )
}

#[derive(Default, Deserialize)]
pub(super) struct ActivityQuery {
    before: Option<String>,
}

pub(super) async fn activity(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Query(query): Query<ActivityQuery>,
) -> Response {
    let cursor: (i64, String, String) = match query.before {
        Some(value) => match serde_json::from_str(&value) {
            Ok(cursor) => cursor,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        },
        None => (i64::MAX, String::new(), String::new()),
    };
    let result = match state.store.as_ref() {
        Some(store) => {
            store
                .superagent_activity(&auth.principal.user_id, &cursor)
                .await
        }
        None => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    match result {
        Ok(rows) => {
            let next = (rows.len() == 100)
                .then(|| rows.last())
                .flatten()
                .map(|row| {
                    serde_json::to_string(&(
                        row["created_at_ms"].as_i64().unwrap(),
                        row["operation"].as_str().unwrap(),
                        row["session_id"].as_str().unwrap(),
                    ))
                    .unwrap()
                });
            (
                [(header::CACHE_CONTROL, "no-store")],
                Json(json!({"activity":rows,"next":next})),
            )
                .into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

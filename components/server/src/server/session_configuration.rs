//! Configure a new session without starting its Provider. Signed presets and
//! defaults define the portable choices; a matching ACP snapshot can supply
//! labels and additional choices, never another session's selected values.

use cowboy_provider_sdk::ConfigurationPreset;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use super::AppState;
use crate::core::SessionMeta;

fn same_scope(left: &SessionMeta, right: &SessionMeta) -> bool {
    left.provider == right.provider
        && left.provider_account_id == right.provider_account_id
        && left.provider_version == right.provider_version
        && left.provider_generation_digest == right.provider_generation_digest
        && left.provider_auth_generation == right.provider_auth_generation
        && left.machine_id == right.machine_id
        && left.owner_user_id == right.owner_user_id
        && right
            .execution_binding
            .as_ref()
            .is_none_or(|binding| binding.managed().is_none())
}

/// [`prepare`] for a client or Agent reading the session's settings: also
/// offers an exited session the model choices of its pinned generation. Not
/// for configuration writes, whose ACP waiters a preview broadcast carrying
/// the previous selection would resolve.
pub(super) fn prepare_for_view(state: &AppState, session_id: &str) {
    match state.hub.persisted_config_options(session_id) {
        Some(snapshot) => carry_model_choices(state, session_id, &snapshot),
        None => prepare(state, session_id),
    }
}

pub(super) fn prepare(state: &AppState, session_id: &str) {
    if state.hub.persisted_config_options(session_id).is_some() {
        return;
    }
    let Some(info) = state.hub.session_info(session_id) else {
        return;
    };
    let meta = info.meta;
    if meta
        .execution_binding
        .as_ref()
        .is_some_and(|binding| binding.managed().is_some())
    {
        return;
    }
    // An embedded/latest presentation may describe a different installed
    // runtime. Only the exact signed generation can promise launch values.
    let Some(package) = state.provider_catalog.package(
        &meta.provider,
        &meta.provider_version,
        &meta.provider_generation_digest,
    ) else {
        return;
    };
    let preferences = state
        .hub
        .config_preferences(session_id)
        .unwrap_or_else(|| json!({}));
    let observed = state
        .hub
        .session_list()
        .into_iter()
        .rev()
        .filter(|candidate| candidate.id != session_id && same_scope(&meta, candidate))
        .filter_map(|candidate| state.hub.persisted_config_options(&candidate.id))
        .find(|options| model_matches(options, &preferences));
    let options = preview(
        &package.manifest.configuration.presets,
        &preferences,
        observed.as_ref(),
    );
    state.hub.set_initial_config_options(&meta, options);
}

/// The options a new session of `target` would offer before it exists, for
/// a form choosing a copy's configuration. Same signed preview as
/// [`prepare`]; a same-scope snapshot supplies labels and choices, preferring
/// one whose model matches, else any (it never supplies selected values).
pub(super) fn target_preview(state: &AppState, target: &SessionMeta, preferences: &Value) -> Option<Value> {
    if is_managed(target) {
        return None;
    }
    let package = state.provider_catalog.package(
        &target.provider,
        &target.provider_version,
        &target.provider_generation_digest,
    )?;
    let peers = state
        .hub
        .session_list()
        .into_iter()
        .rev()
        .filter(|candidate| same_scope(target, candidate))
        .filter_map(|candidate| state.hub.persisted_config_options(&candidate.id))
        .collect::<Vec<_>>();
    let observed = peers
        .iter()
        .find(|options| model_matches(options, preferences))
        .or_else(|| peers.first());
    Some(preview(
        &package.manifest.configuration.presets,
        preferences,
        observed,
    ))
}

/// An exited session keeps the options its last worker reported, which may
/// come from an earlier Provider generation: a dormant re-pin or an operator
/// update moves the session without starting a worker. Offer, for display
/// only, the model choices of the generation it is pinned to now, so its
/// settings and Recommended cards match what its next start runs.
///
/// Sources, without starting anything: the snapshots of sessions with a live
/// worker on the same exact generation, Machine, owner, account and login
/// (the native model list depends on the login). Only when there is none, the
/// generation's signed preset models, if the snapshot is known to come from
/// another generation, or, when its producer is unknown (after a Controller
/// restart), if it lacks the generation's signed default model and so
/// predates it. A snapshot this Controller saw the pinned generation report
/// needs nothing.
fn carry_model_choices(state: &AppState, session_id: &str, snapshot: &Value) {
    let Some(info) = state.hub.session_info(session_id) else {
        return;
    };
    let meta = info.meta;
    if meta.status != crate::core::Status::Exited || is_managed(&meta) {
        return;
    }
    let model_ids = model_option_ids(snapshot);
    if model_ids.is_empty() {
        return;
    }
    let reported_by_current = state.hub.config_reported_by_current(&meta);
    let mut choices = BTreeMap::<String, Vec<Value>>::new();
    if reported_by_current == Some(true) {
        // Nothing to add; still replace an earlier lookup below.
    } else if let Some(peers) = state.hub.live_config_peers(&meta) {
        for options in &peers {
            collect_model_choices(options, &model_ids, &mut choices);
        }
    } else if let Some(package) = state.provider_catalog.package(
        &meta.provider,
        &meta.provider_version,
        &meta.provider_generation_digest,
    ) && let Some(behavior) = meta.provider_behavior.as_ref()
    {
        let predates = reported_by_current == Some(false)
            || model_ids.iter().any(|id| {
                behavior
                    .default_preferences
                    .get(id)
                    .and_then(|value| serde_json::to_value(value).ok())
                    .is_some_and(|default| !offers(snapshot, id, &default))
            });
        if predates {
            for preset in &package.manifest.configuration.presets {
                for id in &model_ids {
                    if let Some(value) = preset.values.get(id) {
                        let name = preset
                            .name
                            .split(" · ")
                            .next()
                            .filter(|name| !name.is_empty())
                            .map_or_else(|| label(value), str::to_owned);
                        let entry = choices.entry(id.clone()).or_default();
                        if !entry.iter().any(|choice| choice["value"] == json!(value)) {
                            entry.push(json!({"value": value, "name": name}));
                        }
                    }
                }
            }
        }
    }
    state
        .hub
        .set_dormant_config_choices(&meta, snapshot, choices);
}

fn is_managed(meta: &SessionMeta) -> bool {
    meta.execution_binding
        .as_ref()
        .is_some_and(|binding| binding.managed().is_some())
}

/// The Web picker's rule: the `model` category, or the legacy `model` id
/// when the agent reports no category.
fn is_model_option(option: &Value) -> bool {
    option["category"] == "model" || (option["category"].is_null() && option["id"] == "model")
}

fn model_option_ids(options: &Value) -> Vec<String> {
    options
        .as_array()
        .into_iter()
        .flatten()
        .filter(|option| is_model_option(option))
        .filter_map(|option| option["id"].as_str().map(str::to_owned))
        .collect()
}

fn offers(options: &Value, id: &str, value: &Value) -> bool {
    options.as_array().into_iter().flatten().any(|option| {
        option["id"] == id
            && option["options"]
                .as_array()
                .is_some_and(|choices| choices.iter().any(|choice| &choice["value"] == value))
    })
}

/// Only flat, named string choices: never a peer's selection or a group.
fn collect_model_choices(
    options: &Value,
    ids: &[String],
    additions: &mut BTreeMap<String, Vec<Value>>,
) {
    for option in options.as_array().into_iter().flatten() {
        if !is_model_option(option) {
            continue;
        }
        let Some(id) = option["id"]
            .as_str()
            .filter(|id| ids.iter().any(|known| known == id))
        else {
            continue;
        };
        for choice in option["options"].as_array().into_iter().flatten() {
            if choice["value"].is_string() && choice["name"].is_string() {
                let choices = additions.entry(id.to_owned()).or_default();
                if !choices
                    .iter()
                    .any(|known| known["value"] == choice["value"])
                {
                    choices.push(json!({"value": choice["value"], "name": choice["name"]}));
                }
            }
        }
    }
}

fn model_matches(options: &Value, preferences: &Value) -> bool {
    options.as_array().is_some_and(|options| {
        options.iter().all(|option| {
            if option["category"] != "model" {
                return true;
            }
            let Some(id) = option["id"].as_str() else {
                return false;
            };
            preferences
                .get(id)
                .is_some_and(|value| cowboy::native_configuration::confirmed(option, value))
        })
    })
}

fn label(id: &str) -> String {
    let text = id.replace('_', " ");
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

fn preview(
    presets: &[ConfigurationPreset],
    preferences: &Value,
    observed: Option<&Value>,
) -> Value {
    let mut values = std::collections::BTreeMap::<String, Vec<Value>>::new();
    for preset in presets {
        for (id, value) in &preset.values {
            let choices = values.entry(id.clone()).or_default();
            let value = json!(value);
            if !choices.contains(&value) {
                choices.push(value);
            }
        }
    }
    // These are the target session's signed defaults or explicit selections.
    // Do not guess defaults from a peer, especially its permissions or model.
    if let Some(preferences) = preferences.as_object() {
        for (id, value) in preferences {
            let choices = values.entry(id.clone()).or_default();
            if !choices.contains(value) {
                choices.push(value.clone());
            }
        }
    }
    let observed = observed.and_then(Value::as_array);
    json!(values.into_iter().filter_map(|(id, values)| {
        let current = preferences.get(&id).cloned().or_else(|| {
            presets.iter().find(|preset| preset.is_default)
                .and_then(|preset| preset.values.get(&id)).map(|value| json!(value))
        })?;
        let option = observed.and_then(|options| options.iter().find(|option| option["id"] == id));
        let mut choices = option.and_then(|option| option["options"].as_array())
            .map(|choices| choices.iter().filter(|choice| {
                choice.get("value").is_some_and(|value| value.is_string() || value.is_boolean())
                    && choice["name"].is_string()
            }).cloned().collect::<Vec<_>>()).unwrap_or_default();
        for value in values {
            if !choices.iter().any(|choice| choice["value"] == value) {
                let name = value.as_str().map_or_else(|| value.to_string(), label);
                choices.push(json!({"value": value, "name": name}));
            }
        }
        let category = option.and_then(|option| option.get("category")).cloned().unwrap_or_else(|| {
            match id.as_str() {
                "model" => json!("model"),
                // ACP names the reasoning category `thought_level`; a Provider
                // may use it as the option id too (Antigravity).
                "effort" | "reasoning_effort" | "thought_level" => json!("thought_level"),
                _ => Value::Null,
            }
        });
        Some(json!({
            "id": id,
            "name": option.and_then(|option| option["name"].as_str()).map_or_else(|| label(&id), str::to_owned),
            "category": category,
            "type": "select",
            "currentValue": current,
            "options": choices,
        }))
    }).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_choices_are_available_without_an_agent_or_another_session() {
        let presets = vec![
            ConfigurationPreset {
                id: "standard".into(),
                name: "Standard".into(),
                detail: String::new(),
                is_default: true,
                values: [
                    ("model".into(), "small".into()),
                    ("effort".into(), "normal".into()),
                ]
                .into(),
            },
            ConfigurationPreset {
                id: "deep".into(),
                name: "Deep".into(),
                detail: String::new(),
                is_default: false,
                values: [
                    ("model".into(), "large".into()),
                    ("effort".into(), "high".into()),
                ]
                .into(),
            },
        ];
        let options = preview(
            &presets,
            &json!({"model": "small", "effort": "normal"}),
            None,
        );
        let model = options
            .as_array()
            .unwrap()
            .iter()
            .find(|option| option["id"] == "model")
            .unwrap();
        assert_eq!(model["currentValue"], "small");
        assert_eq!(model["options"].as_array().unwrap().len(), 2);
        assert_eq!(model["category"], "model");
    }

    #[test]
    fn an_acp_thought_level_option_previews_as_reasoning_before_any_agent_reports_it() {
        let preset = |id: &str, level: &str, is_default| ConfigurationPreset {
            id: id.into(),
            name: id.into(),
            detail: String::new(),
            is_default,
            values: [("model", "gemini-3.8-flash"), ("thought_level", level)]
                .into_iter()
                .map(|(id, value)| (id.to_owned(), value.to_owned()))
                .collect(),
        };
        let presets = vec![
            preset("gemini-flash-high", "high", true),
            preset("gemini-flash-low", "low", false),
        ];
        let options = preview(&presets, &json!({}), None);
        let level = options
            .as_array()
            .unwrap()
            .iter()
            .find(|option| option["id"] == "thought_level")
            .unwrap();
        assert_eq!(level["category"], "thought_level");
        assert_eq!(level["currentValue"], "high");
        assert_eq!(level["options"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn observed_choices_never_copy_peer_selections_or_project_options() {
        let options = preview(
            &[],
            &json!({"model": "small", "mode": "ask"}),
            Some(&json!([
                {"id":"model","name":"Model","currentValue":"large","options":[{"value":"small","name":"Small model"}]},
                {"id":"mode","name":"Permissions","currentValue":"bypass","options":[{"value":"ask","name":"Ask"},{"value":"bypass","name":"Bypass"}]},
                {"id":"workspace_agent","name":"Private agent","currentValue":"secret","options":[]}
            ])),
        );
        assert_eq!(options.as_array().unwrap().len(), 2);
        assert_eq!(options[0]["currentValue"], "ask");
        assert_eq!(options[1]["currentValue"], "small");
        assert_eq!(options[1]["options"][0]["name"], "Small model");
        assert!(!model_matches(
            &json!([{"id":"model","category":"model","currentValue":"large"}]),
            &json!({"model":"small"})
        ));
    }

    #[test]
    fn peer_model_choices_carry_names_but_never_selections_or_other_options() {
        let peer = json!([
            {"id":"model","category":"model","currentValue":"gpt-6.2-astra","options":[
                {"value":"gpt-6.2-sol","name":"6.2 Sol","description":"fast"},
                {"value":"gpt-6.2-astra","name":"6.2 Astra"},
                {"group":"Other","options":[]}
            ]},
            {"id":"reasoning_effort","category":"thought_level","currentValue":"ultra",
             "options":[{"value":"ultra","name":"Ultra"}]}
        ]);
        let ids = model_option_ids(&peer);
        assert_eq!(ids, vec!["model".to_owned()]);
        let mut additions = BTreeMap::new();
        collect_model_choices(&peer, &ids, &mut additions);
        collect_model_choices(&peer, &ids, &mut additions);
        assert_eq!(additions.len(), 1);
        assert_eq!(
            additions["model"],
            vec![
                json!({"value":"gpt-6.2-sol","name":"6.2 Sol"}),
                json!({"value":"gpt-6.2-astra","name":"6.2 Astra"}),
            ]
        );
        assert!(offers(&peer, "model", &json!("gpt-6.2-sol")));
        assert!(!offers(&peer, "model", &json!("gpt-6.1-sol")));
        assert!(!offers(&peer, "reasoning_effort", &json!("gpt-6.2-sol")));
    }
}

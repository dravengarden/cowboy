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

pub(super) fn prepare(state: &AppState, session_id: &str) {
    if let Some(snapshot) = state.hub.persisted_config_options(session_id) {
        carry_model_choices(state, session_id, &snapshot);
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

/// An exited session keeps the options its last worker reported, which may
/// come from an earlier Provider generation: a dormant re-pin or an operator
/// update moves the session without starting a worker. Offer the model choices
/// of the generation it is pinned to now, so its settings and Recommended
/// cards match what its next start runs. Sources, without starting anything:
/// the snapshots of other sessions on the same exact generation, Machine,
/// owner and account (the native model list depends on the account), and,
/// when the snapshot lacks the generation's signed default model and therefore
/// predates it, the generation's signed preset models. The next worker
/// snapshot replaces all of it.
fn carry_model_choices(state: &AppState, session_id: &str, snapshot: &Value) {
    let Some(info) = state.hub.session_info(session_id) else {
        return;
    };
    let meta = info.meta;
    if meta.status != crate::core::Status::Exited
        || meta
            .execution_binding
            .as_ref()
            .is_some_and(|binding| binding.managed().is_some())
    {
        return;
    }
    let model_ids = model_option_ids(snapshot);
    if model_ids.is_empty() {
        return;
    }
    let mut additions = BTreeMap::<String, Vec<Value>>::new();
    let peers = state.hub.session_list().into_iter().filter(|peer| {
        peer.id != meta.id
            && peer.provider == meta.provider
            && peer.provider_account_id == meta.provider_account_id
            && peer.provider_version == meta.provider_version
            && peer.provider_generation_digest == meta.provider_generation_digest
            && peer.machine_id == meta.machine_id
            && peer.owner_user_id == meta.owner_user_id
    });
    // Live workers first: their snapshot is certainly of this generation.
    let (live, dormant): (Vec<_>, Vec<_>) =
        peers.partition(|peer| peer.status != crate::core::Status::Exited);
    for peer in live.into_iter().chain(dormant) {
        if let Some(options) = state.hub.persisted_config_options(&peer.id) {
            collect_model_choices(&options, &model_ids, &mut additions);
        }
    }
    if let Some(package) = state.provider_catalog.package(
        &meta.provider,
        &meta.provider_version,
        &meta.provider_generation_digest,
    ) && let Some(behavior) = meta.provider_behavior.as_ref()
    {
        let defaults = &behavior.default_preferences;
        let predates = model_ids.iter().any(|id| {
            defaults
                .get(id)
                .and_then(|value| serde_json::to_value(value).ok())
                .is_some_and(|default| {
                    !offers(snapshot, id, &default)
                        && !additions
                            .get(id)
                            .is_some_and(|choices| choices.iter().any(|c| c["value"] == default))
                })
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
                        let choices = additions.entry(id.clone()).or_default();
                        if !choices.iter().any(|choice| choice["value"] == json!(value)) {
                            choices.push(json!({"value": value, "name": name}));
                        }
                    }
                }
            }
        }
    }
    if !additions.is_empty() {
        state.hub.add_dormant_config_choices(&meta, &additions);
    }
}

fn model_option_ids(options: &Value) -> Vec<String> {
    options
        .as_array()
        .into_iter()
        .flatten()
        .filter(|option| option["category"] == "model")
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
                .is_some_and(|value| option.get("currentValue") == Some(value))
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
                "effort" | "reasoning_effort" => json!("thought_level"),
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

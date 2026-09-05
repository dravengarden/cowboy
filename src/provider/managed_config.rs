//! Cowboy-owned configuration effects selected by a signed Provider behavior.
//!
//! ACP options remain opaque protocol data. A small set of host-managed options
//! instead change worker launch policy and therefore cannot be forwarded to the
//! adapter. Keep their identifiers and effect dispatch behind this registry so
//! the Hub, server, supervisor, and remote runtime never branch on a
//! Provider-specific option id.

#![warn(clippy::pedantic)]

use cowboy_provider_sdk::ConfigurationBehavior;

const CONTEXT_BUDGET_ID: &str = "deepseek_context";
const CACHE_PROTECTION_ID: &str = "deepseek_cache_protection";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedConfigEffect {
    ContextBudgetV1,
    CacheProtectionV1,
}

const GATEWAY_EFFECTS: &[ManagedConfigEffect] = &[
    ManagedConfigEffect::ContextBudgetV1,
    ManagedConfigEffect::CacheProtectionV1,
];

fn effects(configuration: &ConfigurationBehavior) -> &'static [ManagedConfigEffect] {
    match configuration {
        ConfigurationBehavior::AnthropicGatewayV1 | ConfigurationBehavior::OpenaiGatewayV1 => {
            GATEWAY_EFFECTS
        }
        ConfigurationBehavior::PortableV1
        | ConfigurationBehavior::AcpConfigOptionsV1
        | ConfigurationBehavior::XaiSessionV1 => &[],
    }
}

impl ManagedConfigEffect {
    const fn id(self) -> &'static str {
        match self {
            Self::ContextBudgetV1 => CONTEXT_BUDGET_ID,
            Self::CacheProtectionV1 => CACHE_PROTECTION_ID,
        }
    }
}

#[must_use]
pub fn effect_for_id(
    configuration: &ConfigurationBehavior,
    config_id: &str,
) -> Option<ManagedConfigEffect> {
    effects(configuration)
        .iter()
        .copied()
        .find(|effect| effect.id() == config_id)
}

#[must_use]
pub fn is_managed(configuration: &ConfigurationBehavior, config_id: &str) -> bool {
    effect_for_id(configuration, config_id).is_some()
}

#[must_use]
pub fn context_budget(
    configuration: &ConfigurationBehavior,
    preferences: &serde_json::Value,
) -> Option<crate::deepseek_context::ContextBudget> {
    let effect = effects(configuration)
        .iter()
        .find(|effect| **effect == ManagedConfigEffect::ContextBudgetV1)?;
    let model = preferences.get("model").and_then(serde_json::Value::as_str);
    let requested = preferences
        .get(effect.id())
        .and_then(serde_json::Value::as_str);
    crate::deepseek_context::launch_budget(configuration, model, requested)
}

#[must_use]
pub fn cache_protection(
    configuration: &ConfigurationBehavior,
    preferences: &serde_json::Value,
) -> Option<bool> {
    let effect = effects(configuration)
        .iter()
        .find(|effect| **effect == ManagedConfigEffect::CacheProtectionV1)?;
    crate::deepseek_cache::selected(preferences, configuration, effect.id())
}

fn option_for(
    effect: ManagedConfigEffect,
    configuration: &ConfigurationBehavior,
    preferences: &serde_json::Value,
) -> Option<serde_json::Value> {
    match effect {
        ManagedConfigEffect::ContextBudgetV1 => {
            let model = preferences.get("model").and_then(serde_json::Value::as_str);
            let requested = preferences
                .get(effect.id())
                .and_then(serde_json::Value::as_str);
            crate::deepseek_context::config_option(configuration, model, requested, effect.id())
        }
        ManagedConfigEffect::CacheProtectionV1 => crate::deepseek_cache::config_option(
            configuration,
            cache_protection(configuration, preferences)?,
            effect.id(),
        ),
    }
}

/// Merge host-managed options into an adapter's protocol options. Managed ids
/// are removed first so an adapter cannot shadow or override host policy.
#[must_use]
pub fn projected_options(
    configuration: &ConfigurationBehavior,
    preferences: &serde_json::Value,
    options: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let had_options = options.is_some();
    let managed_effects = effects(configuration);
    let mut options = options.unwrap_or_else(|| serde_json::json!([]));
    let Some(array) = options.as_array_mut() else {
        return Some(options);
    };

    array.retain(|option| {
        option
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|id| effect_for_id(configuration, id).is_none())
    });

    let mut insert_at = array
        .iter()
        .position(|candidate| {
            candidate.get("id").and_then(serde_json::Value::as_str) == Some("model")
        })
        .map_or(array.len(), |index| index.saturating_add(1));
    for effect in managed_effects {
        if let Some(option) = option_for(*effect, configuration, preferences) {
            array.insert(insert_at, option);
            insert_at = insert_at.saturating_add(1);
        }
    }

    (had_options || !managed_effects.is_empty()).then_some(options)
}

/// Validate a proposed managed value and report whether it already represents
/// the effective launch policy.
pub fn change_is_unchanged(
    effect: ManagedConfigEffect,
    configuration: &ConfigurationBehavior,
    preferences: &serde_json::Value,
    value: &serde_json::Value,
) -> Result<bool, String> {
    match effect {
        ManagedConfigEffect::ContextBudgetV1 => {
            let profile = value
                .as_str()
                .ok_or_else(|| "context budget profile must be a string id".to_owned())?;
            let model = preferences.get("model").and_then(serde_json::Value::as_str);
            crate::deepseek_context::resolve(configuration, model, profile)?;
            Ok(preferences
                .get(effect.id())
                .and_then(serde_json::Value::as_str)
                == Some(profile))
        }
        ManagedConfigEffect::CacheProtectionV1 => {
            let enabled = value
                .as_bool()
                .ok_or_else(crate::deepseek_cache::boolean_required_message)?;
            if !crate::deepseek_cache::supported_behavior(configuration) {
                return Err(crate::deepseek_cache::unavailable_message());
            }
            Ok(cache_protection(configuration, preferences) == Some(enabled))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_behavior_selects_managed_effects_without_a_provider_id() {
        for behavior in [
            ConfigurationBehavior::AnthropicGatewayV1,
            ConfigurationBehavior::OpenaiGatewayV1,
        ] {
            assert_eq!(
                effect_for_id(&behavior, CONTEXT_BUDGET_ID),
                Some(ManagedConfigEffect::ContextBudgetV1)
            );
            assert_eq!(
                effect_for_id(&behavior, CACHE_PROTECTION_ID),
                Some(ManagedConfigEffect::CacheProtectionV1)
            );
            assert_eq!(effect_for_id(&behavior, "future_option"), None);
        }
        assert_eq!(
            effect_for_id(&ConfigurationBehavior::PortableV1, CONTEXT_BUDGET_ID),
            None
        );
    }

    #[test]
    fn projection_replaces_adapter_spoofs_and_preserves_portable_options() {
        let adapter = serde_json::json!([
            {"id": "model", "currentValue": "deepseek-v4-flash"},
            {"id": "deepseek_context", "currentValue": "spoofed"},
            {"id": "reasoning_effort", "currentValue": "max"},
        ]);
        let preferences = serde_json::json!({
            "model": "deepseek-v4-flash",
            "deepseek_context": "680k",
            "deepseek_cache_protection": false,
        });
        let projected = projected_options(
            &ConfigurationBehavior::OpenaiGatewayV1,
            &preferences,
            Some(adapter.clone()),
        )
        .expect("gateway options");
        assert_eq!(projected[1]["id"], CONTEXT_BUDGET_ID);
        assert_eq!(projected[1]["currentValue"], "680k");
        assert_eq!(projected[2]["id"], CACHE_PROTECTION_ID);
        assert_eq!(projected[2]["currentValue"], false);
        assert_eq!(projected[3]["id"], "reasoning_effort");

        assert_eq!(
            projected_options(
                &ConfigurationBehavior::PortableV1,
                &preferences,
                Some(adapter.clone()),
            ),
            Some(adapter)
        );
        assert_eq!(
            projected_options(&ConfigurationBehavior::PortableV1, &preferences, None,),
            None
        );
    }
}

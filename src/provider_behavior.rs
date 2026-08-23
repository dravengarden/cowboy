//! Bounded behavior fallback for sessions created before exact Provider
//! generations persisted their signed behavior contract.
//!
//! New sessions carry `ProviderBehaviorContract` over the runtime wire. Keep
//! this compatibility table independent of the Controller launch registry so a
//! Machine-only build can drain legacy sessions without compiling Provider
//! process recipes.

pub(crate) const COMPONENT_COMMANDS_ENV: &str = "COWBOY_PROVIDER_COMPONENT_COMMANDS";

#[must_use]
pub(crate) fn legacy_behavior(id: &str) -> cowboy_provider_sdk::ProviderBehaviorContract {
    use cowboy_provider_sdk::{
        ConfigurationBehavior, PermissionBehavior, ProviderBehaviorContract, SessionBehavior,
        TurnEndBehavior,
    };
    const LEGACY_SOURCES: [(&str, &str); 6] = [
        (
            "claude-code",
            include_str!("../plugins/claude-code/provider.json"),
        ),
        ("codex", include_str!("../plugins/codex/provider.json")),
        ("gemini", include_str!("../plugins/gemini/provider.json")),
        ("grok", include_str!("../plugins/grok/provider.json")),
        (
            "claude-deepseek",
            include_str!("../plugins/claude-deepseek/provider.json"),
        ),
        (
            "codex-deepseek",
            include_str!("../plugins/codex-deepseek/provider.json"),
        ),
    ];
    if let Some((_, source)) = LEGACY_SOURCES
        .iter()
        .find(|(candidate, _)| *candidate == id)
        && let Ok(source) =
            serde_json::from_str::<cowboy_provider_sdk::StandardProviderSource>(source)
        && let Ok(manifest) = source.compile()
    {
        return manifest.runtime.behavior;
    }
    ProviderBehaviorContract {
        schema_version: 1,
        permission: PermissionBehavior::PortableV1,
        session: SessionBehavior::PortableV1,
        turn_end: TurnEndBehavior::PortableV1,
        configuration: ConfigurationBehavior::PortableV1,
        default_preferences: std::collections::BTreeMap::new(),
        error_rules: Vec::new(),
    }
}

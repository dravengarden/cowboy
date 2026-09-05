//! Bounded behavior fallback for sessions created before exact Provider
//! generations persisted their signed behavior contract.
//!
//! New sessions carry `ProviderBehaviorContract` over the runtime wire. Keep
//! this compatibility table independent of the Controller launch registry so a
//! Machine-only build can drain legacy sessions without compiling Provider
//! process recipes.

pub(crate) const COMPONENT_COMMANDS_ENV: &str = "COWBOY_PROVIDER_COMPONENT_COMMANDS";
pub(crate) const PROVIDER_AUTH_REQUIRED_PREFIX: &str = "Provider authentication required - ";

#[must_use]
pub(crate) fn is_provider_auth_required_error(detail: &str) -> bool {
    detail.contains(PROVIDER_AUTH_REQUIRED_PREFIX)
}

#[must_use]
pub(crate) fn legacy_behavior(id: &str) -> cowboy_provider_sdk::ProviderBehaviorContract {
    use cowboy_provider_sdk::{
        ConfigurationBehavior, PermissionBehavior, ProviderBehaviorContract, SessionBehavior,
        TurnEndBehavior,
    };
    if let Some((_, source)) = crate::first_party_sources::PROVIDER_SOURCES
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

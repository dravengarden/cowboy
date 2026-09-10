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

/// ACP `session/resume` / `session/load` timed out while restoring a native
/// thread. This is a session-local hydrate cost, not a worker-generation
/// defect: falling back to a previous binary retries the same unbounded
/// restore. OpenSession must not auto-revive these crashes; an explicit
/// prompt, Retry, or Reload still may.
#[must_use]
pub(crate) fn is_native_session_restore_timeout(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("did not complete acp session/resume")
        || detail.contains("did not complete acp session/load")
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

#[cfg(test)]
mod tests {
    use super::{is_native_session_restore_timeout, is_provider_auth_required_error};

    #[test]
    fn native_restore_timeouts_are_detected_through_worker_wrappers() {
        assert!(is_native_session_restore_timeout(
            "agent did not complete ACP session/resume within 240s"
        ));
        assert!(is_native_session_restore_timeout(
            "worker sess-1 exited before readiness: agent did not complete ACP session/resume within 240s"
        ));
        assert!(is_native_session_restore_timeout(
            "fallback after generation launch failed: worker sess-1 exited before readiness: agent did not complete ACP session/load within 60s"
        ));
        assert!(!is_native_session_restore_timeout(
            "agent did not complete ACP initialize within 60s"
        ));
        assert!(!is_native_session_restore_timeout(
            "worker sess-1 exited before readiness with exit status: 1"
        ));
        assert!(!is_provider_auth_required_error(
            "agent did not complete ACP session/resume within 240s"
        ));
    }
}

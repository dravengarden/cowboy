//! The closed set of Hub settings the `settings` table persists.
//!
//! Both storage backends refuse any other key on write and skip it on load, so
//! retired product settings (for example `session.autoResume.default`) stay
//! ignored. Every `Hub::set_setting` caller must use a key registered here;
//! `examples()` drives the backend round-trip tests.

use crate::admin;

/// Session id -> first epoch ms a retention pass saw the session dormant.
pub const SESSION_DORMANT_SINCE: &str = "session_dormant_since";
/// UI-owned per-Machine maintenance policy; TOML supplies inherited defaults.
pub const MACHINE_MAINTENANCE_PREFIX: &str = "machine_maintenance:";
/// A never-dispatched prompt waiting for resources, resumable after restart.
pub const SESSION_RESOURCE_WAIT_PREFIX: &str = "session_resource_wait:";
/// Explicit queue pause must survive a Controller restart.
pub const SESSION_QUEUE_PAUSED_PREFIX: &str = "session_queue_paused:";
pub const SESSION_DELIVERY_POLICY_PREFIX: &str = "session_delivery_policy:";
pub const SESSION_DELIVERY_HOLD_PREFIX: &str = "session_delivery_hold:";
pub const SESSION_CONTEXT_PREFIX: &str = "session_context:";
/// The prompt that carries a session's linked context; the session id follows.
pub const SESSION_CONTEXT_CARRIER_PREFIX: &str = "session_context_carrier:";
/// The session a continuation was created from; the destination id follows.
pub const SESSION_CONTINUED_FROM_PREFIX: &str = "session_continued_from:";
/// A fork's source and pending native fork; the destination id follows.
pub const SESSION_FORK_PREFIX: &str = "session_fork:";
/// Per-session opt-in for idle Provider updates; the session id follows.
pub const SESSION_PROVIDER_AUTO_UPDATE_PREFIX: &str = "session_provider_auto_update:";
/// Agent tools defaults for one agent kind; the agent id follows.
pub const AGENT_TOOLS_PREFIX: &str = "agent_tools:";
/// A session's agent tools override; the session id follows.
pub const SESSION_TOOLS_PREFIX: &str = "session_tools:";
/// Values an in-place agent change replaced, restorable until the next send;
/// the session id follows.
pub const SESSION_CONFIG_REVERT_PREFIX: &str = "session_config_revert:";
/// One-shot request to adopt the installed Provider release once the session
/// is idle; the session id follows.
pub const SESSION_PROVIDER_UPDATE_WHEN_IDLE_PREFIX: &str = "session_provider_update_when_idle:";

#[must_use]
pub fn is_persisted_setting_key(key: &str) -> bool {
    admin::is_admin_setting_key(key)
        || key == SESSION_DORMANT_SINCE
        || [
            MACHINE_MAINTENANCE_PREFIX,
            SESSION_RESOURCE_WAIT_PREFIX,
            SESSION_QUEUE_PAUSED_PREFIX,
            SESSION_DELIVERY_POLICY_PREFIX,
            SESSION_DELIVERY_HOLD_PREFIX,
            SESSION_CONTEXT_PREFIX,
            SESSION_CONTEXT_CARRIER_PREFIX,
            SESSION_CONTINUED_FROM_PREFIX,
            SESSION_FORK_PREFIX,
            SESSION_PROVIDER_AUTO_UPDATE_PREFIX,
            AGENT_TOOLS_PREFIX,
            SESSION_TOOLS_PREFIX,
            SESSION_PROVIDER_UPDATE_WHEN_IDLE_PREFIX,
            SESSION_CONFIG_REVERT_PREFIX,
        ]
        .into_iter()
        .any(|prefix| {
            key.strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty())
        })
}

/// One concrete key per registered setting.
#[cfg(any(test, feature = "test-support"))]
pub fn examples() -> Vec<String> {
    vec![
        admin::REGISTRATION_SETTING.to_owned(),
        admin::PERMISSIONS_SETTING.to_owned(),
        admin::SESSION_LIMITS_SETTING.to_owned(),
        admin::ADMIN_IDENTITIES_SETTING.to_owned(),
        SESSION_DORMANT_SINCE.to_owned(),
        format!("{MACHINE_MAINTENANCE_PREFIX}ovh"),
        format!("{SESSION_RESOURCE_WAIT_PREFIX}sess-1"),
        format!("{SESSION_QUEUE_PAUSED_PREFIX}sess-1"),
        format!("{SESSION_DELIVERY_POLICY_PREFIX}sess-1"),
        format!("{SESSION_DELIVERY_HOLD_PREFIX}sess-1"),
        format!("{SESSION_CONTEXT_PREFIX}sess-1"),
        format!("{SESSION_CONTEXT_CARRIER_PREFIX}sess-1"),
        format!("{SESSION_CONTINUED_FROM_PREFIX}sess-1"),
        format!("{SESSION_FORK_PREFIX}sess-1"),
        format!("{SESSION_PROVIDER_AUTO_UPDATE_PREFIX}sess-1"),
        format!("{AGENT_TOOLS_PREFIX}codex"),
        format!("{SESSION_TOOLS_PREFIX}sess-1"),
        format!("{SESSION_PROVIDER_UPDATE_WHEN_IDLE_PREFIX}sess-1"),
        format!("{SESSION_CONFIG_REVERT_PREFIX}sess-1"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_accepts_every_example_and_nothing_retired() {
        for key in examples() {
            assert!(is_persisted_setting_key(&key), "{key}");
        }
        for key in [
            "session.autoResume.default",
            "unrelated",
            SESSION_PROVIDER_AUTO_UPDATE_PREFIX,
            AGENT_TOOLS_PREFIX,
            SESSION_TOOLS_PREFIX,
            SESSION_PROVIDER_UPDATE_WHEN_IDLE_PREFIX,
            SESSION_CONFIG_REVERT_PREFIX,
        ] {
            assert!(!is_persisted_setting_key(key), "{key}");
        }
    }
}

//! The closed set of Hub settings the `settings` table persists.
//!
//! Both storage backends refuse any other key on write and skip it on load, so
//! retired product settings (for example `session.autoResume.default`) stay
//! ignored. Every `Hub::set_setting` caller must use a key registered here;
//! `examples()` drives the backend round-trip tests.

use crate::admin;

/// Session id -> first epoch ms a retention pass saw the session dormant.
pub const SESSION_DORMANT_SINCE: &str = "session_dormant_since";
/// Per-session opt-in for idle Provider updates; the session id follows.
pub const SESSION_PROVIDER_AUTO_UPDATE_PREFIX: &str = "session_provider_auto_update:";

#[must_use]
pub fn is_persisted_setting_key(key: &str) -> bool {
    admin::is_admin_setting_key(key)
        || key == SESSION_DORMANT_SINCE
        || key
            .strip_prefix(SESSION_PROVIDER_AUTO_UPDATE_PREFIX)
            .is_some_and(|session| !session.is_empty())
}

/// One concrete key per registered setting.
#[cfg(test)]
pub(crate) fn examples() -> Vec<String> {
    vec![
        admin::REGISTRATION_SETTING.to_owned(),
        admin::PERMISSIONS_SETTING.to_owned(),
        admin::SESSION_LIMITS_SETTING.to_owned(),
        admin::ADMIN_IDENTITIES_SETTING.to_owned(),
        SESSION_DORMANT_SINCE.to_owned(),
        format!("{SESSION_PROVIDER_AUTO_UPDATE_PREFIX}sess-1"),
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
        ] {
            assert!(!is_persisted_setting_key(key), "{key}");
        }
    }
}

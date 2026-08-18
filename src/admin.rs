//! Admin-plane policy types that must not leak onto the product `/ws`.
//!
//! Product clients never receive invite tokens, permission grants, session
//! limits, or admin identities. [`RegistrationPublicStatus`] is the only
//! public registration shape (HTTP status later; omitted from `/ws` Settings).

#![warn(clippy::pedantic)]

use serde::{Deserialize, Serialize};

pub const REGISTRATION_SETTING: &str = "cowboy.registration";
pub const PERMISSIONS_SETTING: &str = "cowboy.permissions";
pub const SESSION_LIMITS_SETTING: &str = "cowboy.session_limits";
pub const ADMIN_IDENTITIES_SETTING: &str = "cowboy.admin.identities";

/// Keys that persist on the Hub but must never appear on product `/ws`.
#[must_use]
pub fn is_admin_setting_key(key: &str) -> bool {
    matches!(
        key,
        REGISTRATION_SETTING
            | PERMISSIONS_SETTING
            | SESSION_LIMITS_SETTING
            | ADMIN_IDENTITIES_SETTING
    )
}

/// Synapse-shaped registration switch. The service, not the client, decides.
#[allow(dead_code)] // `/api/auth/status` consumes this type in a later PR
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    #[default]
    Disabled,
    Token,
    Open,
}

/// Stored registration policy. Token rows stay service-private.
#[allow(dead_code)] // `/api/auth/status` consumes this type in a later PR
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationPolicy {
    pub enabled: bool,
    pub mode: RegistrationMode,
}

#[allow(dead_code)] // `/api/auth/status` consumes these methods in a later PR
impl RegistrationPolicy {
    #[must_use]
    pub fn accepts_registration(&self) -> bool {
        self.enabled && !matches!(self.mode, RegistrationMode::Disabled)
    }

    /// Three-field public view. Never serialize tokens here.
    #[must_use]
    pub fn public_status(&self) -> RegistrationPublicStatus {
        RegistrationPublicStatus {
            enabled: self.enabled,
            mode: self.mode,
            accepts_registration: self.accepts_registration(),
        }
    }
}

/// Product-visible registration flags. No consume path and no invite table.
#[allow(dead_code)] // `/api/auth/status` consumes this type in a later PR
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationPublicStatus {
    pub enabled: bool,
    pub mode: RegistrationMode,
    pub accepts_registration: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_rejects_registration() {
        let policy = RegistrationPolicy::default();
        assert!(!policy.enabled);
        assert_eq!(policy.mode, RegistrationMode::Disabled);
        assert!(!policy.accepts_registration());
        let status = policy.public_status();
        assert!(!status.enabled);
        assert!(!status.accepts_registration);
    }

    #[test]
    fn public_status_has_only_the_three_product_fields() {
        let status = RegistrationPolicy {
            enabled: true,
            mode: RegistrationMode::Token,
        }
        .public_status();
        let json = serde_json::to_value(&status).expect("status serializes");
        let object = json.as_object().expect("object");
        assert_eq!(object.len(), 3);
        assert_eq!(object.get("enabled"), Some(&serde_json::json!(true)));
        assert_eq!(object.get("mode"), Some(&serde_json::json!("token")));
        assert_eq!(
            object.get("accepts_registration"),
            Some(&serde_json::json!(true))
        );
        assert!(object.get("tokens").is_none());
        assert!(object.get("token").is_none());
    }
}

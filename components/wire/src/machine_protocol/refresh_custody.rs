//! Refresh custody of named portable subscription accounts (protocol 51).
//!
//! A Provider whose exact signed release ships [`REFRESH_CUSTODY_DECLARATION`]
//! keeps a named account's single-use refresh token out of every Machine: the
//! Service seals each Machine an access-only copy ([`RefreshCustody::strip`])
//! and restores the full credential only inside one bounded
//! `RenewPortableCredential` operation at a time. See
//! `docs/portable-refresh-lease.md` ("Grok implementation design").
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::PortableCredentialBundle;

/// The signed host-bundle file declaring refresh custody.
pub const REFRESH_CUSTODY_DECLARATION: &str = "collector/refresh-custody.json";

/// Upper bound of one renewal on the Machine, from receipt.
pub const RENEWAL_MACHINE_DEADLINE_MS: u64 = 60_000;

/// The Service admits a new renewal of an account only after the previous
/// one settled or started this long ago: far beyond the Machine's deadline.
pub const RENEWAL_REUSE_WINDOW_MS: i64 = 300_000;

/// A Machine with a live worker asks for a renewal this long before expiry.
pub const RENEWAL_LEAD_MS: i64 = 240_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshCustody {
    pub schema: u16,
    /// Bundle value (base64 JSON document) holding the credential entries.
    pub bundle_key: String,
    /// Fields removed from every entry of that document for access-only copies.
    pub strip_entry_fields: Vec<String>,
    /// Entry field holding the access token's expiry (RFC 3339 or epoch).
    pub expiry_entry_field: String,
    /// Variable the CLI reads its credential file path from.
    pub credential_path_env: String,
    /// Additional variables of a renewal run (make the CLI renew at start).
    #[serde(default)]
    pub renew_environment: std::collections::BTreeMap<String, String>,
}

fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

impl RefreshCustody {
    /// Parse and validate a declaration's bytes.
    ///
    /// # Errors
    /// If the document is malformed or names unsafe fields or variables.
    pub fn parse(text: &str) -> Result<Self, String> {
        let custody: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if custody.schema != 1 {
            return Err("unsupported refresh custody schema".into());
        }
        let fields_ok = !custody.strip_entry_fields.is_empty()
            && custody
                .strip_entry_fields
                .iter()
                .all(|field| safe_name(field))
            && safe_name(&custody.expiry_entry_field)
            && safe_name(&custody.bundle_key);
        let env_ok = |name: &str| {
            safe_name(name)
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_' || byte.is_ascii_digit())
                && !matches!(name, "PATH" | "HOME" | "LD_PRELOAD" | "LD_LIBRARY_PATH")
        };
        if !fields_ok
            || !env_ok(&custody.credential_path_env)
            || !custody
                .renew_environment
                .iter()
                .all(|(name, value)| env_ok(name) && value.len() <= 256)
            || custody.renew_environment.len() > 8
        {
            return Err("invalid refresh custody declaration".into());
        }
        Ok(custody)
    }

    /// The declaration among a verified host bundle's files, if any.
    ///
    /// # Errors
    /// If the file is present but invalid.
    pub fn from_files(
        files: &std::collections::BTreeMap<String, String>,
    ) -> Result<Option<Self>, String> {
        files
            .get(REFRESH_CUSTODY_DECLARATION)
            .map(|text| Self::parse(text))
            .transpose()
    }

    fn document(&self, bundle: &PortableCredentialBundle) -> Option<serde_json::Value> {
        let encoded = bundle.values.get(&self.bundle_key)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn entries(
        document: &serde_json::Value,
    ) -> impl Iterator<Item = &serde_json::Map<String, serde_json::Value>> {
        document
            .as_object()
            .into_iter()
            .flat_map(|object| object.values())
            .filter_map(serde_json::Value::as_object)
    }

    /// The access-only copy: every entry loses `strip_entry_fields`.
    ///
    /// # Errors
    /// If the declared value is missing or not a JSON object document; an
    /// unstrippable bundle must never be sealed as if it were access-only.
    pub fn strip(
        &self,
        bundle: &PortableCredentialBundle,
    ) -> Result<PortableCredentialBundle, String> {
        let mut document = self
            .document(bundle)
            .filter(serde_json::Value::is_object)
            .ok_or("refresh custody bundle value is not a JSON object")?;
        if let Some(object) = document.as_object_mut() {
            for entry in object.values_mut() {
                if let Some(entry) = entry.as_object_mut() {
                    for field in &self.strip_entry_fields {
                        entry.remove(field);
                    }
                }
            }
        }
        let mut stripped = bundle.clone();
        stripped.values.insert(
            self.bundle_key.clone(),
            base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(&document).map_err(|error| error.to_string())?),
        );
        Ok(stripped)
    }

    /// Whether the bundle still carries a field an access-only copy lacks.
    #[must_use]
    pub fn holds_refresh_material(&self, bundle: &PortableCredentialBundle) -> bool {
        self.document(bundle).is_some_and(|document| {
            Self::entries(&document).any(|entry| {
                self.strip_entry_fields
                    .iter()
                    .any(|field| entry.get(field).is_some_and(|value| !value.is_null()))
            })
        })
    }

    /// Earliest access-token expiry across the entries, in epoch milliseconds.
    #[must_use]
    pub fn expiry_ms(&self, bundle: &PortableCredentialBundle) -> Option<i64> {
        let document = self.document(bundle)?;
        Self::entries(&document)
            .filter_map(|entry| match entry.get(&self.expiry_entry_field)? {
                serde_json::Value::String(text) => chrono::DateTime::parse_from_rfc3339(text)
                    .ok()
                    .map(|time| time.timestamp_millis()),
                serde_json::Value::Number(number) => number.as_i64().map(|value| {
                    if value < 1_000_000_000_000 {
                        value.saturating_mul(1000)
                    } else {
                        value
                    }
                }),
                _ => None,
            })
            .min()
    }
}

/// One renewal: the ordinary portable collection of `usage`, run with the
/// full credential that `envelope` seals to this Machine only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableRenewalRequest {
    pub renewal_id: String,
    pub usage: super::portable_usage::PortableUsageRequest,
    /// The vault's full bundle at `usage.auth_generation`, sealed to this
    /// Machine; it is restored into a private file and never persisted.
    /// Absent when `resume_only`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<super::SealedProviderAuth>,
    /// Return the retained reply of `renewal_id` (a lost reply); never run.
    #[serde(default)]
    pub resume_only: bool,
}

/// `PluginHostResponse` payload of a renewal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableRenewalReply {
    pub renewal_id: String,
    /// The collector's output, as for `CollectPortableUsage`.
    pub usage: serde_json::Value,
    /// The credential after the run, when it differs from the restored one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<PortableCredentialBundle>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROK: &str = include_str!("../../../../plugins/grok/collector/refresh-custody.json");

    fn bundle(entry: &serde_json::Value) -> PortableCredentialBundle {
        let document = serde_json::json!({ "https://auth.x.ai::client": entry });
        PortableCredentialBundle {
            portable_schema: "grok-auth-v1".into(),
            method_id: "xai-account".into(),
            values: [(
                "auth_json".to_owned(),
                base64::engine::general_purpose::STANDARD
                    .encode(serde_json::to_vec(&document).unwrap()),
            )]
            .into(),
        }
    }

    fn decoded(bundle: &PortableCredentialBundle) -> serde_json::Value {
        serde_json::from_slice(
            &base64::engine::general_purpose::STANDARD
                .decode(&bundle.values["auth_json"])
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn the_grok_declaration_strips_only_the_refresh_token() {
        let custody = RefreshCustody::parse(GROK).unwrap();
        let full = bundle(&serde_json::json!({
            "auth_mode": "oidc", "key": "access", "user_id": "user",
            "refresh_token": "single-use", "expires_at": "2026-11-21T03:00:00Z",
            "oidc_issuer": "https://auth.x.ai", "oidc_client_id": "client",
        }));
        assert!(custody.holds_refresh_material(&full));
        let stripped = custody.strip(&full).unwrap();
        assert!(!custody.holds_refresh_material(&stripped));
        let entry = &decoded(&stripped)["https://auth.x.ai::client"];
        assert!(entry.get("refresh_token").is_none());
        // The CLI needs these to treat the sign-in as a grok.com account.
        assert_eq!(entry["oidc_issuer"], "https://auth.x.ai");
        assert_eq!(entry["oidc_client_id"], "client");
        assert_eq!(entry["key"], "access");
        // Stripping twice is stable; method and schema never change.
        assert_eq!(custody.strip(&stripped).unwrap(), stripped);
        assert_eq!(stripped.method_id, full.method_id);
        assert_eq!(
            custody.expiry_ms(&full),
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-11-21T03:00:00Z")
                    .unwrap()
                    .timestamp_millis()
            )
        );
        // An "always early" window made the CLI redeem repeatedly in one run.
        assert!(custody.renew_environment.is_empty());
        assert_eq!(custody.credential_path_env, "GROK_AUTH_PATH");
    }

    #[test]
    fn an_unstrippable_bundle_is_refused_and_declarations_are_closed() {
        let custody = RefreshCustody::parse(GROK).unwrap();
        let mut missing = bundle(&serde_json::json!({}));
        missing.values.clear();
        assert!(custody.strip(&missing).is_err());
        let mut array = bundle(&serde_json::json!({}));
        array.values.insert(
            "auth_json".into(),
            base64::engine::general_purpose::STANDARD.encode(b"[1]"),
        );
        assert!(custody.strip(&array).is_err());
        for invalid in [
            r#"{"schema":2,"bundle_key":"a","strip_entry_fields":["r"],"expiry_entry_field":"e","credential_path_env":"A"}"#,
            r#"{"schema":1,"bundle_key":"a","strip_entry_fields":[],"expiry_entry_field":"e","credential_path_env":"A"}"#,
            r#"{"schema":1,"bundle_key":"a","strip_entry_fields":["r"],"expiry_entry_field":"e","credential_path_env":"PATH"}"#,
            r#"{"schema":1,"bundle_key":"a","strip_entry_fields":["r"],"expiry_entry_field":"e","credential_path_env":"lower"}"#,
            r#"{"schema":1,"bundle_key":"a","strip_entry_fields":["a/b"],"expiry_entry_field":"e","credential_path_env":"A"}"#,
            r#"{"schema":1,"bundle_key":"a","strip_entry_fields":["r"],"expiry_entry_field":"e","credential_path_env":"A","extra":1}"#,
        ] {
            assert!(RefreshCustody::parse(invalid).is_err(), "{invalid}");
        }
        let files = std::collections::BTreeMap::new();
        assert_eq!(RefreshCustody::from_files(&files), Ok(None));
    }

    /// No Machine below protocol 51 is ever sent a renewal.
    #[test]
    fn a_renewal_requires_protocol_51() {
        let request: PortableRenewalRequest = serde_json::from_value(serde_json::json!({
            "renewal_id": "renewal-1",
            "usage": {
                "account_id": "11111111-1111-4111-8111-111111111111", "plugin_id": "grok",
                "plugin_version": "3.2.18", "generation_digest": "sha256:x",
                "auth_generation": 3, "payload": {},
            },
        }))
        .unwrap();
        let command = super::super::MachineCommand::RenewPortableCredential {
            request_id: "request-1".into(),
            request: Box::new(request),
        };
        assert_eq!(
            command.minimum_protocol(),
            super::super::REFRESH_CUSTODY_PROTOCOL_VERSION
        );
        assert_eq!(super::super::REFRESH_CUSTODY_PROTOCOL_VERSION, 51);
        const { assert!(super::super::MACHINE_PROTOCOL_VERSION >= 51) };
    }

    /// Only Grok declares custody; every other first-party Provider keeps
    /// today's credential handling.
    #[test]
    fn no_other_first_party_plugin_declares_custody() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        for entry in std::fs::read_dir(&root).unwrap() {
            let plugin = entry.unwrap().path();
            let declared = plugin.join(REFRESH_CUSTODY_DECLARATION).is_file();
            let grok = plugin.file_name().is_some_and(|name| name == "grok");
            assert_eq!(declared, grok, "{}", plugin.display());
        }
    }
}

//! Product authentication configuration over signed, data-only Plugins.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

use crate::oidc::{OidcProvider, OidcProviderRuntimeDocument};
use crate::plugin_catalog::PluginCatalog;

const CONFIG_SCHEMA: &str = "dravengarden.cowboy.authentication/v1";
const MAX_CONFIG_BYTES: u64 = 128 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PasskeyServerPolicy {
    pub enabled: bool,
    pub prompt_after_login: bool,
    pub session_refresh_enabled: bool,
}

#[derive(Clone)]
pub(crate) struct ProductAuthentication {
    pub password_enabled: bool,
    pub passkeys: PasskeyServerPolicy,
    login_method_order: Vec<String>,
    providers: BTreeMap<String, Arc<OidcProvider>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticationDocument {
    schema: String,
    #[serde(default)]
    password: LoginMethodPolicy,
    #[serde(default)]
    passkeys: PasskeyPolicyDocument,
    #[serde(default)]
    login_method_order: Option<Vec<String>>,
    #[serde(default)]
    providers: Vec<AuthenticationProviderSelection>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginMethodPolicy {
    #[serde(default = "default_true")]
    enabled: bool,
}

impl Default for LoginMethodPolicy {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PasskeyPolicyDocument {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_true")]
    prompt_after_login: bool,
    #[serde(default = "default_true")]
    session_refresh_enabled: bool,
}

impl Default for PasskeyPolicyDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            prompt_after_login: true,
            session_refresh_enabled: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthenticationProviderSelection {
    plugin_id: String,
    plugin_version: String,
    artifact_digest: String,
    oidc: OidcProviderRuntimeDocument,
}

impl ProductAuthentication {
    pub(crate) fn disabled() -> Self {
        Self {
            password_enabled: false,
            passkeys: PasskeyServerPolicy {
                enabled: false,
                prompt_after_login: false,
                session_refresh_enabled: false,
            },
            login_method_order: Vec::new(),
            providers: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_default(provider: Option<Arc<OidcProvider>>) -> Self {
        let mut providers = BTreeMap::new();
        if let Some(provider) = provider {
            providers.insert(provider.id().to_owned(), provider);
        }
        let provider_ids = providers.keys().cloned().collect::<Vec<_>>();
        Self {
            password_enabled: true,
            passkeys: PasskeyServerPolicy {
                enabled: true,
                prompt_after_login: true,
                session_refresh_enabled: true,
            },
            login_method_order: resolve_login_method_order(None, true, &provider_ids)
                .expect("default login method order"),
            providers,
        }
    }

    pub(crate) fn load(
        path: Option<&Path>,
        catalog: &PluginCatalog,
        legacy: Option<Arc<OidcProvider>>,
    ) -> Result<Self> {
        let document = path
            .map(read_document)
            .transpose()?
            .unwrap_or(AuthenticationDocument {
                schema: CONFIG_SCHEMA.to_owned(),
                password: LoginMethodPolicy::default(),
                passkeys: PasskeyPolicyDocument::default(),
                login_method_order: None,
                providers: Vec::new(),
            });
        ensure!(
            document.schema == CONFIG_SCHEMA,
            "unsupported authentication config"
        );
        ensure!(
            document.providers.len() <= 16,
            "too many Authentication Providers"
        );
        let mut providers = BTreeMap::new();
        for selected in document.providers {
            let contract = catalog.resolve_authentication_provider(
                &selected.plugin_id,
                &selected.plugin_version,
                &selected.artifact_digest,
            )?;
            ensure!(
                contract.id == selected.plugin_id && contract.version == selected.plugin_version,
                "Authentication Provider identity mismatch"
            );
            let provider = Arc::new(OidcProvider::load_plugin(&contract, selected.oidc)?);
            ensure!(
                providers
                    .insert(provider.id().to_owned(), provider)
                    .is_none(),
                "duplicate Authentication Provider"
            );
        }
        if let Some(provider) = legacy {
            providers
                .entry(provider.id().to_owned())
                .or_insert(provider);
        }
        ensure!(
            document.password.enabled || !providers.is_empty(),
            "product authentication requires at least one login method"
        );
        ensure!(
            document.passkeys.enabled || !document.passkeys.prompt_after_login,
            "Passkey setup prompting requires Passkeys to be enabled"
        );
        ensure!(
            document.passkeys.enabled || !document.passkeys.session_refresh_enabled,
            "Passkey session refresh requires Passkeys to be enabled"
        );
        let provider_ids = providers.keys().cloned().collect::<Vec<_>>();
        let login_method_order = resolve_login_method_order(
            document.login_method_order,
            document.password.enabled,
            &provider_ids,
        )?;
        Ok(Self {
            password_enabled: document.password.enabled,
            passkeys: PasskeyServerPolicy {
                enabled: document.passkeys.enabled,
                prompt_after_login: document.passkeys.prompt_after_login,
                session_refresh_enabled: document.passkeys.session_refresh_enabled,
            },
            login_method_order,
            providers,
        })
    }

    pub(crate) fn provider(&self, id: &str) -> Option<&Arc<OidcProvider>> {
        self.providers.get(id)
    }

    pub(crate) fn public_providers(&self) -> Vec<crate::oidc::PublicProvider> {
        self.providers
            .values()
            .map(|provider| provider.public())
            .collect()
    }

    pub(crate) fn login_method_order(&self) -> &[String] {
        &self.login_method_order
    }
}

fn resolve_login_method_order(
    configured: Option<Vec<String>>,
    password_enabled: bool,
    provider_ids: &[String],
) -> Result<Vec<String>> {
    ensure!(
        provider_ids.iter().all(|id| id != "password"),
        "Authentication Provider ID password is reserved"
    );
    let mut default_order = Vec::with_capacity(provider_ids.len() + usize::from(password_enabled));
    if provider_ids.iter().any(|id| id == "cardea") {
        default_order.push("cardea".to_owned());
    }
    if password_enabled {
        default_order.push("password".to_owned());
    }
    default_order.extend(
        provider_ids
            .iter()
            .filter(|id| id.as_str() != "cardea")
            .cloned(),
    );

    let Some(configured) = configured else {
        return Ok(default_order);
    };
    let configured_set = configured.iter().cloned().collect::<BTreeSet<_>>();
    ensure!(
        configured_set.len() == configured.len(),
        "login method order contains a duplicate"
    );
    let available_set = default_order.iter().cloned().collect::<BTreeSet<_>>();
    ensure!(
        configured_set == available_set,
        "login method order must contain every enabled method exactly once"
    );
    Ok(configured)
}

fn default_true() -> bool {
    true
}

fn read_document(path: &Path) -> Result<AuthenticationDocument> {
    ensure!(
        path.is_absolute(),
        "authentication config path must be absolute"
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("opening authentication config {}", path.display()))?;
    let metadata = file
        .metadata()
        .context("inspecting authentication config")?;
    ensure!(
        metadata.is_file(),
        "authentication config must be a regular file"
    );
    ensure!(
        metadata.mode() & 0o077 == 0,
        "authentication config permissions are too broad"
    );
    ensure!(
        metadata.len() <= MAX_CONFIG_BYTES,
        "authentication config is too large"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CONFIG_BYTES,
        "authentication config is too large"
    );
    serde_json::from_slice(&bytes).context("decoding authentication config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_keeps_password_and_passkeys_available() {
        let password = LoginMethodPolicy::default();
        let passkeys = PasskeyPolicyDocument::default();
        assert!(password.enabled);
        assert!(passkeys.enabled);
        assert!(passkeys.prompt_after_login);
        assert!(passkeys.session_refresh_enabled);
    }

    #[test]
    fn default_login_order_prefers_cardea_without_reordering_other_servers() {
        assert_eq!(
            resolve_login_method_order(None, true, &["cardea".to_owned(), "google".to_owned()],)
                .unwrap(),
            ["cardea", "password", "google"]
        );
        assert_eq!(
            resolve_login_method_order(None, true, &["google".to_owned()]).unwrap(),
            ["password", "google"]
        );
    }

    #[test]
    fn configured_login_order_is_an_exact_permutation() {
        let providers = ["cardea".to_owned(), "google".to_owned()];
        assert_eq!(
            resolve_login_method_order(
                Some(vec![
                    "google".to_owned(),
                    "cardea".to_owned(),
                    "password".to_owned(),
                ]),
                true,
                &providers,
            )
            .unwrap(),
            ["google", "cardea", "password"]
        );
        assert!(
            resolve_login_method_order(
                Some(vec!["cardea".to_owned(), "cardea".to_owned()]),
                false,
                &providers,
            )
            .unwrap_err()
            .to_string()
            .contains("duplicate")
        );
        assert!(
            resolve_login_method_order(
                Some(vec!["cardea".to_owned(), "password".to_owned()]),
                true,
                &providers,
            )
            .unwrap_err()
            .to_string()
            .contains("every enabled method")
        );
        assert!(
            resolve_login_method_order(None, true, &["password".to_owned()])
                .unwrap_err()
                .to_string()
                .contains("reserved")
        );
    }
}

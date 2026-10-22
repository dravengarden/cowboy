//! Stable credential identities. Display names, email addresses, subscription
//! tiers and authentication generations are deliberately not account identity.

use serde::{Deserialize, Serialize};

#[cfg(any(feature = "full", feature = "machine-host", test))]
pub(crate) mod portable_contract;

/// Whether a native login made under `signed_in` may be used, in place, by the
/// `target` release. The authentication contract fingerprint alone is not
/// enough: it deliberately excludes native identity recipes. Require the whole
/// signed authentication section (methods, identity recipes, credential
/// layout) to be identical as well, so no login, home or credential changes.
#[cfg(any(feature = "full", feature = "machine-host", test))]
pub(crate) fn native_release_compatible(
    signed_in: &cowboy_provider_sdk::ProviderManifest,
    target: &cowboy_provider_sdk::ProviderManifest,
) -> bool {
    signed_in.id == target.id
        && signed_in.compatibility.auth_contract_fingerprint
            == target.compatibility.auth_contract_fingerprint
        && serde_json::to_value(&signed_in.authentication).ok()
            == serde_json::to_value(&target.authentication).ok()
}

/// One bounded native authorization response. Validate at every ingress, not
/// just the CLI: embedded newlines would become additional native commands.
#[cfg(any(feature = "full", feature = "machine-host", test))]
pub(crate) fn login_code(value: &str) -> Result<&str, &'static str> {
    if value.len() > 4096 {
        return Err("native login code exceeds 4096 bytes");
    }
    let value = value.trim();
    if value.is_empty() || value.contains(['\r', '\n', '\0']) {
        return Err("expected one non-empty native login code");
    }
    Ok(value)
}

/// Native identity reported by a signed Provider after a fresh isolated login.
/// Display labels and credentials are deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeIdentity {
    pub authority: String,
    pub subject: String,
    pub tenant: Option<String>,
}

/// Machine-local native login completion evidence. This is metadata only,
/// never a credential export or authorization to launch a session. Controller
/// admission must additionally bind the authenticated Machine and login claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProfileReceipt {
    pub account_id: ProviderAccountId,
    pub provider_id: String,
    pub request_id: String,
    pub generation: u64,
    pub method: String,
    pub provider_version: String,
    pub generation_digest: String,
    pub auth_contract_fingerprint: String,
    pub identity: NativeIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLoginSpec {
    pub generation: u64,
    pub method: String,
    pub provider_version: String,
    pub generation_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLogoutRequest {
    pub account_id: ProviderAccountId,
    pub provider_id: String,
    pub generation: u64,
    pub login_request_id: String,
    pub operation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginStopRequest {
    pub account_id: ProviderAccountId,
    pub provider_id: String,
    pub login_request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExitRequest {
    pub account_id: ProviderAccountId,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_command_id: Option<String>,
}

#[cfg(any(feature = "full", feature = "machine-host"))]
pub(crate) fn account_stop_command_id(
    operation: &str,
    session: &str,
    account: &ProviderAccountId,
) -> String {
    use sha2::{Digest as _, Sha256};
    let scope =
        serde_json::to_vec(&(operation, session, account)).expect("string tuple serializes");
    format!("account-stop-{:x}", Sha256::digest(scope))
}

/// Returned only while a durable account-stop fence covers this launch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExitObservation {
    pub request: WorkerExitRequest,
    pub launch_id: uuid::Uuid,
}

/// Migration preparation evidence, never permission to launch a worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedBinding {
    pub operation: String,
    pub stopped: WorkerExitObservation,
    pub target: NativeProfileReceipt,
}

/// Historical local cleanup and optional tree evidence, never account revocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // Preserve independent, versioned historical evidence.
pub struct LoginStopObservation {
    pub request: LoginStopRequest,
    pub process_started: bool,
    pub leader_exited: bool,
    pub candidate_removed: bool,
    #[serde(default)]
    pub process_tree_exited: bool,
}

/// Native command outcome only; not whole-account logout or remote token revocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeLogoutObservation {
    pub request: NativeLogoutRequest,
    pub succeeded: Option<bool>,
    /// Durable generation tombstone and confirmed removal of its private home.
    #[serde(default)]
    pub profile_cleaned: bool,
    /// Both native trees exited and the native logout outcome is known. Local
    /// crash recovery alone must not manufacture this completion evidence.
    #[serde(default)]
    pub process_trees_exited: bool,
}

/// Prepare one private directory without following a symlink for either the
/// directory or its immediate parent. Call in parent-first order for a scope.
/// Permissions are applied to the opened directory, never to a link target.
/// The service-owned ancestors remain a trusted filesystem boundary; this is
/// not a sandbox against a process with the same UID or root privileges.
#[cfg(any(feature = "full", feature = "machine-host", test))]
pub(crate) fn prepare_private_directory(path: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use rustix::fs::{Mode, OFlags, fchmod, mkdirat, open, openat};

    let parent = path.parent().context("Provider directory has no parent")?;
    let name = path.file_name().context("Provider directory has no name")?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let parent = open(parent, flags, Mode::empty())
        .with_context(|| format!("opening Provider directory parent {}", path.display()))?;
    match mkdirat(&parent, name, Mode::RWXU) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    let directory = openat(&parent, name, flags, Mode::empty())
        .with_context(|| format!("opening private Provider directory {}", path.display()))?;
    fchmod(&directory, Mode::RWXU)?;
    Ok(())
}

/// A controller-issued identity, also safe as a single filesystem component.
/// `default` names the existing single-account lineage during migration.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProviderAccountId(String);

/// Transitional first-party credential boundary. Named Claude subscriptions
/// need native-owned login/status/logout, not the portable-vault prototype.
/// Remove this admission fence only when that distinct lifecycle is accepted;
/// a new identity recipe or protocol version alone must never open export.
/// Legacy default accounts retain their existing migration boundary.
#[cfg(any(feature = "full", feature = "machine-host", test))]
pub(crate) fn portable_credentials_allowed(
    account: &ProviderAccountId,
    provider: &str,
) -> Result<(), &'static str> {
    if !account.is_default() && provider == "claude-code" {
        Err("native_owned_credentials_required")
    } else {
        Ok(())
    }
}

impl Default for ProviderAccountId {
    fn default() -> Self {
        Self("default".into())
    }
}

impl ProviderAccountId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_default(&self) -> bool {
        self.0 == "default"
    }
}

impl TryFrom<String> for ProviderAccountId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "default"
            || (value.len() == 36
                && value.bytes().enumerate().all(|(index, byte)| {
                    if [8, 13, 18, 23].contains(&index) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                }))
        {
            Ok(Self(value))
        } else {
            Err("invalid Provider account identity")
        }
    }
}

impl From<ProviderAccountId> for String {
    fn from(value: ProviderAccountId) -> Self {
        value.0
    }
}

impl std::str::FromStr for ProviderAccountId {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_follows_only_releases_with_identical_signed_authentication() {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../plugins/claude-code/provider.json")).unwrap();
        let signed_in = source.compile().unwrap();
        let mut next = signed_in.clone();
        next.version = "999.0.0".into();
        next.runtime = signed_in.runtime.clone();
        assert!(native_release_compatible(&signed_in, &next));
        // The fingerprint excludes native identity recipes; any change to the
        // signed authentication section (methods, recipes, layout) refuses.
        let mut recipe = next.clone();
        recipe.authentication.methods.pop();
        assert!(!native_release_compatible(&signed_in, &recipe));
        let mut contract = next.clone();
        contract.compatibility.auth_contract_fingerprint = "sha256:other".into();
        assert!(!native_release_compatible(&signed_in, &contract));
        let mut provider = next;
        provider.id = "claude-deepseek".into();
        assert!(!native_release_compatible(&signed_in, &provider));
    }

    #[test]
    fn authorization_input_is_one_bounded_line() {
        assert_eq!(login_code(" code#state\n"), Ok("code#state"));
        for input in [
            "".into(),
            "secret\ncommand".into(),
            "secret\0".into(),
            "s".repeat(4097),
        ] {
            let error = login_code(&input).unwrap_err();
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn native_owned_credentials_are_separate_from_legacy_and_api_accounts() {
        let named = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert_eq!(
            portable_credentials_allowed(&named, "claude-code"),
            Err("native_owned_credentials_required")
        );
        assert!(portable_credentials_allowed(&ProviderAccountId::default(), "claude-code").is_ok());
        assert!(portable_credentials_allowed(&named, "claude-deepseek").is_ok());
    }

    #[test]
    fn private_scope_rejects_links_without_touching_the_target() {
        use std::fs;
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(target.join("sentinel"), "unchanged").unwrap();
        let alias = root.path().join("alias");
        symlink(&target, &alias).unwrap();
        assert!(prepare_private_directory(&alias).is_err());
        assert!(prepare_private_directory(&alias.join("child")).is_err());
        assert!(!target.join("child").exists());
        assert_eq!(
            fs::read_to_string(target.join("sentinel")).unwrap(),
            "unchanged"
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );

        let private = root.path().join("private");
        prepare_private_directory(&private).unwrap();
        prepare_private_directory(&private).unwrap();
        assert_eq!(
            fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let file = root.path().join("file");
        fs::write(&file, "unchanged").unwrap();
        assert!(prepare_private_directory(&file).is_err());
        let dangling = root.path().join("dangling");
        symlink(root.path().join("missing"), &dangling).unwrap();
        assert!(prepare_private_directory(&dangling).is_err());
        assert!(!root.path().join("missing").exists());
    }

    #[test]
    fn account_identity_cannot_alias_another_directory() {
        for value in [
            "",
            ".",
            "..",
            "../default",
            "DEFAULT",
            "default/",
            "a\\b",
            "a\0b",
        ] {
            assert!(ProviderAccountId::try_from(value.to_owned()).is_err());
            assert!(serde_json::from_value::<ProviderAccountId>(serde_json::json!(value)).is_err());
        }
        let id = "f21e9462-2cac-4cba-8f11-1a1ed3996ca9";
        assert_eq!(
            ProviderAccountId::try_from(id.to_owned()).unwrap().as_str(),
            id
        );
        assert!(ProviderAccountId::default().is_default());
    }
}

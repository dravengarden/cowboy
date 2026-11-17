//! Native subscription recipes: the signed contract that lets a Provider CLI
//! log in inside a per-account home and lets the Machine verify the identity
//! the CLI wrote there.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    AuthExecutor, AuthMethod, AuthenticationContract, validate_id, validate_relative_path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeLogoutRecipe {
    CommandV1 { arguments: Vec<String> },
}

/// Identity written by the native CLI into a newly created login home. Never
/// read this recipe against an existing session or host configuration directory.
///
/// A path component [`NATIVE_IDENTITY_SOLE_MEMBER`] selects the only member
/// of an object, for CLIs that key their login by issuer. With
/// `subject_claim`, the subject path names a compact JWT written by the CLI
/// and the subject is that claim of its payload. The token is read from the
/// same fresh login file as any other field; it is not a trust anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeIdentityRecipe {
    FreshLoginJsonV1 {
        authority: String,
        relative_path: String,
        subject_path: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tenant_path: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject_claim: Option<Vec<String>>,
    },
    /// The owner names the subscription when adding the account; the native
    /// login writes no identity Cowboy may read without opening credentials.
    /// The subject is the Cowboy account ID, so the binding proves only which
    /// account card authorized the login, never which vendor account it was.
    OwnerDeclaredV1 { authority: String },
}

/// Path component selecting the sole member of a JSON object.
pub const NATIVE_IDENTITY_SOLE_MEMBER: &str = "*";

impl NativeIdentityRecipe {
    /// Whether the recipe uses identity paths newer than Provider SDK 3.1.15.
    #[must_use]
    pub fn uses_extended_paths(&self) -> bool {
        match self {
            Self::FreshLoginJsonV1 {
                subject_path,
                tenant_path,
                subject_claim,
                ..
            } => {
                subject_claim.is_some()
                    || subject_path
                        .iter()
                        .chain(tenant_path.iter().flatten())
                        .any(|part| part == NATIVE_IDENTITY_SOLE_MEMBER)
            }
            Self::OwnerDeclaredV1 { .. } => false,
        }
    }

    /// Whether the recipe is newer than Provider SDK 3.3.0.
    #[must_use]
    pub fn is_owner_declared(&self) -> bool {
        matches!(self, Self::OwnerDeclaredV1 { .. })
    }

    /// The authority every identity produced by this recipe carries.
    #[must_use]
    pub fn authority(&self) -> &str {
        match self {
            Self::FreshLoginJsonV1 { authority, .. } | Self::OwnerDeclaredV1 { authority } => {
                authority
            }
        }
    }

    /// Whether identities produced by this recipe carry a tenant.
    #[must_use]
    pub fn has_tenant(&self) -> bool {
        matches!(
            self,
            Self::FreshLoginJsonV1 {
                tenant_path: Some(_),
                ..
            }
        )
    }
}

impl AuthenticationContract {
    /// Named subscription accounts are native-owned: the Provider CLI logs in
    /// inside a per-account home and Cowboy verifies the identity it wrote.
    /// A Provider supports them exactly when it signs such a login method.
    #[must_use]
    pub fn native_accounts(&self) -> bool {
        self.methods
            .iter()
            .any(|method| method.native_identity.is_some())
    }
}

/// Validate one method's native logout and identity recipes.
pub(crate) fn validate_method(method: &AuthMethod) -> Result<()> {
    if let Some(NativeLogoutRecipe::CommandV1 { arguments }) = &method.native_logout {
        ensure!(
            method.native_identity.is_some()
                && matches!(method.executor, AuthExecutor::CommandV1 { .. }),
            "native logout requires native identity and command login"
        );
        ensure!(
            !arguments.is_empty()
                && arguments.len() <= 32
                && arguments.iter().all(|argument| !argument.is_empty()
                    && argument.len() <= 1024
                    && !argument.chars().any(char::is_control)),
            "invalid native logout arguments"
        );
    }
    if let Some(NativeIdentityRecipe::OwnerDeclaredV1 { authority }) = &method.native_identity {
        ensure!(
            matches!(method.executor, AuthExecutor::CommandV1 { .. }),
            "native identity requires a native login command"
        );
        validate_id(authority, "native identity authority")?;
    }
    if let Some(NativeIdentityRecipe::FreshLoginJsonV1 {
        authority,
        relative_path,
        subject_path,
        tenant_path,
        subject_claim,
    }) = &method.native_identity
    {
        ensure!(
            matches!(method.executor, AuthExecutor::CommandV1 { .. }),
            "native identity requires a native login command"
        );
        validate_id(authority, "native identity authority")?;
        ensure!(
            !relative_path.is_empty() && relative_path.len() <= 512,
            "invalid native identity file"
        );
        validate_relative_path(relative_path)?;
        for path in std::iter::once(subject_path)
            .chain(tenant_path.iter())
            .chain(subject_claim.iter())
        {
            ensure!(
                !path.is_empty()
                    && path.len() <= 16
                    && path.iter().all(|part| !part.is_empty()
                        && part.len() <= 128
                        && !part.chars().any(char::is_control)),
                "invalid native identity JSON path"
            );
        }
    }
    Ok(())
}

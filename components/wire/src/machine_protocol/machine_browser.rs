//! Machine browser lifecycle and display-stream binding (Machine protocol 45).
//!
//! A browser instance belongs to one Provider account on one Machine. The
//! Controller drives it with the closed command set below and learns about it
//! through `Event::State`; the display itself travels on a dedicated
//! WebSocket bound by a one-time token, never on the command channel or the
//! bulk lanes. Payloads carry identities, geometry and reasons only: no URL
//! other than an owner-supplied `Tab` navigation, no credential, no pixels.
//! See `docs/machine-browser.md`.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::provider_account::ProviderAccountId;

/// Dedicated WebSocket the Machine opens for one attached viewer.
pub const STREAM_PATH: &str = "/api/machine/browser-stream";
/// One-time token minted by the Controller for exactly one attachment,
/// presented on the stream upgrade. It is never an owner credential.
pub const STREAM_TOKEN_HEADER: &str = "x-cowboy-browser-stream-token";
/// Largest binary frame relayed in either direction.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// A stream token not presented within this window is void.
pub const STREAM_TOKEN_TTL_MS: i64 = 30_000;
/// A viewer upgrade not paired with its Machine stream within this window is
/// closed.
pub const PAIRING_TIMEOUT_MS: i64 = 15_000;
/// The viewer renews its attachment lease this often.
pub const LEASE_RENEWAL_MS: i64 = 10_000;
/// An attachment whose lease is not renewed for this long is detached,
/// whatever the RFB traffic says.
pub const LEASE_EXPIRY_MS: i64 = 30_000;

pub const MIN_WIDTH: u32 = 320;
pub const MIN_HEIGHT: u32 = 480;
pub const MAX_DIMENSION: u32 = 4096;
const MAX_URL_BYTES: usize = 2048;
const MAX_ID_BYTES: usize = 128;

/// Framebuffer size in device pixels and the browser's device scale factor.
/// `scale` is fixed for the life of an incarnation because it is a launch
/// flag; the viewer owns every later resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub scale: u8,
}

impl Viewport {
    /// # Errors
    /// Returns when the geometry is outside the supported range.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (MIN_WIDTH..=MAX_DIMENSION).contains(&self.width)
                && (MIN_HEIGHT..=MAX_DIMENSION).contains(&self.height),
            "browser viewport outside the supported range"
        );
        ensure!(
            matches!(self.scale, 1 | 2),
            "browser device scale factor must be 1 or 2"
        );
        Ok(())
    }
}

/// Why an instance exists. A sign-in instance follows its native login's
/// authority: it is bound to that login's account under the login lock and
/// hibernates when the login ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Purpose {
    SignIn { login_request_id: String },
    Tab,
}

/// Exact instance identity. Every mutation names the incarnation it expects
/// so a delayed command for a replaced instance is a recorded no-op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceRef {
    pub account_id: ProviderAccountId,
    pub instance_id: String,
    pub incarnation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Open {
        account_id: ProviderAccountId,
        instance_id: String,
        purpose: Purpose,
        viewport: Viewport,
    },
    Attach {
        instance: InstanceRef,
        attachment_id: String,
        stream_token: String,
        viewport: Viewport,
    },
    Lease {
        instance: InstanceRef,
        attachment_id: String,
        expires_at_ms: i64,
    },
    Detach {
        instance: InstanceRef,
        attachment_id: String,
    },
    Resize {
        instance: InstanceRef,
        attachment_id: String,
        width: u32,
        height: u32,
    },
    /// `Tab` instances only; sign-in instances navigate to the URL the
    /// Machine itself emitted for the login.
    Navigate {
        instance: InstanceRef,
        url: String,
    },
    Snapshot {
        instance: InstanceRef,
    },
    Hibernate {
        instance: InstanceRef,
    },
    Reset {
        account_id: ProviderAccountId,
        expected_profile_revision: u64,
    },
    /// Account revocation: unconditional, idempotent by `operation_id`;
    /// deletes the profile like `Reset` while the control record survives.
    Revoke {
        account_id: ProviderAccountId,
        operation_id: String,
    },
    /// Current state of the account's browser, running or hibernated.
    Query {
        account_id: ProviderAccountId,
    },
}

impl Command {
    /// Bounds every field at the wire boundary; the Machine validates again.
    ///
    /// # Errors
    /// Returns when an identifier, geometry or URL is outside its bounds.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Open {
                account_id,
                instance_id,
                purpose,
                viewport,
            } => {
                ensure!(!account_id.is_default(), "browser requires a named account");
                validate_id(instance_id, "browser instance id")?;
                if let Purpose::SignIn { login_request_id } = purpose {
                    validate_id(login_request_id, "browser login request id")?;
                }
                viewport.validate()
            }
            Self::Attach {
                instance,
                attachment_id,
                stream_token,
                viewport,
            } => {
                validate_instance(instance)?;
                validate_id(attachment_id, "browser attachment id")?;
                ensure!(
                    (16..=256).contains(&stream_token.len())
                        && stream_token.bytes().all(|byte| byte.is_ascii_alphanumeric()
                            || byte == b'-'
                            || byte == b'_'),
                    "invalid browser stream token"
                );
                viewport.validate()
            }
            Self::Lease {
                instance,
                attachment_id,
                expires_at_ms,
            } => {
                validate_instance(instance)?;
                validate_id(attachment_id, "browser attachment id")?;
                ensure!(*expires_at_ms > 0, "invalid browser lease expiry");
                Ok(())
            }
            Self::Detach {
                instance,
                attachment_id,
            } => {
                validate_instance(instance)?;
                validate_id(attachment_id, "browser attachment id")
            }
            Self::Resize {
                instance,
                attachment_id,
                width,
                height,
            } => {
                validate_instance(instance)?;
                validate_id(attachment_id, "browser attachment id")?;
                Viewport {
                    width: *width,
                    height: *height,
                    scale: 1,
                }
                .validate()
            }
            Self::Navigate { instance, url } => {
                validate_instance(instance)?;
                validate_url(url)
            }
            Self::Snapshot { instance } | Self::Hibernate { instance } => {
                validate_instance(instance)
            }
            Self::Reset { account_id, .. } | Self::Query { account_id } => {
                ensure!(!account_id.is_default(), "browser requires a named account");
                Ok(())
            }
            Self::Revoke {
                account_id,
                operation_id,
            } => {
                ensure!(!account_id.is_default(), "browser requires a named account");
                validate_id(operation_id, "browser revocation operation id")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// No process; the published snapshot is the only state.
    Hibernated,
    Starting,
    /// Running without a live attachment; the grace timer is counting.
    Detached,
    Attached,
    Stopping,
}

/// Closed reason vocabulary. Unknown values from a newer peer decode as
/// `Unknown` so an older Controller still renders the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    NotAvailable,
    HostRequirements,
    SandboxUnavailable,
    ConfinementUnverified,
    ListenerDetected,
    StorageExceeded,
    ProfileNewerThanRuntime,
    Superseded,
    LoginEnded,
    /// The named login is not this account's active native login.
    LoginMismatch,
    Deadline,
    Grace,
    Reclaim,
    Revoked,
    Disabled,
    OwnerRequest,
    /// `browser.max_instances` running instances already.
    Capacity,
    /// No memory headroom for the browser's reservation.
    Memory,
    /// A lifecycle operation on this account is still in progress.
    Busy,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
}

/// One instance observation. Every field beyond the identity is optional so
/// a hibernated instance and a running one share the shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateEvent {
    /// The command this observation answers, when it answers one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub instance: InstanceRef,
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<Geometry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_at_ms: Option<i64>,
    /// The account's profile revision; `Reset` names the one it expects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_revision: Option<u64>,
    #[serde(default)]
    pub unsaved_state_lost: bool,
    pub observed_at_ms: i64,
}

/// Whether this Machine can run browsers at all, and with which installed
/// Plugin generation. Sent in the hello and again whenever it changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    pub max_instances: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    State(StateEvent),
    Capability(Capability),
    /// The Machine has no browser capability any more (Plugin uninstalled or
    /// host requirements lost); running instances were stopped.
    CapabilityLost,
}

fn validate_instance(instance: &InstanceRef) -> Result<()> {
    ensure!(
        !instance.account_id.is_default(),
        "browser requires a named account"
    );
    validate_id(&instance.instance_id, "browser instance id")
}

fn validate_id(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= MAX_ID_BYTES
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        "invalid {label}"
    );
    Ok(())
}

/// Owner-supplied navigation for a `Tab`: HTTPS, bounded, no control bytes.
///
/// # Errors
/// Returns when the URL is not a plain HTTPS URL within bounds.
pub fn validate_url(url: &str) -> Result<()> {
    ensure!(
        url.len() <= MAX_URL_BYTES
            && url.starts_with("https://")
            && url.len() > "https://".len()
            && !url
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control()),
        "browser navigation must be a bounded https URL"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> ProviderAccountId {
        ProviderAccountId::try_from("11111111-2222-4333-8444-555555555555".to_owned()).unwrap()
    }

    fn instance() -> InstanceRef {
        InstanceRef {
            account_id: account(),
            instance_id: "inst-1".to_owned(),
            incarnation: 3,
        }
    }

    #[test]
    fn commands_bound_geometry_identity_and_urls() {
        let open = Command::Open {
            account_id: account(),
            instance_id: "inst-1".to_owned(),
            purpose: Purpose::SignIn {
                login_request_id: "login-1".to_owned(),
            },
            viewport: Viewport {
                width: 780,
                height: 1688,
                scale: 2,
            },
        };
        open.validate().unwrap();
        let huge = Command::Resize {
            instance: instance(),
            attachment_id: "att-1".to_owned(),
            width: 5000,
            height: 800,
        };
        assert!(huge.validate().is_err());
        let scale = Command::Open {
            account_id: account(),
            instance_id: "inst-1".to_owned(),
            purpose: Purpose::Tab,
            viewport: Viewport {
                width: 1280,
                height: 800,
                scale: 3,
            },
        };
        assert!(scale.validate().is_err());
        let default_account = Command::Reset {
            account_id: ProviderAccountId::default(),
            expected_profile_revision: 1,
        };
        assert!(default_account.validate().is_err());
        assert!(
            Command::Revoke {
                account_id: account(),
                operation_id: "op/1".to_owned(),
            }
            .validate()
            .is_err()
        );
        Command::Query {
            account_id: account(),
        }
        .validate()
        .unwrap();
        assert!(
            Command::Navigate {
                instance: instance(),
                url: "http://example.com".to_owned(),
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::Navigate {
                instance: instance(),
                url: "https://example.com/a b".to_owned(),
            }
            .validate()
            .is_err()
        );
        Command::Navigate {
            instance: instance(),
            url: "https://example.com/path?q=1".to_owned(),
        }
        .validate()
        .unwrap();
        assert!(
            Command::Attach {
                instance: instance(),
                attachment_id: "att-1".to_owned(),
                stream_token: "short".to_owned(),
                viewport: Viewport {
                    width: 1280,
                    height: 800,
                    scale: 1,
                },
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn unknown_reasons_decode_for_older_controllers() {
        let event: StateEvent = serde_json::from_str(
            r#"{"instance":{"account_id":"11111111-2222-4333-8444-555555555555","instance_id":"inst-1","incarnation":1},
                "state":"detached","reason":"something_newer","observed_at_ms":1}"#,
        )
        .unwrap();
        assert_eq!(event.reason, Some(Reason::Unknown));
        assert!(!event.unsaved_state_lost);
        let json = serde_json::to_string(&Event::CapabilityLost).unwrap();
        assert_eq!(json, r#"{"kind":"capability_lost"}"#);
    }
}

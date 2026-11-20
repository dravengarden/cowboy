//! `cowboy-account-sandbox-v1`: newline-delimited JSON over the daemon's Unix
//! socket. A connection carries sequential requests; each gets one response
//! line. `register_runtime` is followed by exactly `size` raw archive bytes,
//! and a successful `start` response carries the unit's stdin, stdout and
//! stderr pipe ends (and, for `runtime`, the listening execution socket) as
//! `SCM_RIGHTS` on the response line. Requests are strict: unknown fields,
//! oversize lines and malformed ids are refused.

use std::fmt;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids;

pub const PROTOCOL: &str = "cowboy-account-sandbox-v1";
/// Longest request line.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// Longest decoded per-turn execution descriptor.
pub const MAX_DESCRIPTOR_BYTES: usize = 32 * 1024;

/// What an account unit runs; the command lines are fixed in [`crate::units`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Runtime,
    Login,
    Logout,
    Probe,
    /// The account's quota report (`agy -p /usage`); started only by
    /// `collect_usage`, never by `start`.
    Usage,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Login => "login",
            Self::Logout => "logout",
            Self::Probe => "probe",
            Self::Usage => "usage",
        }
    }

    /// `login` and `logout` share the account's single authentication slot.
    #[must_use]
    pub fn is_auth(self) -> bool {
        matches!(self, Self::Login | Self::Logout)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    EnsureAccount {
        service_id: String,
        account_id: String,
    },
    RegisterRuntime {
        sha256: String,
        size: u64,
    },
    /// Run a registered runtime's fixed probe command in a unit of the
    /// daemon-owned probe identity (never a real account).
    ProbeRuntime {
        runtime_sha256: String,
    },
    Start {
        key: String,
        mode: Mode,
        credential_generation: u64,
        runtime_sha256: String,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        descriptor_b64: Option<String>,
    },
    /// Run the account's fixed, read-only quota report in a `usage` unit on
    /// its current signed-in generation and answer only the validated
    /// projection of its output (`crate::usage`).
    CollectUsage {
        key: String,
        credential_generation: u64,
        runtime_sha256: String,
    },
    StopUnit {
        unit: String,
    },
    StopAccount {
        key: String,
        /// The Controller's account-control revision, with `operation_id`.
        #[serde(default)]
        revision: Option<u64>,
        #[serde(default)]
        operation_id: Option<String>,
    },
    AdmitAccount {
        key: String,
        #[serde(default)]
        revision: Option<u64>,
        #[serde(default)]
        operation_id: Option<String>,
    },
    ListUnits {
        key: String,
    },
    UnitState {
        unit: String,
    },
    LogoutCleanup {
        key: String,
        /// The credential generation whose logout this cleans up.
        credential_generation: u64,
        #[serde(default)]
        operation_id: Option<String>,
    },
    RemoveAccount {
        key: String,
        /// The caller's idempotent operation; a retry of the completed
        /// removal is answered as its replay.
        #[serde(default)]
        operation_id: Option<String>,
    },
}

/// Failure classes of a response; fixed so callers map them exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    InvalidRequest,
    NotFound,
    Fenced,
    Conflict,
    Limit,
    Precondition,
    InsufficientSpace,
    RuntimeRejected,
    Internal,
}

#[derive(Debug)]
pub struct Failure {
    pub code: Code,
    pub message: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            code: Code::Internal,
            message: format!("{error:#}"),
        }
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::from(anyhow::Error::from(error))
    }
}

impl From<rustix::io::Errno> for Failure {
    fn from(error: rustix::io::Errno) -> Self {
        Self::from(anyhow::Error::from(error))
    }
}

/// A failure of `code`.
pub fn fail(code: Code, message: impl Into<String>) -> Failure {
    Failure {
        code,
        message: message.into(),
    }
}

pub type Outcome<T> = std::result::Result<T, Failure>;

/// Parse and validate one request line (without its newline).
pub fn parse_request(line: &[u8]) -> Outcome<Request> {
    if line.len() > MAX_LINE_BYTES {
        return Err(fail(Code::InvalidRequest, "request line too long"));
    }
    let request: Request = serde_json::from_slice(line)
        .map_err(|error| fail(Code::InvalidRequest, format!("malformed request: {error}")))?;
    validate(&request).map_err(|error| fail(Code::InvalidRequest, format!("{error:#}")))?;
    Ok(request)
}

fn validate(request: &Request) -> Result<()> {
    match request {
        Request::EnsureAccount {
            service_id,
            account_id,
        } => {
            ids::validate_id("service_id", service_id)?;
            anyhow::ensure!(
                service_id != crate::probe::PROBE_SERVICE,
                "service_id {service_id:?} is reserved"
            );
            ids::validate_id("account_id", account_id)
        }
        Request::ProbeRuntime { runtime_sha256 } => {
            ids::validate_hex("runtime_sha256", runtime_sha256, 64)
        }
        Request::RegisterRuntime { sha256, size } => {
            ids::validate_hex("sha256", sha256, 64)?;
            anyhow::ensure!(*size > 0, "size must be positive");
            Ok(())
        }
        Request::Start {
            key,
            mode,
            runtime_sha256,
            session_id,
            descriptor_b64,
            ..
        } => {
            anyhow::ensure!(
                *mode != Mode::Usage,
                "usage units start only through collect_usage"
            );
            ids::validate_key(key)?;
            ids::validate_hex("runtime_sha256", runtime_sha256, 64)?;
            if let Some(session_id) = session_id {
                ids::validate_id("session_id", session_id)?;
            }
            if let Some(descriptor) = descriptor_b64 {
                anyhow::ensure!(
                    descriptor.len() <= MAX_DESCRIPTOR_BYTES.div_ceil(3) * 4,
                    "descriptor exceeds {MAX_DESCRIPTOR_BYTES} bytes"
                );
                decode_base64(descriptor)?;
            }
            Ok(())
        }
        Request::StopUnit { unit } | Request::UnitState { unit } => {
            validate_unit_name(unit).map(|_| ())
        }
        Request::StopAccount {
            key,
            revision,
            operation_id,
        }
        | Request::AdmitAccount {
            key,
            revision,
            operation_id,
        } => {
            ids::validate_key(key)?;
            anyhow::ensure!(
                revision.is_some() == operation_id.is_some(),
                "revision and operation_id go together"
            );
            if let Some(operation_id) = operation_id {
                ids::validate_id("operation_id", operation_id)?;
            }
            Ok(())
        }
        Request::LogoutCleanup {
            key, operation_id, ..
        }
        | Request::RemoveAccount { key, operation_id } => {
            ids::validate_key(key)?;
            if let Some(operation_id) = operation_id {
                ids::validate_id("operation_id", operation_id)?;
            }
            Ok(())
        }
        Request::ListUnits { key } => ids::validate_key(key),
        Request::CollectUsage {
            key,
            runtime_sha256,
            ..
        } => {
            ids::validate_key(key)?;
            ids::validate_hex("runtime_sha256", runtime_sha256, 64)
        }
    }
}

/// `cga-<key>-<mode>-<nonce>.service`; returns the key.
pub fn validate_unit_name(unit: &str) -> Result<&str> {
    let parsed = (|| {
        let stem = unit
            .strip_suffix(".service")?
            .strip_prefix(ids::USER_PREFIX)?;
        let (key, rest) = stem.split_once('-')?;
        let (mode, nonce) = rest.split_once('-')?;
        ids::validate_key(key).ok()?;
        ids::validate_hex("nonce", nonce, 16).ok()?;
        matches!(mode, "runtime" | "login" | "logout" | "probe" | "usage").then_some(key)
    })();
    parsed.ok_or_else(|| anyhow::anyhow!("invalid unit name"))
}

/// Strict standard base64 (with padding).
pub fn decode_base64(text: &str) -> Result<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    anyhow::ensure!(
        bytes.len().is_multiple_of(4),
        "base64 length is not a multiple of 4"
    );
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let padding = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        anyhow::ensure!(
            padding <= 2 && (padding == 0 || last),
            "invalid base64 padding"
        );
        let mut word = 0_u32;
        for &byte in &chunk[..4 - padding] {
            word = (word << 6) | value(byte).ok_or_else(|| anyhow::anyhow!("invalid base64"))?;
        }
        word <<= 6 * padding;
        let decoded = word.to_be_bytes();
        out.extend_from_slice(&decoded[1..4 - padding]);
    }
    Ok(out)
}

/// A successful response line.
#[must_use]
pub fn ok_line(result: &Value) -> Vec<u8> {
    line(&serde_json::json!({ "ok": true, "result": result }))
}

/// A failure response line.
#[must_use]
pub fn error_line(failure: &Failure) -> Vec<u8> {
    line(&serde_json::json!({
        "ok": false,
        "error": { "code": failure.code, "message": failure.message },
    }))
}

fn line(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(value).expect("JSON values serialize");
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "0123456789abcdef";

    #[test]
    fn requests_parse_strictly() {
        let request =
            parse_request(br#"{"op":"ensure_account","service_id":"s","account_id":"a"}"#).unwrap();
        assert_eq!(
            request,
            Request::EnsureAccount {
                service_id: "s".into(),
                account_id: "a".into()
            }
        );
        let unknown =
            parse_request(br#"{"op":"ensure_account","service_id":"s","account_id":"a","x":1}"#);
        assert_eq!(unknown.unwrap_err().code, Code::InvalidRequest);
        assert!(parse_request(br#"{"op":"nope"}"#).is_err());
        assert!(parse_request(br#"{"op":"stop_account"}"#).is_err());
        assert!(parse_request(br#"{"op":"stop_account","key":"../etc"}"#).is_err());
        assert!(
            parse_request(br#"{"op":"ensure_account","service_id":"","account_id":"a"}"#).is_err()
        );
        assert!(
            parse_request(br#"{"op":"ensure_account","service_id":"a\nb","account_id":"a"}"#)
                .is_err()
        );
        let oversize = format!(
            r#"{{"op":"stop_account","key":"{KEY}","pad":"{}"}}"#,
            "x".repeat(MAX_LINE_BYTES)
        );
        assert!(
            parse_request(oversize.as_bytes())
                .unwrap_err()
                .message
                .contains("too long")
        );
    }

    #[test]
    fn start_requests_are_validated() {
        let sha = "a".repeat(64);
        let good = format!(
            r#"{{"op":"start","key":"{KEY}","mode":"runtime","credential_generation":3,"runtime_sha256":"{sha}","session_id":"s1","descriptor_b64":"eyJ4IjoxfQ=="}}"#
        );
        assert!(matches!(
            parse_request(good.as_bytes()).unwrap(),
            Request::Start {
                mode: Mode::Runtime,
                ..
            }
        ));
        let bad_mode = good.replace("\"runtime\"", "\"shell\"");
        assert!(parse_request(bad_mode.as_bytes()).is_err());
        let bad_b64 = good.replace("eyJ4IjoxfQ==", "eyJ4Ijox$Q==");
        assert!(parse_request(bad_b64.as_bytes()).is_err());
        let argv_injection = good.replace("\"s1\"", "\"s1\\u0000--x\"");
        assert!(parse_request(argv_injection.as_bytes()).is_err());
        // A usage unit's output never reaches the caller as raw pipes.
        let usage = good.replace("\"runtime\"", "\"usage\"");
        assert!(
            parse_request(usage.as_bytes())
                .unwrap_err()
                .message
                .contains("collect_usage")
        );
    }

    #[test]
    fn collect_usage_requests_are_validated() {
        let sha = "b".repeat(64);
        let good = format!(
            r#"{{"op":"collect_usage","key":"{KEY}","credential_generation":2,"runtime_sha256":"{sha}"}}"#
        );
        assert_eq!(
            parse_request(good.as_bytes()).unwrap(),
            Request::CollectUsage {
                key: KEY.into(),
                credential_generation: 2,
                runtime_sha256: sha.clone(),
            }
        );
        assert!(parse_request(good.replace(KEY, "../etc").as_bytes()).is_err());
        assert!(parse_request(good.replace(&sha, "zz").as_bytes()).is_err());
        let extra = good.replace('}', r#","argv":["sh"]}"#);
        assert!(parse_request(extra.as_bytes()).is_err());
    }

    #[test]
    fn base64_round_trips() {
        assert_eq!(decode_base64("eyJ4IjoxfQ==").unwrap(), br#"{"x":1}"#);
        assert_eq!(decode_base64("YWI=").unwrap(), b"ab");
        assert_eq!(decode_base64("YWJj").unwrap(), b"abc");
        assert_eq!(decode_base64("").unwrap(), b"");
        assert!(decode_base64("YW=j").is_err());
        assert!(decode_base64("YWJ").is_err());
    }

    #[test]
    fn unit_names_are_validated() {
        assert_eq!(
            validate_unit_name("cga-0123456789abcdef-runtime-00112233aabbccdd.service").unwrap(),
            KEY
        );
        assert_eq!(
            validate_unit_name("cga-0123456789abcdef-usage-00112233aabbccdd.service").unwrap(),
            KEY
        );
        assert!(validate_unit_name("cga-0123456789abcdef-shell-00112233aabbccdd.service").is_err());
        assert!(validate_unit_name("sshd.service").is_err());
        assert!(validate_unit_name("cga-0123456789abcdef-runtime-0011.service").is_err());
    }
}

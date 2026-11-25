//! Bounded, unsequenced Antigravity post-edit diagnostic inspection wire types
//! and independent Machine-to-Controller HTTPS transport constants (R4).
//!
//! These requests are owner-authorized, session-scoped read requests (never a
//! trusted root-edit proof). Worker <-> Broker <-> `machine_cli` carry them on
//! bounded low-priority local Unix frames (`Runtime 9`); `machine_cli` <->
//! Controller carry them on an independent authenticated HTTPS endpoint so zero
//! diagnostic bytes enter the primary Machine WebSocket.

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostEditScope {
    pub binding_id: String,
    pub revision: u64,
    pub incarnation: String,
}

impl PostEditScope {
    #[must_use]
    pub fn new(binding_id: String, revision: u64, incarnation: String) -> Self {
        Self {
            binding_id,
            revision,
            incarnation,
        }
    }

    #[must_use]
    pub fn matches_parts(&self, binding_id: &str, revision: u64, incarnation: &str) -> bool {
        self.binding_id == binding_id
            && self.revision == revision
            && self.incarnation == incarnation
    }
}

#[must_use]
pub fn valid_inspect_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Minimum local runtime protocol version supporting `Frame::PostEditInspectRequest`
/// and `Frame::PostEditInspectReply` on Worker <-> Broker <-> `machine_cli`.
pub const POST_EDIT_INSPECT_RUNTIME_PROTOCOL_VERSION: u16 = 9;

/// Upgrade request/response header advertising Controller support for the
/// independent authenticated HTTPS post-edit inspection route.
pub const POST_EDIT_INSPECT_HEADER: &str = "x-cowboy-machine-post-edit";
pub const POST_EDIT_INSPECT_PROTOCOL: &str = "post-edit-v1";
/// Per-primary-connection secret token returned on `/api/machine/connect`
/// upgrade, registered only after Machine Ed25519 challenge acceptance, and
/// revoked automatically when that primary connection ends.
pub const POST_EDIT_INSPECT_TOKEN_HEADER: &str = "x-cowboy-machine-post-edit-token";
/// Controller HTTPS endpoint for Machine-proxied post-edit inspections.
pub const POST_EDIT_INSPECT_PATH: &str = "/api/machine/post-edit-inspect";

/// Hard serialized request bound (both local Unix frame and HTTPS body).
pub const MAX_POST_EDIT_REQUEST_BYTES: usize = 8 * 1024;
/// Hard serialized reply bound (both HTTPS response and local Unix frame).
pub const MAX_POST_EDIT_REPLY_BYTES: usize = 16 * 1024;
/// Hard serialized projected payload bound inside `PostEditInspectOutcome::Projected`.
pub const MAX_POST_EDIT_PAYLOAD_BYTES: usize = 12 * 1024;
/// Maximum end-to-end inspection deadline across all hops.
pub const MAX_POST_EDIT_DEADLINE_MS: u64 = 6_000;
/// Minimum remaining budget required to admit or forward an inspection; values
/// below this are refused immediately and never rounded upward.
pub const MIN_POST_EDIT_DEADLINE_MS: u64 = 250;
/// Minimum remaining budget required immediately before starting a cold buffer
/// acquisition or mirror refresh.
pub const MIN_POST_EDIT_COLD_START_MS: u64 = 500;
/// Maximum UTF-8 byte length of a workspace-relative POSIX path.
pub const MAX_POST_EDIT_PATH_BYTES: usize = 4096;

/// Allowed closed refusal reason codes (never raw backend/system error strings).
pub const ALLOWED_REFUSAL_REASONS: &[&str] = &[
    "unsupported",
    "restricted",
    "permission_denied",
    "superagent_scope_denied",
    "capacity",
    "timeout",
    "not_open",
    "unavailable",
    "invalid_request",
];

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostEditInspectRequest {
    pub session_id: String,
    pub worker_epoch: String,
    /// Worker-generated ID on Worker<->Broker leg; replaced with a
    /// broker-minted globally unique correlation ID on Broker<->`machine_cli`<->Controller.
    pub request_id: String,
    /// Exact active turn ID validated against the Worker's active turn nonce
    /// and checked against Controller's live `current_turn_id`.
    pub turn_id: String,
    pub scope: PostEditScope,
    /// Untrusted workspace-relative POSIX file path (no leading `/`, no `..`).
    pub path: String,
    /// Untrusted 64-char lowercase hex SHA-256 of LF-normalized UTF-8 text.
    pub expected_lf_sha256: String,
    /// Remaining end-to-end budget in ms (`MIN..=MAX`; strictly clamped downward).
    pub remaining_ms: u64,
    /// Optional canary-only correlation ID linking Controller logs to the
    /// isolated canary relay stdout trace. Absent in all production runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_inspect_id: Option<String>,
}

impl std::fmt::Debug for PostEditInspectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostEditInspectRequest")
            .field("session_id", &self.session_id)
            .field("worker_epoch", &self.worker_epoch)
            .field("request_id", &self.request_id)
            .field("turn_id", &self.turn_id)
            .field("remaining_ms", &self.remaining_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostEditInspectReply {
    pub session_id: String,
    pub worker_epoch: String,
    pub request_id: String,
    pub scope: PostEditScope,
    pub outcome: PostEditInspectOutcome,
}

impl std::fmt::Debug for PostEditInspectReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PostEditInspectReply")
            .field("session_id", &self.session_id)
            .field("worker_epoch", &self.worker_epoch)
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum PostEditInspectOutcome {
    Projected {
        payload: serde_json::Value,
        matches_expected: bool,
    },
    Refused {
        reason: String,
    },
}

/// Strictly clamp `ms` downward to `<= MAX_POST_EDIT_DEADLINE_MS`, returning
/// `None` whenever less than `MIN_POST_EDIT_DEADLINE_MS` remains (never
/// rounding small values upward).
#[must_use]
pub fn clamp_remaining_ms(ms: u64) -> Option<u64> {
    let clamped = ms.min(MAX_POST_EDIT_DEADLINE_MS);
    (clamped >= MIN_POST_EDIT_DEADLINE_MS).then_some(clamped)
}

#[must_use]
pub fn valid_relative_posix_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > MAX_POST_EDIT_PATH_BYTES
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || path.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n')
    {
        return false;
    }
    let mut parts = 0usize;
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return false;
        }
        parts += 1;
    }
    parts > 0
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[must_use]
pub fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[must_use]
pub fn valid_refusal_reason(reason: &str) -> bool {
    ALLOWED_REFUSAL_REASONS.contains(&reason)
}

impl PostEditInspectRequest {
    /// Validate structural fields independent of `remaining_ms` bounds so ingress
    /// layers can return `"timeout"` for `< 250 ms` and clamp `> 6000 ms` downward.
    #[must_use]
    pub fn validate_structure(&self) -> bool {
        valid_inspect_id(&self.session_id)
            && !self.worker_epoch.is_empty()
            && self.worker_epoch.len() <= 128
            && valid_inspect_id(&self.request_id)
            && !self.turn_id.is_empty()
            && self.turn_id.len() <= 128
            && !self.scope.binding_id.is_empty()
            && !self.scope.incarnation.is_empty()
            && valid_relative_posix_path(&self.path)
            && valid_sha256_hex(&self.expected_lf_sha256)
            && self
                .canary_inspect_id
                .as_deref()
                .is_none_or(valid_inspect_id)
            && serde_json::to_vec(self)
                .is_ok_and(|bytes| bytes.len() <= MAX_POST_EDIT_REQUEST_BYTES)
    }

    #[must_use]
    pub fn validate(&self) -> bool {
        self.validate_structure()
            && clamp_remaining_ms(self.remaining_ms) == Some(self.remaining_ms)
    }
}

impl PostEditInspectReply {
    #[must_use]
    pub fn refused(
        session_id: String,
        worker_epoch: String,
        request_id: String,
        scope: PostEditScope,
        reason: &'static str,
    ) -> Self {
        Self {
            session_id,
            worker_epoch,
            request_id,
            scope,
            outcome: PostEditInspectOutcome::Refused {
                reason: reason.to_owned(),
            },
        }
    }

    #[must_use]
    pub fn validate(&self) -> bool {
        let outcome_ok = match &self.outcome {
            PostEditInspectOutcome::Refused { reason } => valid_refusal_reason(reason),
            PostEditInspectOutcome::Projected { payload, .. } => {
                payload.is_object()
                    && serde_json::to_vec(payload)
                        .is_ok_and(|bytes| bytes.len() <= MAX_POST_EDIT_PAYLOAD_BYTES)
            }
        };
        outcome_ok
            && valid_inspect_id(&self.session_id)
            && !self.worker_epoch.is_empty()
            && self.worker_epoch.len() <= 128
            && valid_inspect_id(&self.request_id)
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= MAX_POST_EDIT_REPLY_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_scope() -> PostEditScope {
        PostEditScope {
            binding_id: "bind-1".into(),
            revision: 1,
            incarnation: "inc-1".into(),
        }
    }

    fn sample_request() -> PostEditInspectRequest {
        PostEditInspectRequest {
            session_id: "sess-1".into(),
            worker_epoch: "epoch-1".into(),
            request_id: "req-1".into(),
            turn_id: "turn-1".into(),
            scope: sample_scope(),
            path: "src/lib.rs".into(),
            expected_lf_sha256: "a".repeat(64),
            remaining_ms: 4_000,
            canary_inspect_id: None,
        }
    }

    #[test]
    fn validates_relative_posix_paths_and_rejects_escapes() {
        assert!(valid_relative_posix_path("a.rs"));
        assert!(valid_relative_posix_path("src/nested/file.ts"));
        for bad in [
            "",
            "/abs/path.rs",
            "src/",
            "src//lib.rs",
            ".",
            "..",
            "../escape.rs",
            "src/../lib.rs",
            "src/./lib.rs",
            "src/a\0b.rs",
            "src/a\nb.rs",
        ] {
            assert!(!valid_relative_posix_path(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn clamp_remaining_never_rounds_small_values_upward() {
        assert_eq!(clamp_remaining_ms(0), None);
        assert_eq!(clamp_remaining_ms(249), None);
        assert_eq!(clamp_remaining_ms(250), Some(250));
        assert_eq!(clamp_remaining_ms(4_000), Some(4_000));
        assert_eq!(clamp_remaining_ms(6_000), Some(6_000));
        assert_eq!(clamp_remaining_ms(25_000), Some(6_000));
    }

    #[test]
    fn request_and_reply_round_trip_and_reject_unknown_fields() {
        let req = sample_request();
        assert!(req.validate());
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("canary_inspect_id").is_none());
        let mut extra = json.clone();
        extra["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<PostEditInspectRequest>(extra).is_err());

        let reply = PostEditInspectReply::refused(
            "sess-1".into(),
            "epoch-1".into(),
            "req-1".into(),
            sample_scope(),
            "restricted",
        );
        assert!(reply.validate());
        let mut bad_reply = serde_json::to_value(&reply).unwrap();
        bad_reply["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<PostEditInspectReply>(bad_reply).is_err());
    }
}

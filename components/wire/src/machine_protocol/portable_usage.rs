//! Usage of named portable accounts (protocol 50) and the closed usage failure
//! classification shared by the Controller and the Machine.
//!
//! A portable account keeps its credential in the account's Service vault,
//! replicated to every Machine at account scope. Its usage runs the Provider's
//! signed collector on one Machine, in the account's own replica scope (the
//! environment a session on that account receives), never with the Machine's
//! Service-account credential. Only a closed reason code leaves the Machine.

use serde::{Deserialize, Serialize};

/// One named portable account's usage query. Answered only by
/// `MachineEvent::PluginHostResponse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableUsageRequest {
    /// Named account whose vault replica the collector runs in.
    pub account_id: crate::provider_account::ProviderAccountId,
    /// Exact active Plugin generation that owns the signed collector.
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    /// The account vault's current credential generation of `plugin_id`. The
    /// Machine refuses unless its account replica is exactly this generation,
    /// before and after the collector runs.
    pub auth_generation: u64,
    /// The collector's JSON request; the Machine sets its `operation`.
    pub payload: serde_json::Value,
}

/// One owner-confirmed or scheduled banked reset of a named portable
/// account, run by the same signed collector (`consume_reset`) in the same
/// account replica scope and generation as [`PortableUsageRequest`]. Answered
/// only by `MachineEvent::PluginHostResponse`; a failure keeps `started`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableResetRequest {
    pub account_id: crate::provider_account::ProviderAccountId,
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    pub auth_generation: u64,
    /// The collector's bounded `consume_reset` input.
    pub reset: super::NativeResetRequest,
}

/// Coarse kind of a usage collection failure, from its diagnostic text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageFailureKind {
    Transient,
    Authentication,
    Configuration,
    Other,
}

/// Closed reason codes of a portable usage query. Collector output, paths and
/// credentials never leave the Machine; only one of these codes does.
#[must_use]
pub fn portable_usage_error_code(detail: &str) -> &'static str {
    const CLOSED: [&str; 13] = [
        "machine_usage_update_required",
        "machine_connection_changed",
        "portable_usage_account_required",
        "portable_usage_busy",
        "portable_usage_replica_pending",
        "portable_usage_authentication_required",
        "portable_usage_collector_unavailable",
        "portable_usage_collector_failed",
        "portable_usage_temporarily_unavailable",
        "portable_usage_throttled",
        "portable_reset_request_invalid",
        "portable_reset_unsupported",
        "portable_reset_outcome_unknown",
    ];
    if let Some(code) = CLOSED.iter().find(|code| **code == detail) {
        return code;
    }
    let lower = detail.to_ascii_lowercase();
    // Nothing started: the account replica is not (yet) the vault's
    // generation, or the exact Plugin generation moved before invocation.
    if lower.contains("changed before host invocation") || lower.contains("auth replica") {
        return "portable_usage_replica_pending";
    }
    if lower.contains("has no usage capability")
        || lower.contains("has no signed host bundle")
        || lower.contains("plugin is not active on this machine")
    {
        return "portable_usage_collector_unavailable";
    }
    if provider_throttled(detail) {
        return "portable_usage_throttled";
    }
    match classify_usage_failure(detail) {
        UsageFailureKind::Authentication => "portable_usage_authentication_required",
        UsageFailureKind::Configuration => "portable_usage_collector_unavailable",
        UsageFailureKind::Transient => "portable_usage_temporarily_unavailable",
        UsageFailureKind::Other => "portable_usage_collector_failed",
    }
}

/// The Provider itself asked us to slow down: never retry quickly.
pub fn provider_throttled(detail: &str) -> bool {
    // Not "rate limit": collectors read the "rate limits" endpoints, so their
    // ordinary failures name them.
    let detail = detail.to_ascii_lowercase();
    contains_http_status(&detail, 429)
        || ["too many requests", "rate limited", "rate_limit_exceeded"]
            .iter()
            .any(|needle| detail.contains(needle))
}

pub fn contains_http_status(detail: &str, status: u16) -> bool {
    let status = status.to_string();
    [
        format!(" {status} "),
        format!("http {status}"),
        format!("status {status}"),
        format!("status={status}"),
        format!("status: {status}"),
        format!("\"status\":{status}"),
        format!("\"status\": {status}"),
        format!("error ({status} "),
    ]
    .iter()
    .any(|needle| detail.contains(needle))
}

pub fn classify_usage_failure(detail: &str) -> UsageFailureKind {
    let detail = detail.to_ascii_lowercase();
    if [
        "authentication required",
        "authorization required",
        "unauthorized",
        "forbidden",
        "not signed in",
        "login required",
        "sign in required",
        "refresh token was already used",
        "http 401",
        "http 403",
        "status 401",
        "status 403",
        "401 unauthorized",
        "403 forbidden",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
    {
        return UsageFailureKind::Authentication;
    }
    if [
        "not configured",
        "unsupported",
        "does not expose",
        "method not found",
        "invalid request",
        "has no plugin command",
        "collector argv is empty",
        "spawn plugin command",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
    {
        return UsageFailureKind::Configuration;
    }
    if [
        "timed out",
        "timeout",
        "temporarily unavailable",
        // The Plugin host fence refuses an observation that straddled an
        // installation or authentication change, including the credential
        // refresh a collector performs itself; the next attempt observes the
        // settled generation.
        "changed during host observation",
        "changed before host invocation",
        // A Plugin installation or its reconciliation briefly fences host
        // observations; the account itself is unaffected, so keep its usage.
        "requires install attempt reconciliation",
        "requires machine operation reconciliation",
        "installation requires reconciliation",
        "lifecycle is changing",
        "service unavailable",
        "connection refused",
        "connection reset",
        "connection closed",
        "network error",
        "dns error",
        "failed to connect",
        "http 408",
        "http 409",
        "http 429",
        "status 408",
        "status 409",
        "status 429",
        "408 request timeout",
        "409 conflict",
        "429 too many requests",
    ]
    .iter()
    .any(|needle| detail.contains(needle))
        || (500..=599).any(|status| contains_http_status(&detail, status))
    {
        return UsageFailureKind::Transient;
    }
    UsageFailureKind::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_closed_codes_leave_the_machine() {
        for (detail, code) in [
            ("portable_usage_busy", "portable_usage_busy"),
            (
                "machine_usage_update_required",
                "machine_usage_update_required",
            ),
            (
                "active Plugin or authentication generation changed before host invocation",
                "portable_usage_replica_pending",
            ),
            (
                "session Provider auth replica is missing",
                "portable_usage_replica_pending",
            ),
            (
                "Plugin host command exited 1: account/read: 401 Unauthorized",
                "portable_usage_authentication_required",
            ),
            (
                "Plugin host command exited 1: HTTP 429 Too Many Requests",
                "portable_usage_throttled",
            ),
            (
                "Plugin authentication changed during host observation",
                "portable_usage_temporarily_unavailable",
            ),
            (
                "installed Plugin host has no usage capability",
                "portable_usage_collector_unavailable",
            ),
            (
                "Plugin host command exited 1: /home/x/.codex/auth.json sk-secret",
                "portable_usage_collector_failed",
            ),
        ] {
            assert_eq!(portable_usage_error_code(detail), code, "{detail}");
        }
    }

    #[test]
    fn requests_are_closed() {
        let value = serde_json::json!({
            "account_id": "11111111-1111-4111-8111-111111111111",
            "plugin_id": "codex", "plugin_version": "1.0.0",
            "generation_digest": "sha256:x", "auth_generation": 7,
            "payload": {"operation": "collect"},
        });
        let request: PortableUsageRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.auth_generation, 7);
        let mut extra = value;
        extra["home"] = serde_json::json!("/tmp");
        assert!(serde_json::from_value::<PortableUsageRequest>(extra).is_err());
    }
}

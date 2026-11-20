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
    /// The exact sidecar Plugin generations (the gateways) the
    /// Controller checked for this Machine. All participants share the
    /// collector's vault generation: the Machine refuses unless each account
    /// replica is exactly `auth_generation`, and starts only these sidecars,
    /// so one observation never mixes two credentials or unchecked Plugins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sidecars: Vec<PortableSidecar>,
}

/// One exact sidecar Plugin generation a portable collection may start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSidecar {
    pub plugin_id: String,
    pub generation_digest: String,
}

/// The signed collector file whose presence declares that it never asks its
/// Provider CLI to redeem a refresh token when the request carries
/// `"token_refresh": false` (it reports authentication required instead).
/// A Provider whose CLI rotates credentials runs named portable usage or
/// resets only with such a collector: a usage poll must never race a
/// session's refresh-token redemption.
pub const REFRESH_FREE_DECLARATION: &str = "collector/portable-account.json";

/// Message of a reset whose result is indeterminate.
pub const RESET_UNKNOWN_MESSAGE: &str =
    "The reset outcome is unknown. Refresh usage before trying again.";

/// Message of a reset refused because the account must sign in again; the
/// shared classifier recognizes it as an authentication failure.
pub const RESET_AUTHENTICATION_MESSAGE: &str =
    "Usage authentication required: sign in again, then refresh usage. No reset was sent.";

/// One bounded display field of a collector reset reply: short printable text
/// without structured payloads (an upstream RPC error body never survives).
fn bounded_text(value: Option<&serde_json::Value>, max: usize) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| {
            !text.is_empty()
                && text.len() <= max
                && text.chars().all(|c| c.is_ascii_graphic() || c == ' ')
                && !text.contains(['{', '}', '[', ']', '"', '\\', '<', '>'])
        })
        .map(str::to_owned)
}

/// A reset grant identifier: bounded and opaque.
fn bounded_credit(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .filter(|id| {
            (1..=128).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.'))
        })
        .map(str::to_owned)
}

/// Project a collector's `consume_reset` reply onto the closed reset reply:
/// `{outcome, credit_id}` or `{error:{message, call_may_have_reached_provider,
/// credit_id}}`, every field bounded. Only explicit evidence decides: a
/// success without a determinate outcome, or a refusal that does not state it
/// never reached the Provider, is unknown.
#[must_use]
pub fn project_reset_reply(reply: &serde_json::Value) -> serde_json::Value {
    let unknown = |credit: Option<String>, message: Option<String>| {
        serde_json::json!({"error": {
            "message": message.unwrap_or_else(|| RESET_UNKNOWN_MESSAGE.to_owned()),
            "call_may_have_reached_provider": true, "credit_id": credit}})
    };
    if let Some(error) = reply.get("error") {
        let credit = bounded_credit(error.get("credit_id"));
        if error.get("call_may_have_reached_provider") != Some(&serde_json::Value::Bool(false)) {
            return unknown(credit, bounded_text(error.get("message"), 200));
        }
        let raw = error.get("message").and_then(serde_json::Value::as_str);
        // Keep the one signal the Service acts on (authentication backoff).
        let message = if raw
            .is_some_and(|raw| classify_usage_failure(raw) == UsageFailureKind::Authentication)
        {
            Some(RESET_AUTHENTICATION_MESSAGE.to_owned())
        } else {
            bounded_text(error.get("message"), 200)
        }
        .unwrap_or_else(|| "The Provider refused this reset; no reset was sent.".to_owned());
        return serde_json::json!({"error": {"message": message,
            "call_may_have_reached_provider": false, "credit_id": credit}});
    }
    let credit = bounded_credit(reply.get("credit_id"));
    match bounded_text(reply.get("outcome"), 120) {
        Some(outcome)
            if !matches!(
                outcome.to_ascii_lowercase().as_str(),
                "unknown" | "indeterminate" | "pending"
            ) =>
        {
            serde_json::json!({"outcome": outcome, "credit_id": credit})
        }
        _ => unknown(credit, None),
    }
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
    const CLOSED: [&str; 11] = [
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
        "portable_usage_refresh_unsupported",
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
    fn reset_replies_keep_only_bounded_explicit_evidence() {
        use serde_json::json;
        let unknown = |credit: serde_json::Value| {
            json!({"error": {"message": RESET_UNKNOWN_MESSAGE,
                "call_may_have_reached_provider": true, "credit_id": credit}})
        };
        for (reply, projected) in [
            (
                json!({"outcome": "reset", "credit_id": "grant:1"}),
                json!({"outcome": "reset", "credit_id": "grant:1"}),
            ),
            // A literal unknown outcome is not success.
            (
                json!({"outcome": "unknown", "credit_id": "c1"}),
                unknown(json!("c1")),
            ),
            (json!({"credit_id": "c1"}), unknown(json!("c1"))),
            // A refusal must state it never reached the Provider.
            (
                json!({"error": {"message": "refused"}}),
                json!({"error": {"message": "refused",
                    "call_may_have_reached_provider": true, "credit_id": null}}),
            ),
            (
                json!({"error": {"message": "nearest reset credit changed; refresh and confirm again",
                    "call_may_have_reached_provider": false, "credit_id": "c2"}}),
                json!({"error": {"message": "nearest reset credit changed; refresh and confirm again",
                    "call_may_have_reached_provider": false, "credit_id": "c2"}}),
            ),
            // An upstream RPC error body never leaves; the certainty stays.
            (
                json!({"error": {"message": "consume: {\"code\":-32000,\"token\":\"sk\"}",
                    "call_may_have_reached_provider": false, "credit_id": "bad id with spaces"}}),
                json!({"error": {"message": "The Provider refused this reset; no reset was sent.",
                    "call_may_have_reached_provider": false, "credit_id": null}}),
            ),
            (
                json!({"error": {"message": "account/rateLimits/read: {\"status\":401} 401 Unauthorized",
                    "call_may_have_reached_provider": false}}),
                json!({"error": {"message": RESET_AUTHENTICATION_MESSAGE,
                    "call_may_have_reached_provider": false, "credit_id": null}}),
            ),
        ] {
            assert_eq!(project_reset_reply(&reply), projected, "{reply}");
        }
        assert_eq!(
            classify_usage_failure(RESET_AUTHENTICATION_MESSAGE),
            UsageFailureKind::Authentication
        );
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

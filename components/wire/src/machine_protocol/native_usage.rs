use serde::{Deserialize, Serialize};

/// The collector's `consume_reset` input. The Controller supplies the owner's
/// confirmation of one exact grant and a stable idempotency key; the Machine
/// adds nothing else and the collector re-checks availability first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResetRequest {
    /// The signed usage binding's account (`anthropic`).
    pub provider: String,
    /// Sent to the Provider as its request ID; stable across a retry.
    pub idempotency_key: String,
    /// The grant the owner confirmed. Named resets are immediate only, so it
    /// is always present.
    #[serde(default)]
    pub expected_credit_id: Option<String>,
}

/// Where and until when one named reset may start: the exact signed release
/// the Controller selected and an absolute expiry (Unix ms) after which the
/// Machine never spawns the collector, so a request its Controller abandoned
/// can never spend late.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResetDispatch {
    pub plugin_version: String,
    pub generation_digest: String,
    pub expires_at_ms: i64,
    /// The spend is the reset's second paced Provider start: the Machine runs
    /// the identity read first and never starts the spend before this.
    pub spend_not_before_ms: i64,
}

/// The closed success outcomes of the signed Claude collector's
/// `consume_reset`. Anything else is an unknown outcome.
pub const NATIVE_RESET_OUTCOMES: [&str; 2] = ["reset", "already_used"];

/// Project a collector's `consume_reset` reply onto the closed reply that may
/// leave the Machine: `{outcome, credit_id}` with a closed outcome, or
/// `{error:{code, call_may_have_reached_provider, credit_id}}` with a closed
/// code. Collector text never leaves. Only a refusal that explicitly states it
/// never reached the Provider is unsent; an authentication failure keeps its
/// category whether or not it was reached, so it still backs off.
#[must_use]
pub fn project_native_reset_reply(reply: &serde_json::Value) -> serde_json::Value {
    use super::portable_usage::{UsageFailureKind, classify_usage_failure};
    let credit = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_str)
            .filter(|id| {
                (1..=128).contains(&id.len())
                    && id.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.')
                    })
            })
            .map(str::to_owned)
    };
    let failure = |code: &str, reached: bool, credit_id: Option<String>| {
        serde_json::json!({"error": {"code": code,
            "call_may_have_reached_provider": reached, "credit_id": credit_id}})
    };
    let Some(error) = reply.get("error") else {
        let credit_id = credit(reply.get("credit_id"));
        return match reply.get("outcome").and_then(serde_json::Value::as_str) {
            Some(outcome) if NATIVE_RESET_OUTCOMES.contains(&outcome) => {
                serde_json::json!({"outcome": outcome, "credit_id": credit_id})
            }
            _ => failure("reset_outcome_unknown", true, credit_id),
        };
    };
    let credit_id = credit(error.get("credit_id"));
    let reached =
        error.get("call_may_have_reached_provider") != Some(&serde_json::Value::Bool(false));
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let code = if classify_usage_failure(&message) == UsageFailureKind::Authentication {
        "reset_authentication_required"
    } else if reached {
        "reset_outcome_unknown"
    } else if message.contains("changed") {
        "reset_credit_changed"
    } else if message.contains("no ") && message.contains("reset") {
        "reset_unavailable"
    } else {
        "reset_refused"
    };
    failure(code, reached, credit_id)
}

impl NativeResetRequest {
    /// The bounded shapes the collector accepts; anything else never starts it.
    #[must_use]
    pub fn valid(&self) -> bool {
        let id = |value: &str, max: usize| {
            (1..=max).contains(&value.len())
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b':'))
        };
        id(&self.provider, 64)
            && id(&self.idempotency_key, 64)
            && !self.idempotency_key.contains(':')
            && self
                .expected_credit_id
                .as_deref()
                .is_some_and(|value| id(value, 64))
    }
}

/// Refusals a Machine reports only before a reset's collector started.
pub const PRE_START_RESET_REFUSALS: [&str; 20] = [
    "native_reset_request_invalid",
    "native_reset_expired",
    "native_reset_unsupported",
    "native_usage_busy",
    "native_usage_admission_expired",
    "native_usage_profile_changed",
    "native_usage_installation_changed",
    "native_usage_collector_unavailable",
    "native_usage_home_unsupported",
    "native_usage_authentication_required",
    "machine_usage_update_required",
    "machine_connection_changed",
    "portable_usage_account_required",
    "portable_usage_busy",
    "portable_usage_replica_pending",
    "portable_usage_collector_unavailable",
    "portable_usage_refresh_unsupported",
    "portable_usage_machine_unavailable",
    "portable_usage_authentication_required",
    "native_usage_location_unsupported",
];

/// Whether a refused reset certainly never reached the Provider: only an
/// explicit `started: false` with a recognised pre-start refusal. A missing
/// `started` field, an unknown code or `native_reset_outcome_unknown` may
/// follow a spent reset.
#[must_use]
pub fn reset_refusal_unsent(started: Option<bool>, code: &str) -> bool {
    started == Some(false) && PRE_START_RESET_REFUSALS.contains(&code)
}

/// Closed diagnostics for native observations; arbitrary subprocess output is
/// never an account API error or a log field.
pub fn native_usage_error_code(value: &str) -> &'static str {
    match value {
        "native_usage_busy" => "native_usage_busy",
        "native_usage_admission_expired" => "native_usage_admission_expired",
        "native_usage_profile_changed" => "native_usage_profile_changed",
        "native_usage_collector_unavailable" => "native_usage_collector_unavailable",
        "native_usage_home_unsupported" => "native_usage_home_unsupported",
        "native_usage_installation_changed" => "native_usage_installation_changed",
        "native_usage_authentication_required" => "native_usage_authentication_required",
        "native_usage_location_unsupported" => "native_usage_location_unsupported",
        "native_usage_collector_failed" => "native_usage_collector_failed",
        "native_reset_unsupported" => "native_reset_unsupported",
        "native_reset_request_invalid" => "native_reset_request_invalid",
        "native_reset_outcome_unknown" => "native_reset_outcome_unknown",
        "native_reset_expired" => "native_reset_expired",
        "machine_usage_update_required" => "machine_usage_update_required",
        "machine_connection_changed" => "machine_connection_changed",
        _ => "native_usage_unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn named_resets_need_reset_aware_machines_and_bounded_requests() {
        let value = serde_json::json!({"command":"consume_native_reset","request_id":"r",
            "profile":{"account_id":"11111111-1111-4111-8111-111111111111",
                "provider_id":"claude-code","request_id":"login","generation":1,
                "method":"native","provider_version":"1","generation_digest":"digest",
                "auth_contract_fingerprint":"contract",
                "identity":{"authority":"claude","subject":"otter","tenant":null}},
            "request":{"provider":"anthropic","idempotency_key":"cowboy-reset-1-2-3",
                "expected_credit_id":"grant_1:2"},
            "dispatch":{"plugin_version":"3.0.0","generation_digest":"sha256:x",
                "expires_at_ms":1,"spend_not_before_ms":1}});
        let command: MachineCommand = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(command.minimum_protocol(), NATIVE_RESET_PROTOCOL_VERSION);
        const {
            assert!(NATIVE_RESET_PROTOCOL_VERSION <= MACHINE_PROTOCOL_VERSION);
            assert!(NATIVE_RESET_PROTOCOL_VERSION > ACCOUNT_FENCE_PROTOCOL_VERSION);
        }
        assert_eq!(serde_json::to_value(&command).unwrap(), value);
        let MachineCommand::ConsumeNativeReset { request, .. } = command else {
            unreachable!()
        };
        assert!(request.valid());
        for invalid in [
            // Named resets are immediate: one confirmed grant, never "next".
            NativeResetRequest {
                expected_credit_id: None,
                ..request.clone()
            },
            NativeResetRequest {
                idempotency_key: String::new(),
                ..request.clone()
            },
            NativeResetRequest {
                idempotency_key: "a:b".into(),
                ..request.clone()
            },
            NativeResetRequest {
                idempotency_key: "x".repeat(65),
                ..request.clone()
            },
            NativeResetRequest {
                expected_credit_id: Some("grant 1".into()),
                ..request.clone()
            },
            NativeResetRequest {
                provider: "../anthropic".into(),
                ..request.clone()
            },
        ] {
            assert!(!invalid.valid(), "{invalid:?}");
        }
        assert_eq!(
            native_usage_error_code("native_reset_outcome_unknown"),
            "native_reset_outcome_unknown"
        );
    }

    #[test]
    fn only_closed_reset_replies_leave_the_machine() {
        use serde_json::json;
        let error = |code: &str, reached: bool, credit: serde_json::Value| {
            json!({"error": {"code": code, "call_may_have_reached_provider": reached,
                "credit_id": credit}})
        };
        for (reply, projected) in [
            (
                json!({"outcome": "reset", "credit_id": "grant:1"}),
                json!({"outcome": "reset", "credit_id": "grant:1"}),
            ),
            (
                json!({"outcome": "unknown", "credit_id": "grant:1"}),
                error("reset_outcome_unknown", true, json!("grant:1")),
            ),
            (
                json!({"outcome": "consumed"}),
                error("reset_outcome_unknown", true, json!(null)),
            ),
            (
                json!({"error": {"message": "Claude reset changed; refresh and confirm again.",
                    "call_may_have_reached_provider": false, "credit_id": "grant:2"}}),
                error("reset_credit_changed", false, json!("grant:2")),
            ),
            (
                json!({"error": {"message": "No Claude reset is currently usable.",
                    "call_may_have_reached_provider": false}}),
                error("reset_unavailable", false, json!(null)),
            ),
            // Missing reached evidence is unknown; collector text never leaves.
            (
                json!({"error": {"message": "{\"token\":\"secret\"}"}}),
                error("reset_outcome_unknown", true, json!(null)),
            ),
            // A 401 after the call keeps its authentication category.
            (
                json!({"error": {"message": "HTTP 401 Unauthorized",
                    "call_may_have_reached_provider": true}}),
                error("reset_authentication_required", true, json!(null)),
            ),
            (
                json!({"error": {"message": "Claude Code usage authentication required.",
                    "call_may_have_reached_provider": false}}),
                error("reset_authentication_required", false, json!(null)),
            ),
        ] {
            assert_eq!(project_native_reset_reply(&reply), projected, "{reply}");
        }
    }

    #[test]
    fn only_an_explicit_pre_start_refusal_is_unsent() {
        assert!(reset_refusal_unsent(Some(false), "native_reset_expired"));
        assert!(!reset_refusal_unsent(None, "native_reset_expired"));
        assert!(!reset_refusal_unsent(Some(true), "native_reset_expired"));
        assert!(!reset_refusal_unsent(
            Some(false),
            "native_reset_outcome_unknown"
        ));
        assert!(!reset_refusal_unsent(Some(false), "something new"));
    }
}

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
    /// The confirmed grant; `None` takes the Provider's next grant (a
    /// scheduled reset).
    #[serde(default)]
    pub expected_credit_id: Option<String>,
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
                .is_none_or(|value| id(value, 64))
    }
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
        "native_usage_collector_failed" => "native_usage_collector_failed",
        "native_reset_unsupported" => "native_reset_unsupported",
        "native_reset_request_invalid" => "native_reset_request_invalid",
        "native_reset_outcome_unknown" => "native_reset_outcome_unknown",
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
                "expected_credit_id":"grant_1:2"}});
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
        let scheduled = NativeResetRequest {
            expected_credit_id: None,
            ..request.clone()
        };
        assert!(scheduled.valid());
        for invalid in [
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
}

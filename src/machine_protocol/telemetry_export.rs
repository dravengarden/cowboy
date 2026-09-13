//! One managed OTLP attempt. These wire values are requests/evidence, never
//! reusable capabilities, binding mutations or persisted replay instructions.

use super::telemetry_binding::{BindingDigest, BindingSnapshot, binding_digest, valid_service};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub(crate) const ATTEMPT_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportAttempt {
    pub schema: u16,
    pub attempt_id: String,
    pub service_id: String,
    pub machine_id: String,
    pub binding: BindingSnapshot,
    pub payload: crate::otlp::Export,
    pub expires_at_ms: i64,
}

impl ExportAttempt {
    pub(crate) fn validate(&self) -> Result<()> {
        let id = |value: &str, min| {
            (min..=128).contains(&value.len())
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        };
        ensure!(
            self.schema == 1 && id(&self.attempt_id, 16),
            "invalid export identity"
        );
        ensure!(
            valid_service(&self.service_id) && id(&self.machine_id, 1),
            "invalid export owner"
        );
        ensure!(
            (1..=9_007_199_254_740_991).contains(&self.expires_at_ms),
            "invalid export deadline"
        );
        self.binding.validate()?;
        ensure!(
            self.binding.selection.is_some(),
            "export requires a selected managed binding"
        );
        self.payload.decode()?;
        Ok(())
    }

    pub(crate) fn request_digest(&self) -> Result<BindingDigest> {
        self.validate()?;
        Ok(binding_digest(&serde_json::to_vec(self)?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportReceipt {
    pub request_digest: BindingDigest,
    pub outcome: ExportOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExportOutcome {
    Delivered {},
    Partial {
        rejected_items: u64,
    },
    Disabled {},
    /// No HTTP was admitted; no error text, endpoint or token crosses the wire.
    NotAdmitted {},
    /// An HTTP attempt was admitted, but delivery was not established. It may
    /// already have emitted data. Never retry or claim restoration from this.
    Unknown {},
}

impl ExportReceipt {
    #[cfg(any(feature = "full", test))]
    pub(crate) fn matches(&self, request: &ExportAttempt) -> bool {
        request
            .request_digest()
            .is_ok_and(|d| d == self.request_digest)
            && match self.outcome {
                ExportOutcome::Partial { rejected_items } => request
                    .payload
                    .decode()
                    .is_ok_and(|(_, sent)| rejected_items > 0 && rejected_items <= sent as u64),
                _ => true,
            }
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> ExportAttempt {
    ExportAttempt {
        schema: 1,
        attempt_id: "managed-export-fixture".into(),
        service_id: "service-test".into(),
        machine_id: "machine-test".into(),
        binding: super::telemetry_binding::fixture().after().unwrap(),
        payload: crate::otlp::Export {
            signal: crate::otlp::Signal::Logs,
            protobuf: String::new(),
        },
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 15_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_export_request_digest_binds_every_axis_and_receipts_are_closed() {
        let request = fixture();
        let receipt = ExportReceipt {
            request_digest: request.request_digest().unwrap(),
            outcome: ExportOutcome::Delivered {},
        };
        assert!(receipt.matches(&request));
        let value = serde_json::to_value(&request).unwrap();
        for (key, replacement) in [
            ("schema", serde_json::json!(2)),
            ("attempt_id", serde_json::json!("other-export-attempt")),
            ("service_id", serde_json::json!("other-service")),
            ("machine_id", serde_json::json!("other-machine")),
            ("expires_at_ms", serde_json::json!(1)),
            (
                "payload",
                serde_json::json!({"signal": "metrics", "protobuf": ""}),
            ),
        ] {
            let mut changed = value.clone();
            changed[key] = replacement;
            let changed: ExportAttempt = serde_json::from_value(changed).unwrap();
            assert!(!receipt.matches(&changed), "{key}");
        }
        for key in ["revision", "policy_epoch"] {
            let mut changed = value.clone();
            changed["binding"][key] = serde_json::json!("99");
            assert!(!receipt.matches(&serde_json::from_value(changed).unwrap()));
        }
        for replacement in [
            serde_json::json!(1),
            serde_json::json!("01"),
            serde_json::Value::Null,
        ] {
            let mut changed = value.clone();
            changed["binding"]["policy_epoch"] = replacement;
            assert!(serde_json::from_value::<ExportAttempt>(changed).is_err());
        }
        let mut extra = value;
        extra["reusable_grant"] = true.into();
        assert!(serde_json::from_value::<ExportAttempt>(extra).is_err());
        assert!(
            serde_json::from_str::<ExportOutcome>(r#"{"kind":"delivered","endpoint":"secret"}"#)
                .is_err()
        );
        let mut partial = receipt;
        partial.outcome = ExportOutcome::Partial { rejected_items: 1 };
        assert!(
            !partial.matches(&request),
            "cannot reject more items than sent"
        );
    }

    #[test]
    fn managed_export_payload_is_bounded_redacted_and_cannot_downgrade_protocol() {
        let mut request = fixture();
        request.payload.protobuf = "sensitive-body".into();
        assert!(!format!("{request:?}").contains("sensitive-body"));
        assert!(request.validate().is_err());
        request.payload.protobuf = "A".repeat(400_000);
        assert!(request.validate().is_err());
        let command = super::super::MachineCommand::ExportBoundTelemetry {
            request_id: "rpc".into(),
            attempt: Box::new(fixture()),
        };
        assert_eq!(command.minimum_protocol(), 16);
        let encoded = serde_json::to_vec(&command).unwrap();
        let decoded: super::super::MachineCommand = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.minimum_protocol(), 16);
    }
}

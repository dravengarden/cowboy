use super::*;
use crate::machine_protocol::telemetry_binding::BindingInstallation;
use crate::machine_protocol::telemetry_export::{ExportAttempt, ExportOutcome, ExportReceipt};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(in super::super) struct Wire {
    pub entries: Vec<WireExport>,
    target: Option<BindingInstallation>,
    pending: BTreeMap<String, usize>,
    seen: BTreeSet<String>,
    next_ack: Ack,
}

impl Wire {
    pub fn target(&mut self, target: BindingInstallation) -> Result<(), Failure> {
        check(self.target.is_none() && self.entries.is_empty())?;
        self.target = Some(target);
        Ok(())
    }

    pub fn arm(&mut self, ack: Ack) -> Result<(), Failure> {
        check(self.pending.is_empty() && self.next_ack == Ack::Forward)?;
        self.next_ack = ack;
        Ok(())
    }

    pub fn begin(&mut self, id: String, attempt: ExportAttempt) -> Result<(), Failure> {
        let digest = attempt
            .request_digest()
            .map_err(|_| Failure::WrongObservation)?;
        let (bytes, items) = attempt.payload.decode().map_err(|_| Failure::FrameDecode)?;
        check(
            self.entries.len() < 96
                && !id.is_empty()
                && id.len() <= 128
                && attempt.service_id == SERVICE
                && attempt.machine_id == MACHINE
                && attempt.binding.selection.is_some()
                && attempt.binding.selection == self.target
                && items > 0
                && !self
                    .entries
                    .iter()
                    .any(|e| e.request_sha256 == String::from(digest.clone()))
                && self.seen.insert(id.clone()),
        )?;
        self.pending.insert(id, self.entries.len());
        self.entries.push(WireExport {
            signal: attempt.payload.signal,
            request_sha256: digest.into(),
            payload_sha256: sha256(&bytes),
            items,
            receipt: None,
            ack: None,
        });
        Ok(())
    }

    pub fn finish(&mut self, id: &str, receipt: Option<ExportReceipt>) -> Result<Ack, Failure> {
        let index = *self.pending.get(id).ok_or(Failure::WrongObservation)?;
        let receipt = receipt.ok_or(Failure::WrongObservation)?;
        let entry = &mut self.entries[index];
        check(
            entry.request_sha256 == String::from(receipt.request_digest)
                && entry.receipt.is_none()
                && match &receipt.outcome {
                    ExportOutcome::Partial { rejected_items } => {
                        *rejected_items > 0 && *rejected_items <= entry.items as u64
                    }
                    _ => true,
                },
        )?;
        let ack = std::mem::take(&mut self.next_ack);
        entry.receipt = Some(receipt.outcome);
        entry.ack = Some(ack);
        self.pending.remove(id);
        Ok(ack)
    }
}

#[test]
fn export_capture_correlates_receipts_and_refuses_replay_foreign_targets_and_payloadless_success() {
    let mut attempt = crate::machine_protocol::telemetry_export::fixture();
    attempt.service_id = SERVICE.into();
    attempt.machine_id = MACHINE.into();
    let (signal, body) = crate::otlp::client_fixtures().remove(0);
    attempt.payload = crate::otlp::Request::decode(signal, &body)
        .unwrap()
        .export();
    let mut wire = Wire::default();
    wire.target(attempt.binding.selection.clone().unwrap())
        .unwrap();
    wire.arm(Ack::Drop).unwrap();
    wire.begin("one".into(), attempt.clone()).unwrap();
    assert!(wire.begin("other".into(), attempt.clone()).is_err());
    assert!(wire.arm(Ack::Disconnect).is_err());
    assert!(wire.finish("foreign", None).is_err());
    assert!(wire.finish("one", None).is_err());
    let receipt = ExportReceipt {
        request_digest: attempt.request_digest().unwrap(),
        outcome: ExportOutcome::Delivered {},
    };
    assert_eq!(
        wire.finish("one", Some(receipt.clone())).unwrap(),
        Ack::Drop
    );
    assert!(wire.finish("one", Some(receipt)).is_err());
    attempt.attempt_id.push_str("-new");
    attempt.machine_id = "foreign".into();
    assert!(wire.begin("two".into(), attempt).is_err());
    assert_eq!(wire.entries.len(), 1);
}

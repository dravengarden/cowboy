//! Closed public projections. Never serialize a journal, Actor or authority.
use super::*;
use crate::machine_protocol::telemetry_binding::{BindingChange, BindingSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    AbortBeforeDispatch,
    AcceptApplied,
    RecordRejected,
}

impl From<&ResolutionAction> for Action {
    fn from(action: &ResolutionAction) -> Self {
        match action {
            ResolutionAction::AbortBeforeDispatch => Self::AbortBeforeDispatch,
            ResolutionAction::AcceptApplied { .. } => Self::AcceptApplied,
            ResolutionAction::RecordRejected { .. } => Self::RecordRejected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Dispatching,
    NeedsAttention,
    Aborted,
    Completed,
    Rejected,
}

impl From<&Progress> for Phase {
    fn from(progress: &Progress) -> Self {
        match progress {
            Progress::Prepared => Self::Prepared,
            Progress::Dispatching => Self::Dispatching,
            Progress::NeedsAttention { .. } => Self::NeedsAttention,
            Progress::Aborted => Self::Aborted,
            Progress::Completed { .. } => Self::Completed,
            Progress::Rejected { .. } => Self::Rejected,
        }
    }
}

#[derive(serde::Serialize)]
pub(super) struct OperationView {
    operation_id: String,
    machine_id: String,
    operation_digest: BindingDigest,
    phase: Phase,
    attention: Option<Attention>,
    expected: Option<BindingSnapshot>,
    change: BindingChange,
}

impl OperationView {
    fn new(operation: &Operation) -> Result<Self> {
        Ok(Self {
            operation_id: operation.intent.operation_id.clone(),
            machine_id: operation.intent.machine_id.clone(),
            operation_digest: binding_digest(&serde_json::to_vec(operation)?),
            phase: Phase::from(&operation.progress),
            attention: match operation.progress {
                Progress::NeedsAttention { reason, .. } => Some(reason),
                _ => None,
            },
            expected: operation.intent.expected.clone(),
            change: operation.intent.change.clone(),
        })
    }
}

#[derive(serde::Serialize)]
pub(super) struct ReceiptView {
    schema: u16,
    resolution_id: String,
    operation_id: String,
    machine_id: String,
    action: Action,
    operation_digest: BindingDigest,
    phase: Phase,
    resolved_at_ms: i64,
}

impl From<&crate::telemetry_binding::resolution::ResolutionRecord> for ReceiptView {
    fn from(record: &crate::telemetry_binding::resolution::ResolutionRecord) -> Self {
        Self {
            schema: 1,
            resolution_id: record.intent.resolution_id.clone(),
            operation_id: record.intent.operation_id.clone(),
            machine_id: record.intent.machine_id.clone(),
            action: Action::from(&record.intent.action),
            operation_digest: record.intent.operation_digest.clone(),
            phase: Phase::from(&record.after),
            resolved_at_ms: record.resolved_at_ms,
        }
    }
}

#[derive(serde::Serialize)]
pub(super) struct PlanView {
    schema: u16,
    plan_id: String,
    action: Action,
    expires_at_ms: i64,
    confirmation_available: bool,
    operation: OperationView,
    result_phase: Phase,
    result_head: Option<BindingSnapshot>,
}

impl PlanView {
    pub(super) fn new(
        intent: &ResolutionIntent,
        before: &Operation,
        after: Progress,
        confirmation_available: bool,
    ) -> Result<Self> {
        let result_head = match &after {
            Progress::Aborted => before.intent.expected.clone(),
            Progress::Completed { observation } | Progress::Rejected { observation } => {
                let BindingObservation::Observed { snapshot } = observation else {
                    anyhow::bail!("missing verified observation");
                };
                snapshot.current.clone()
            }
            _ => anyhow::bail!("not a terminal resolution"),
        };
        Ok(Self {
            schema: 1,
            plan_id: intent.resolution_id.clone(),
            action: Action::from(&intent.action),
            expires_at_ms: intent.expires_at_ms,
            confirmation_available,
            operation: OperationView::new(before)?,
            result_phase: Phase::from(&after),
            result_head,
        })
    }
}

#[derive(serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum JournalView {
    Absent,
    Retained(Box<RetainedJournal>),
}

#[derive(serde::Serialize)]
struct RetainedJournal {
    current: Option<BindingSnapshot>,
    latest: OperationView,
    resolution: Option<ReceiptView>,
}

#[derive(serde::Serialize)]
pub(super) struct StatusView {
    schema: u16,
    resolution_admission: Admission,
    journal: JournalView,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Admission {
    Open,
    Closed,
}

impl StatusView {
    pub(super) fn new(ledger: Option<&Ledger>, enabled: bool) -> Result<Self> {
        Ok(Self {
            schema: 1,
            resolution_admission: if enabled {
                Admission::Open
            } else {
                Admission::Closed
            },
            journal: match ledger {
                None => JournalView::Absent,
                Some(ledger) => {
                    let latest = ledger.operations.last().context("missing operation")?;
                    JournalView::Retained(Box::new(RetainedJournal {
                        current: ledger.current.clone(),
                        latest: OperationView::new(latest)?,
                        resolution: ledger
                            .resolutions
                            .iter()
                            .find(|record| record.intent.operation_id == latest.intent.operation_id)
                            .map(ReceiptView::from),
                    }))
                }
            },
        })
    }
}

//! Bounded diagnostic metadata for the existing runtime IPC, never ACP data.
//! Invalid or unsupported metadata is ignored without rejecting its command.
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};

pub(crate) const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
pub(crate) const MAX_SPANS_PER_FRAME: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceContext {
    pub traceparent: String,
    /// Controller-generated correlation nonce, not a browser identity claim.
    pub correlation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    ControllerDispatch,
    ControllerQueue,
    ControllerDelivery,
    MachineDispatch,
    WorkerQueue,
    WorkerPrompt,
    WorkerFirstOutput,
}

impl Stage {
    #[cfg(feature = "full")]
    pub(crate) fn service(self) -> &'static str {
        match self {
            Self::ControllerDispatch | Self::ControllerQueue | Self::ControllerDelivery => {
                "cowboy-controller"
            }
            Self::MachineDispatch => "cowboy-machine",
            Self::WorkerQueue | Self::WorkerPrompt | Self::WorkerFirstOutput => "cowboy-worker",
        }
    }

    #[cfg(feature = "full")]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::ControllerDispatch => "cowboy.controller.dispatch",
            Self::ControllerQueue => "cowboy.controller.queue",
            Self::ControllerDelivery => "cowboy.controller.delivery",
            Self::MachineDispatch => "cowboy.machine.dispatch",
            Self::WorkerQueue => "cowboy.worker.queue",
            Self::WorkerPrompt => "cowboy.worker.prompt",
            Self::WorkerFirstOutput => "cowboy.worker.first_output",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanRecord {
    pub context: TraceContext,
    pub parent_span_id: String,
    pub stage: Stage,
    pub started_ns: u64,
    pub duration_ns: u64,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceCarrier {
    pub context: TraceContext,
    /// A Machine handoff measurement travels with its command and is returned
    /// in the worker's existing sequenced event envelope, not the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_dispatch: Option<SpanRecord>,
}

pub(crate) fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| u64::try_from(v.as_nanos()).ok())
        .unwrap_or(0)
}

fn hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        && value.bytes().any(|v| v != b'0')
}

/// Only sampled W3C v00 contexts are retained. No baggage, tracestate, env
/// injection or global task-local span state crosses the process boundary.
pub(crate) fn valid_parent(parent: &str) -> bool {
    parent.is_ascii()
        && parent.len() == 55
        && parent.starts_with("00-")
        && parent.as_bytes()[35] == b'-'
        && parent.as_bytes()[52] == b'-'
        && hex(&parent[3..35], 16)
        && hex(&parent[36..52], 8)
        && parent[53..55]
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        && u8::from_str_radix(&parent[53..55], 16).is_ok_and(|v| v & 1 == 1)
}

impl TraceContext {
    #[cfg(feature = "full")]
    pub(crate) fn from_browser(parent: &str) -> Option<Self> {
        valid_parent(parent).then(|| Self {
            traceparent: parent.to_owned(),
            correlation_id: uuid::Uuid::new_v4().simple().to_string(),
        })
    }

    pub(crate) fn valid(&self) -> bool {
        valid_parent(&self.traceparent) && hex(&self.correlation_id, 16)
    }

    pub(crate) fn trace_id(&self) -> &str {
        &self.traceparent[3..35]
    }
    pub(crate) fn span_id(&self) -> &str {
        &self.traceparent[36..52]
    }
}

impl TraceCarrier {
    #[cfg(feature = "full")]
    pub(crate) fn new(context: TraceContext) -> Self {
        Self {
            context,
            machine_dispatch: None,
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.context.valid()
            && self.machine_dispatch.as_ref().is_none_or(|span| {
                span.valid() && span.stage == Stage::MachineDispatch && span.context == self.context
            })
    }
}

impl SpanRecord {
    pub(crate) fn valid(&self) -> bool {
        self.context.valid()
            && hex(&self.parent_span_id, 8)
            && self.parent_span_id != self.context.span_id()
            && self.started_ns > 0
            && self.duration_ns <= u64::try_from(MAX_AGE.as_nanos()).unwrap()
            && self.started_ns.checked_add(self.duration_ns).is_some()
    }
}

/// Uses a monotonic duration within one process. Wall-clock corrections or
/// clock skew between Machines must never be subtracted to infer latency.
#[derive(Debug, Clone)]
pub(crate) struct SpanTimer {
    pub(crate) record: SpanRecord,
    started: Instant,
}

impl SpanTimer {
    pub(crate) fn start(parent: &TraceContext, stage: Stage) -> Option<Self> {
        if !parent.valid() {
            return None;
        }
        let span_id = uuid::Uuid::new_v4().simple().to_string()[..16].to_owned();
        Some(Self {
            record: SpanRecord {
                context: TraceContext {
                    traceparent: format!("00-{}-{span_id}-01", parent.trace_id()),
                    correlation_id: parent.correlation_id.clone(),
                },
                parent_span_id: parent.span_id().to_owned(),
                stage,
                started_ns: now_ns(),
                duration_ns: 0,
                outcome: Outcome::Ok,
            },
            started: Instant::now(),
        })
    }

    pub(crate) fn expired(&self) -> bool {
        self.started.elapsed() > MAX_AGE
    }

    pub(crate) fn finish(mut self, outcome: Outcome) -> Option<SpanRecord> {
        self.record.duration_ns = u64::try_from(self.started.elapsed().as_nanos()).ok()?;
        self.record.outcome = outcome;
        self.record.valid().then_some(self.record)
    }
}

pub(crate) fn deserialize_carrier<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<TraceCarrier>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<TraceCarrier>(value)
        .ok()
        .filter(TraceCarrier::valid))
}

pub(crate) fn deserialize_spans<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Vec<SpanRecord>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    if !value
        .as_array()
        .is_some_and(|items| items.len() <= MAX_SPANS_PER_FRAME)
    {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_value::<Vec<SpanRecord>>(value)
        .ok()
        .filter(|spans| spans.iter().all(SpanRecord::valid))
        .unwrap_or_default())
}

#[cfg(feature = "machine-host")]
#[derive(Default)]
pub(crate) struct MachineSpans(std::collections::BTreeMap<(String, String), SpanTimer>);

#[cfg(feature = "machine-host")]
impl MachineSpans {
    pub(crate) fn start(&mut self, session: &str, command: &str, carrier: &TraceCarrier) {
        self.0.retain(|_, span| !span.expired());
        if session.len() > 128 || command.len() > 128 || self.0.len() >= 512 {
            return;
        }
        let key = (session.to_owned(), command.to_owned());
        if !self.0.contains_key(&key)
            && carrier.valid()
            && let Some(span) = SpanTimer::start(&carrier.context, Stage::MachineDispatch)
        {
            self.0.insert(key, span);
        }
    }

    pub(crate) fn dispatch(&mut self, session: &str, command: &str, carrier: &mut TraceCarrier) {
        let key = (session.to_owned(), command.to_owned());
        // A replay with different metadata cannot consume or replace the
        // original queued command's timer, even if the command ID matches.
        if !carrier.valid()
            || !self.0.get(&key).is_some_and(|span| {
                span.record.context.correlation_id == carrier.context.correlation_id
                    && span.record.context.trace_id() == carrier.context.trace_id()
                    && span.record.parent_span_id == carrier.context.span_id()
            })
        {
            return;
        }
        if let Some(span) = self.0.remove(&key)
            && !span.expired()
            && let Some(record) = span.finish(Outcome::Ok)
        {
            carrier.context = record.context.clone();
            carrier.machine_dispatch = Some(record);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "machine-host")]
    fn machine_timers_are_bounded_and_replay_cannot_replace_original_context() {
        let original = TraceCarrier {
            context: TraceContext {
                traceparent: "00-11111111111111111111111111111111-2222222222222222-01".into(),
                correlation_id: "33333333333333333333333333333333".into(),
            },
            machine_dispatch: None,
        };
        let mut traces = MachineSpans::default();
        for i in 0..1000 {
            traces.start("s", &i.to_string(), &original);
        }
        assert_eq!(traces.0.len(), 512);
        let mut foreign = original.clone();
        foreign.context.correlation_id = "44444444444444444444444444444444".into();
        traces.start("s", "0", &foreign);
        traces.dispatch("s", "0", &mut foreign);
        assert!(foreign.machine_dispatch.is_none());
        assert_eq!(traces.0.len(), 512);
        let mut command = original.clone();
        traces.dispatch("s", "0", &mut command);
        let record = command.machine_dispatch.as_ref().unwrap();
        assert_eq!(record.parent_span_id, original.context.span_id());
        assert_eq!(
            record.context.correlation_id,
            original.context.correlation_id
        );
        assert!(command.valid());
        assert_eq!(traces.0.len(), 511);
        for span in traces.0.values_mut() {
            span.started = Instant::now() - MAX_AGE - Duration::from_secs(1);
        }
        traces.start("s", "fresh", &original);
        assert_eq!(traces.0.len(), 1);
    }

    #[test]
    fn sampled_w3c_context_is_closed_and_span_timing_is_monotonic() {
        let context = TraceContext {
            traceparent: "00-11111111111111111111111111111111-2222222222222222-01".into(),
            correlation_id: "33333333333333333333333333333333".into(),
        };
        let span = SpanTimer::start(&context, Stage::WorkerPrompt)
            .unwrap()
            .finish(Outcome::Ok)
            .unwrap();
        assert_eq!(span.context.trace_id(), context.trace_id());
        assert_eq!(span.parent_span_id, context.span_id());
        assert_ne!(span.context.span_id(), context.span_id());
        assert!(span.valid());
        for parent in [
            "",
            "00-11111111111111111111111111111111-2222222222222222-00",
            "00-00000000000000000000000000000000-2222222222222222-01",
            "00-11111111111111111111111111111111-2222222222222222-zz",
            "00-11111111111111111111111111111111-2222222222222222-0é",
        ] {
            assert!(!valid_parent(parent));
        }
    }
}

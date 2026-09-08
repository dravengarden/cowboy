//! Explicit ACP-turn instrumentation; never inspects prompt bodies or config.
use std::collections::{BTreeMap, VecDeque};

use crate::runtime_trace::{Outcome, SpanRecord, SpanTimer, Stage, TraceCarrier};
use crate::runtime_wire::{RuntimeEvent, WorkerState};

struct Active {
    cmid: Option<String>,
    prompt: SpanTimer,
    first_output: Option<SpanTimer>,
}

#[derive(Default)]
pub(crate) struct WorkerTelemetry {
    queued: BTreeMap<String, SpanTimer>,
    active: Option<Active>,
    completed: VecDeque<SpanRecord>,
}

impl WorkerTelemetry {
    fn push(&mut self, record: Option<SpanRecord>) {
        if self.completed.len() < crate::runtime_trace::MAX_SPANS_PER_FRAME
            && let Some(record) = record
        {
            self.completed.push_back(record);
        }
    }

    pub(crate) fn admit(&mut self, cmid: Option<&str>, trace: Option<TraceCarrier>) {
        self.queued.retain(|_, span| !span.expired());
        let (Some(cmid), Some(trace)) = (cmid, trace) else {
            return;
        };
        if cmid.len() > 128
            || !trace.valid()
            || self.queued.len() >= 32
            || self.queued.contains_key(cmid)
        {
            return;
        }
        let Some(queue) = SpanTimer::start(&trace.context, Stage::WorkerQueue) else {
            return;
        };
        self.push(trace.machine_dispatch);
        self.queued.insert(cmid.to_owned(), queue);
    }

    pub(crate) fn started(&mut self, cmid: Option<&str>) {
        // The ACP prompt lock serializes real starts. A missing context must
        // clear previous attribution as well, including an untraced turn.
        if let Some(active) = self.active.take() {
            self.push(active.prompt.finish(Outcome::Error));
        }
        let Some(queue) = cmid
            .and_then(|cmid| self.queued.remove(cmid))
            .filter(|span| !span.expired())
        else {
            return;
        };
        let prompt = SpanTimer::start(&queue.record.context, Stage::WorkerPrompt);
        self.push(queue.finish(Outcome::Ok));
        if let Some(prompt) = prompt {
            let first_output = SpanTimer::start(&prompt.record.context, Stage::WorkerFirstOutput);
            self.active = Some(Active {
                cmid: cmid.map(str::to_owned),
                prompt,
                first_output,
            });
        }
    }

    pub(crate) fn completed(&mut self, cmid: Option<&str>, reason: &str) {
        let outcome = if reason.eq_ignore_ascii_case("cancelled") {
            Outcome::Cancelled
        } else if reason.eq_ignore_ascii_case("error") {
            Outcome::Error
        } else {
            Outcome::Ok
        };
        if let Some(queue) = cmid.and_then(|id| self.queued.remove(id)) {
            self.push(queue.finish(outcome));
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.cmid.as_deref() == cmid)
            && let Some(active) = self.active.take()
        {
            // No first-output span is emitted when no output ever arrived.
            self.push(active.prompt.finish(outcome));
        }
    }

    pub(crate) fn event(&mut self, event: &RuntimeEvent) -> Vec<SpanRecord> {
        if matches!(event, RuntimeEvent::Update { update, cmid } if update["sessionUpdate"] == "agent_message_chunk"
            && cmid.as_deref().is_none_or(|id| self.active.as_ref().is_some_and(|active| active.cmid.as_deref() == Some(id))))
        {
            let first = self
                .active
                .as_mut()
                .and_then(|active| active.first_output.take());
            self.push(first.and_then(|span| span.finish(Outcome::Ok)));
        }
        if matches!(
            event,
            RuntimeEvent::Status {
                state: WorkerState::Exited | WorkerState::Crashed,
                ..
            }
        ) {
            if let Some(active) = self.active.take() {
                self.push(active.prompt.finish(Outcome::Error));
            }
            for (_, queued) in std::mem::take(&mut self.queued) {
                self.push(queued.finish(Outcome::Error));
            }
        }
        self.completed.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_trace::TraceContext;

    fn carrier() -> TraceCarrier {
        TraceCarrier::new(
            TraceContext::from_browser("00-11111111111111111111111111111111-2222222222222222-01")
                .unwrap(),
        )
    }
    fn output() -> RuntimeEvent {
        RuntimeEvent::Update {
            update: serde_json::json!({"sessionUpdate":"agent_message_chunk","content":{"text":"not telemetry data"}}),
            cmid: None,
        }
    }

    #[test]
    fn first_output_begins_at_actual_rpc_and_is_recorded_once_without_text() {
        let mut telemetry = WorkerTelemetry::default();
        telemetry.admit(Some("turn-a"), Some(carrier()));
        assert!(telemetry.event(&output()).is_empty()); // history/startup is not a turn
        telemetry.started(Some("turn-a"));
        let mut foreign = output();
        if let RuntimeEvent::Update { cmid, .. } = &mut foreign {
            *cmid = Some("foreign-turn".into());
        }
        let queued = telemetry.event(&foreign);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].stage, Stage::WorkerQueue);
        let records = telemetry.event(&output());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].stage, Stage::WorkerFirstOutput);
        let first_parent = records[0].parent_span_id.clone();
        assert!(
            !serde_json::to_string(&records)
                .unwrap()
                .contains("not telemetry data")
        );
        assert!(telemetry.event(&output()).is_empty());
        telemetry.completed(Some("foreign-turn"), "Error");
        assert!(telemetry.active.is_some());
        telemetry.completed(Some("turn-a"), "Cancelled");
        let records = telemetry.event(&RuntimeEvent::TurnEnded {
            turn_id: "a".into(),
            stop_reason: "Cancelled".into(),
        });
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].outcome, Outcome::Cancelled);
        assert_eq!(records[0].context.span_id(), first_parent);
        assert!(telemetry.active.is_none());
    }

    #[test]
    fn pre_rpc_cancel_exit_and_pending_limits_do_not_create_fake_output_spans() {
        let mut telemetry = WorkerTelemetry::default();
        for i in 0..100 {
            telemetry.admit(Some(&i.to_string()), Some(carrier()));
        }
        assert_eq!(telemetry.queued.len(), 32);
        telemetry.completed(Some("0"), "Cancelled");
        let records = telemetry.event(&output());
        assert_eq!(records[0].stage, Stage::WorkerQueue);
        assert_eq!(records[0].outcome, Outcome::Cancelled);
        telemetry.started(Some("1"));
        let records = telemetry.event(&RuntimeEvent::Status {
            state: WorkerState::Crashed,
            detail: Some("must not export".into()),
        });
        assert!(
            records
                .iter()
                .all(|span| span.stage != Stage::WorkerFirstOutput)
        );
        assert!(records.len() <= 4);
        assert!(telemetry.queued.is_empty());
        assert!(telemetry.active.is_none());
    }
}

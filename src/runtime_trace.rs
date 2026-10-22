//! Runtime tracing contracts live in `cowboy-wire`; Machine dispatch timing
//! stays here because it reports through the Controller log pipeline.

pub use cowboy_wire::runtime_trace::*;

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
            crate::logs::runtime_spans(session, std::slice::from_ref(&record));
            carrier.machine_dispatch = crate::logs::forward_runtime().then_some(record);
        }
    }
}

#[cfg(all(test, feature = "machine-host"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
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
}

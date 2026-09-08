//! Controller-owned, ephemeral correlation authority. Never persisted in the
//! queue, transcript, launch declaration, worker snapshot or Provider state.
use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use sha2::{Digest as _, Sha256};

use crate::runtime_trace::{
    MAX_AGE, Outcome, SpanRecord, SpanTimer, Stage, TraceCarrier, TraceContext,
};

struct Entry {
    session: String,
    cmid: String,
    machine: String,
    owner: String,
    parent: TraceContext,
    queue: Option<SpanTimer>,
    delivery: Option<SpanTimer>,
    seen: HashSet<String>,
    finished: bool,
    born: Instant,
}

#[derive(Default)]
pub(crate) struct RuntimeTelemetry {
    entries: BTreeMap<String, Entry>,
}

impl RuntimeTelemetry {
    fn prune(&mut self) {
        self.entries
            .retain(|_, entry| entry.born.elapsed() <= MAX_AGE);
    }

    pub(crate) fn bind(
        &mut self,
        principal: &str,
        machine: &str,
        session: &str,
        cmid: &str,
        parent: &TraceContext,
    ) {
        self.prune();
        if !parent.valid()
            || [machine, session, cmid]
                .iter()
                .any(|v| v.is_empty() || v.len() > 128)
            || self
                .entries
                .values()
                .any(|entry| entry.session == session && entry.cmid == cmid)
        {
            return;
        }
        if self.entries.len() >= 512 {
            // Retain recent completed correlations for dedup, but do not let
            // yesterday's successful turns exclude every new trace today.
            let oldest = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.finished && entry.delivery.is_none())
                .min_by_key(|(_, entry)| entry.born)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.entries.remove(&id);
            } else {
                return;
            }
        }
        let Some(queue) = SpanTimer::start(parent, Stage::ControllerQueue) else {
            return;
        };
        self.entries.insert(
            parent.correlation_id.clone(),
            Entry {
                session: session.into(),
                cmid: cmid.into(),
                machine: machine.into(),
                owner: owner_hash(principal),
                parent: parent.clone(),
                queue: Some(queue),
                delivery: None,
                seen: HashSet::new(),
                finished: false,
                born: Instant::now(),
            },
        );
    }

    pub(crate) fn dispatch(
        &mut self,
        session: &str,
        cmid: &str,
    ) -> Option<(String, Option<SpanRecord>, TraceCarrier)> {
        self.prune();
        let entry = self
            .entries
            .values_mut()
            .find(|entry| entry.session == session && entry.cmid == cmid)?;
        let queued = entry
            .queue
            .take()
            .and_then(|timer| timer.finish(Outcome::Ok));
        if let Some(record) = &queued {
            entry.parent = record.context.clone();
        }
        if entry.delivery.is_none() {
            entry.delivery = SpanTimer::start(&entry.parent, Stage::ControllerDelivery);
        }
        let context = entry.delivery.as_ref()?.record.context.clone();
        Some((entry.owner.clone(), queued, TraceCarrier::new(context)))
    }

    pub(crate) fn delivery(
        &mut self,
        context: &TraceContext,
        outcome: Outcome,
    ) -> Option<(String, SpanRecord)> {
        if !context.valid() {
            return None;
        }
        self.prune();
        let entry = self.entries.get_mut(&context.correlation_id)?;
        if entry.delivery.as_ref()?.record.context != *context {
            return None;
        }
        Some((entry.owner.clone(), entry.delivery.take()?.finish(outcome)?))
    }

    pub(crate) fn accept(
        &mut self,
        machine: &str,
        session: &str,
        record: &SpanRecord,
    ) -> Option<String> {
        self.prune();
        if !record.valid()
            || !matches!(
                record.stage,
                Stage::MachineDispatch
                    | Stage::WorkerQueue
                    | Stage::WorkerPrompt
                    | Stage::WorkerFirstOutput
            )
        {
            return None;
        }
        let now = crate::runtime_trace::now_ns();
        if record.started_ns < now.saturating_sub(7 * 86_400_000_000_000)
            || record.started_ns + record.duration_ns > now.saturating_add(300_000_000_000)
        {
            return None;
        }
        let entry = self.entries.get_mut(&record.context.correlation_id)?;
        if entry.machine != machine
            || entry.session != session
            || entry.parent.trace_id() != record.context.trace_id()
            || entry.seen.len() >= 32
            || !entry.seen.insert(record.context.span_id().to_owned())
        {
            return None;
        }
        entry.finished |= record.stage == Stage::WorkerPrompt
            || (record.stage == Stage::WorkerQueue && record.outcome != Outcome::Ok);
        Some(entry.owner.clone())
    }
}

pub(crate) fn owner_hash(principal: &str) -> String {
    format!("{:x}", Sha256::digest(principal.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_is_bound_to_session_machine_and_original_submit_and_deduplicated() {
        let parent =
            TraceContext::from_browser("00-11111111111111111111111111111111-2222222222222222-01")
                .unwrap();
        let mut traces = RuntimeTelemetry::default();
        traces.bind("owner", "machine", "session", "cmid", &parent);
        let attacker = TraceContext::from_browser(&parent.traceparent).unwrap();
        traces.bind("other-owner", "machine", "session", "cmid", &attacker);
        let (owner, queued, carrier) = traces.dispatch("session", "cmid").unwrap();
        assert_eq!(owner, owner_hash("owner"));
        assert_eq!(
            queued.unwrap().context.correlation_id,
            parent.correlation_id
        );
        assert_eq!(traces.dispatch("session", "cmid").unwrap().2, carrier);
        let record = SpanTimer::start(&carrier.context, Stage::WorkerPrompt)
            .unwrap()
            .finish(Outcome::Ok)
            .unwrap();
        assert!(traces.accept("other-machine", "session", &record).is_none());
        assert!(traces.accept("machine", "other-session", &record).is_none());
        let mut forged = record.clone();
        forged.context.correlation_id = attacker.correlation_id;
        assert!(traces.accept("machine", "session", &forged).is_none());
        let mut forged = record.clone();
        forged.stage = Stage::ControllerDispatch;
        assert!(traces.accept("machine", "session", &forged).is_none());
        let mut forged = record.clone();
        forged.context.traceparent =
            "00-44444444444444444444444444444444-5555555555555555-01".into();
        assert!(traces.accept("machine", "session", &forged).is_none());
        let mut skewed = record.clone();
        skewed.started_ns = crate::runtime_trace::now_ns() + 600_000_000_000;
        assert!(traces.accept("machine", "session", &skewed).is_none());
        skewed.started_ns = crate::runtime_trace::now_ns() - 8 * 86_400_000_000_000;
        assert!(traces.accept("machine", "session", &skewed).is_none());
        assert_eq!(traces.accept("machine", "session", &record), Some(owner));
        assert!(traces.accept("machine", "session", &record).is_none());
        assert!(traces.delivery(&carrier.context, Outcome::Ok).is_some());
        assert!(traces.delivery(&carrier.context, Outcome::Ok).is_none());
        for _ in 1..32 {
            let record = SpanTimer::start(&carrier.context, Stage::WorkerPrompt)
                .unwrap()
                .finish(Outcome::Ok)
                .unwrap();
            assert!(traces.accept("machine", "session", &record).is_some());
        }
        let extra = SpanTimer::start(&carrier.context, Stage::WorkerPrompt)
            .unwrap()
            .finish(Outcome::Ok)
            .unwrap();
        assert!(traces.accept("machine", "session", &extra).is_none());
    }

    #[test]
    fn correlation_expiry_and_capacity_do_not_grow_with_session_lifetime() {
        let mut traces = RuntimeTelemetry::default();
        for i in 0..1000 {
            let parent = TraceContext::from_browser(
                "00-11111111111111111111111111111111-2222222222222222-01",
            )
            .unwrap();
            traces.bind("owner", "machine", "session", &i.to_string(), &parent);
        }
        assert_eq!(traces.entries.len(), 512);
        let (_, _, completed) = traces.dispatch("session", "0").unwrap();
        let record = SpanTimer::start(&completed.context, Stage::WorkerPrompt)
            .unwrap()
            .finish(Outcome::Ok)
            .unwrap();
        assert!(traces.accept("machine", "session", &record).is_some());
        assert!(traces.delivery(&completed.context, Outcome::Ok).is_some());
        let fresh = TraceContext::from_browser(&completed.context.traceparent).unwrap();
        traces.bind("owner", "machine", "session", "fresh", &fresh);
        assert_eq!(traces.entries.len(), 512);
        assert!(traces.dispatch("session", "0").is_none());
        assert!(traces.dispatch("session", "fresh").is_some());
        assert!(traces.accept("machine", "session", &record).is_none());
        for entry in traces.entries.values_mut() {
            entry.born = Instant::now() - MAX_AGE - std::time::Duration::from_secs(1);
        }
        assert!(traces.dispatch("session", "0").is_none());
        assert!(traces.entries.is_empty());
    }
}

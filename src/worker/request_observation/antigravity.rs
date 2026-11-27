//! Content-free per-turn diagnostic projection for Antigravity workers.
//!
//! `agy` runs inside `account_sandbox_v1` with `--disable-telemetry`. Its
//! signed proxy (`cowboy-antigravity-acp`) attaches bounded numeric backend
//! evidence (`antigravity_backend`, plus barrier-bounded
//! `antigravity_cache_diagnostics` from `evidence.mjs`) under
//! `PromptResponse._meta.cowboy` (and error `data.cowboy` on refusals).
//!
//! Because Antigravity reports turn-level ACP usage rather than 1:1 native
//! HTTP request usage, this recorder emits distinct
//! `cowboy.provider.turn_diagnostics` records (never `cowboy.provider.request_usage`)
//! so HTTP request counters and parent clocks stay exact.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;

use parking_lot::Mutex;
use serde_json::{Map, Value};

use super::{WorkerArgs, bounded, metadata};
use crate::agent_model::TurnUsage;
use crate::logs::{
    now_ms,
    wait_cache::{self, Fields, Tokens},
};

pub(super) struct Recorder {
    state: Mutex<State>,
}

struct State {
    session: String,
    epoch: String,
    release: String,
    launch_prefix_fingerprint: String,
    managed: bool,
    fresh: bool,
    epoch_first_start_ms: Option<i64>,
    epoch_saw_loss_before_first: bool,
    renewed_at: Option<i64>,
    compacted: VecDeque<i64>,
    compact_coverage: bool,
    pending_prompt_started_at_ms: Option<i64>,
    model: Option<String>,
    turn_seq: u64,
    dropped: u64,
    unmatched: u64,
    closed: bool,
    facts: metadata::Facts,
}

impl Recorder {
    pub(super) fn new(
        args: &WorkerArgs,
        epoch: &str,
        launch_prefix_fingerprint: String,
        facts: metadata::Facts,
    ) -> Self {
        let managed = args
            .execution_binding
            .as_ref()
            .and_then(crate::execution_environment::ExecutionBinding::managed)
            .is_some();
        Self {
            state: Mutex::new(State {
                session: args.session_id.clone(),
                epoch: epoch.into(),
                release: args.provider_version.clone(),
                launch_prefix_fingerprint,
                managed,
                fresh: args.resume.is_none() && args.native_fork.is_none(),
                epoch_first_start_ms: None,
                epoch_saw_loss_before_first: false,
                renewed_at: None,
                compacted: VecDeque::new(),
                compact_coverage: true,
                pending_prompt_started_at_ms: None,
                model: None,
                turn_seq: 0,
                dropped: 0,
                unmatched: 0,
                closed: false,
                facts,
            }),
        }
    }

    pub(super) fn prompt_started(&self, at_ms: i64) {
        let mut state = self.state.lock();
        if state.closed {
            return;
        }
        if state.pending_prompt_started_at_ms.replace(at_ms).is_some() {
            state.unmatched = state.unmatched.saturating_add(1);
            if state.epoch_first_start_ms.is_none() {
                state.epoch_saw_loss_before_first = true;
            }
        }
    }

    pub(super) fn set_model(&self, model: &str) {
        if let Some(clean) = bounded(Some(model)) {
            self.state.lock().model = Some(clean);
        }
    }

    pub(super) fn record_turn(
        &self,
        stop_reason: &str,
        usage: Option<TurnUsage>,
        meta: Option<&Map<String, Value>>,
    ) {
        let end_ms = now_ms();
        let mut state = self.state.lock();
        if state.closed {
            return;
        }
        state.sync_facts();
        state.turn_seq = state.turn_seq.saturating_add(1);
        let turn_seq = state.turn_seq;
        let had_start = state.pending_prompt_started_at_ms.take();
        let start_ms = had_start.unwrap_or(end_ms).min(end_ms);
        let role = if state.managed {
            "managed_child"
        } else {
            "parent"
        };
        let is_parent = role == "parent";
        let (first_rollover, first_compact) = if is_parent {
            let rollover = state.renewed_at.is_some_and(|start| start <= start_ms);
            if rollover {
                state.renewed_at = None;
            }
            state.fresh = false;
            let mut compact = false;
            while state.compacted.front().is_some_and(|at| *at <= start_ms) {
                state.compacted.pop_front();
                compact = true;
            }
            (
                Some(rollover),
                (compact || state.compact_coverage).then_some(compact),
            )
        } else {
            (None, None)
        };

        // Consistent epoch ordering across managed and unmanaged roots; unknown
        // when prompt start was missing or prior unmatched loss preceded first turn.
        let first_in_epoch = match (had_start, state.epoch_first_start_ms) {
            (Some(started), None) if started > 0 && !state.epoch_saw_loss_before_first => {
                state.epoch_first_start_ms = Some(started);
                Some(true)
            }
            (Some(started), Some(first)) if started >= first => Some(false),
            _ => None,
        };

        let outcome =
            if stop_reason.starts_with("error:") || stop_reason.eq_ignore_ascii_case("error") {
                "error"
            } else if stop_reason.eq_ignore_ascii_case("cancelled") {
                if usage.is_some() {
                    "usage"
                } else {
                    "incomplete"
                }
            } else if usage.is_some() {
                "usage"
            } else {
                "no_usage"
            };

        let cowboy = meta
            .and_then(|m| m.get("cowboy"))
            .and_then(Value::as_object);
        let dispatch_status = classify_dispatch_status(cowboy);

        let mut fields = state
            .common()
            .string("role", role)
            .string("outcome", outcome)
            .string("dispatch_status", dispatch_status)
            .string("start_source", "worker_prompt_start")
            .string("usage_source", "acp_prompt_response")
            .string("retention_tier", "unknown")
            .integer("request_started_at_ms", u64::try_from(start_ms).ok())
            .integer(
                "cowboy.duration_ms",
                u64::try_from(end_ms.saturating_sub(start_ms)).ok(),
            )
            .boolean("is_first_after_rollover", first_rollover)
            .boolean("is_first_after_compact", first_compact)
            .boolean("is_first_in_worker_epoch", first_in_epoch);

        if let Some(native_id) = state
            .facts
            .native_id
            .borrow()
            .as_deref()
            .and_then(|id| bounded(Some(id)))
        {
            fields = fields.string("native_session_id", &native_id);
        }
        if let Some(model) = state.model.as_deref() {
            fields = fields.string("model", model);
        }

        if let Some(cowboy) = cowboy {
            fields = append_proxy_diagnostics(fields, cowboy);
        }

        let tokens = usage.map_or_else(Tokens::default, |u| Tokens {
            input: Some(u.input_tokens),
            read: u.cached_input_tokens,
            write: u.cache_write_tokens,
            output: Some(u.output_tokens),
            ..Tokens::default()
        });
        let attempt_id = format!("turn:{turn_seq}");
        wait_cache::record(
            wait_cache::TURN_DIAGNOSTICS,
            &[&state.session, &state.epoch, &attempt_id],
            Some(start_ms),
            tokens.fields(fields, "normalized_total"),
        );
    }

    pub(super) fn emit_coverage(&self, outcome: &str) {
        let mut state = self.state.lock();
        state.sync_facts();
        if outcome == "closed" {
            state.closed = true;
            if state.pending_prompt_started_at_ms.take().is_some() {
                state.unmatched = state.unmatched.saturating_add(1);
            }
        }
        let dropped = std::mem::take(&mut state.dropped);
        let unmatched = std::mem::take(&mut state.unmatched);
        wait_cache::record(
            wait_cache::COVERAGE,
            &[],
            None,
            state
                .common()
                .string("outcome", outcome)
                .integer("producer_dropped", dropped)
                .integer("producer_unmatched", unmatched),
        );
    }
}

impl State {
    fn sync_facts(&mut self) {
        if self.fresh {
            self.renewed_at = self.facts.first_prompt.get().copied().flatten();
        }
        let mut dropped = 0;
        for _ in 0..metadata::COMPACTIONS {
            let Ok(at) = self.facts.compacted.try_recv() else {
                break;
            };
            if self.compacted.len() == metadata::COMPACTIONS {
                self.compacted.pop_front();
                dropped += 1;
            }
            self.compacted.push_back(at);
        }
        dropped += self.facts.dropped.swap(0, Ordering::Relaxed);
        if dropped != 0 {
            self.dropped += dropped;
            self.compact_coverage = false;
        }
    }

    fn common(&self) -> Fields {
        Fields::default()
            .string("cowboy.session.id", &self.session)
            .string("provider", "antigravity")
            .string("producer", "antigravity_proxy")
            .string("producer_release", &self.release)
            .string("launch_prefix_fingerprint", &self.launch_prefix_fingerprint)
            .integer("producer_schema_version", 1_u64)
    }
}

fn bounded_u64(map: &Map<String, Value>, key: &str, max: u64) -> Option<u64> {
    map.get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value <= max)
}

fn allowlisted_backend_class(class: &str) -> Option<&'static str> {
    match class {
        "ok" => Some("ok"),
        "not_dispatched" => Some("not_dispatched"),
        "rate_limited" => Some("rate_limited"),
        "verification_required" => Some("verification_required"),
        "location_unsupported" => Some("location_unsupported"),
        "unknown_refusal" => Some("unknown_refusal"),
        _ => None,
    }
}

fn classify_dispatch_status(cowboy: Option<&Map<String, Value>>) -> &'static str {
    let Some(cowboy) = cowboy else {
        return "unknown";
    };
    let backend = cowboy.get("antigravity_backend").and_then(Value::as_object);
    if backend.and_then(|b| b.get("class")).and_then(Value::as_str) == Some("not_dispatched") {
        return "not_dispatched";
    }
    let diag = cowboy
        .get("antigravity_cache_diagnostics")
        .and_then(Value::as_object)
        .filter(|d| d.get("schema").and_then(Value::as_u64) == Some(1));
    if diag
        .and_then(|d| bounded_u64(d, "backend_steps", 10_000))
        .is_some_and(|n| n > 0)
        || backend
            .and_then(|b| bounded_u64(b, "dispatches", 10_000))
            .is_some_and(|n| n > 0)
    {
        return "dispatched";
    }
    if backend
        .and_then(|b| b.get("barrier"))
        .and_then(Value::as_bool)
        == Some(true)
        && backend.and_then(|b| bounded_u64(b, "dispatches", 10_000)) == Some(0)
        && diag
            .and_then(|d| bounded_u64(d, "backend_steps", 10_000))
            .unwrap_or(0)
            == 0
    {
        return "not_dispatched";
    }
    "unknown"
}

fn append_proxy_diagnostics(mut fields: Fields, cowboy: &Map<String, Value>) -> Fields {
    const MAX_STEPS: u64 = 10_000;
    const MAX_MS: u64 = 86_400_000;
    const MAX_TOKENS: u64 = 9_007_199_254_740_991;

    if let Some(class) = cowboy
        .get("antigravity_backend")
        .and_then(Value::as_object)
        .and_then(|b| b.get("class"))
        .and_then(Value::as_str)
        .and_then(allowlisted_backend_class)
    {
        fields = fields.string("backend_class", class);
    }

    let diag = cowboy
        .get("antigravity_cache_diagnostics")
        .and_then(Value::as_object)
        .filter(|d| d.get("schema").and_then(Value::as_u64) == Some(1));

    // Only project barrier-bounded counts from typed cache diagnostics; never
    // fall back to un-bounded post-barrier classifier totals.
    if let Some(diag) = diag {
        fields = fields
            .integer(
                "backend_step_count",
                bounded_u64(diag, "backend_steps", MAX_STEPS),
            )
            .integer(
                "transient_retry_count",
                bounded_u64(diag, "transient_retries", MAX_STEPS),
            )
            .integer(
                "first_step_offset_ms",
                bounded_u64(diag, "first_step_offset_ms", MAX_MS),
            )
            .integer("step_span_ms", bounded_u64(diag, "step_span_ms", MAX_MS))
            .integer(
                "max_inter_step_ms",
                bounded_u64(diag, "max_inter_step_ms", MAX_MS),
            )
            .integer(
                "usage_update_count",
                bounded_u64(diag, "usage_updates", MAX_STEPS),
            )
            .integer(
                "start_context_used",
                bounded_u64(diag, "start_context_used", MAX_TOKENS),
            )
            .integer(
                "end_context_used",
                bounded_u64(diag, "end_context_used", MAX_TOKENS),
            )
            .integer(
                "context_size",
                bounded_u64(diag, "context_size", MAX_TOKENS),
            )
            .boolean(
                "had_subagents",
                diag.get("had_subagents").and_then(Value::as_bool),
            )
            .integer(
                "subagent_step_count",
                bounded_u64(diag, "subagent_steps", MAX_STEPS),
            );
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::any_value;

    fn extract_ints(fields: &Fields) -> std::collections::BTreeMap<String, i64> {
        let mut out = std::collections::BTreeMap::new();
        for kv in fields.as_slice() {
            if let Some(any_value::Value::IntValue(v)) =
                kv.value.as_ref().and_then(|a| a.value.as_ref())
            {
                out.insert(kv.key.clone(), *v);
            }
        }
        out
    }

    #[test]
    fn proxy_diagnostics_uses_only_barrier_bounded_counts_and_allowlisted_classes() {
        let without_diag = serde_json::json!({
            "antigravity_backend": { "class": "rate_limited", "successes": 4, "transient": 1 }
        });
        let fields = append_proxy_diagnostics(Fields::default(), without_diag.as_object().unwrap());
        let ints = extract_ints(&fields);
        assert!(!ints.contains_key("backend_step_count"));
        assert!(!ints.contains_key("transient_retry_count"));
        assert_eq!(
            classify_dispatch_status(without_diag.as_object()),
            "unknown"
        );

        let not_dispatched = serde_json::json!({
            "antigravity_backend": { "class": "not_dispatched", "barrier": true, "dispatches": 0 }
        });
        assert_eq!(
            classify_dispatch_status(not_dispatched.as_object()),
            "not_dispatched"
        );

        let full = serde_json::json!({
            "antigravity_backend": { "class": "ok", "successes": 9, "transient": 3, "barrier": true, "dispatches": 1 },
            "antigravity_cache_diagnostics": {
                "schema": 1,
                "backend_steps": 5,
                "transient_retries": 2,
                "first_step_offset_ms": 780,
                "step_span_ms": 19119,
                "max_inter_step_ms": 8460,
                "usage_updates": 4,
                "start_context_used": 42000,
                "end_context_used": 58000,
                "context_size": 1048576,
                "had_subagents": true,
                "subagent_steps": null
            }
        });
        assert_eq!(classify_dispatch_status(full.as_object()), "dispatched");
        let ints = extract_ints(&append_proxy_diagnostics(
            Fields::default(),
            full.as_object().unwrap(),
        ));
        assert_eq!(ints.get("backend_step_count"), Some(&5));
        assert_eq!(ints.get("transient_retry_count"), Some(&2));
        assert_eq!(ints.get("first_step_offset_ms"), Some(&780));
        assert_eq!(ints.get("step_span_ms"), Some(&19119));
        assert_eq!(ints.get("max_inter_step_ms"), Some(&8460));
        assert_eq!(ints.get("usage_update_count"), Some(&4));
        assert_eq!(ints.get("start_context_used"), Some(&42000));
        assert_eq!(ints.get("end_context_used"), Some(&58000));
        assert_eq!(ints.get("context_size"), Some(&1048576));
        assert!(!ints.contains_key("subagent_step_count"));
    }

    #[test]
    fn shutdown_with_pending_prompt_records_unmatched_and_managed_tracks_epoch_order() {
        let (mut args, _) = super::super::tests::fixture_input("antigravity");
        args.execution_binding = None;
        let (_obs, facts) = metadata::Observer::new(None);
        let rec = Recorder::new(&args, "ep-1", "0123456789abcdef".into(), facts);
        rec.prompt_started(1_700_000_000_000);
        // Second prompt_started before first finalized increments unmatched
        rec.prompt_started(1_700_000_001_000);
        assert_eq!(rec.state.lock().unmatched, 1);
        rec.emit_coverage("closed");
        assert!(rec.state.lock().closed);
        assert!(rec.state.lock().pending_prompt_started_at_ms.is_none());
        assert_eq!(rec.state.lock().unmatched, 0);
        // Late prompt_started / record_turn after close are fenced and do not advance turn_seq.
        rec.prompt_started(1_700_000_002_000);
        rec.record_turn("EndTurn", None, None);
        assert_eq!(rec.state.lock().turn_seq, 0);
    }
}

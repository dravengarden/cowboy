//! Content-free observations for wait/cache analysis. Identifiers in this module
//! are for the local log only. Metric export builds a separate projection.
use super::{capture, data::text};
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue, any_value};

pub const REQUEST: &str = "cowboy.provider.request_usage";
pub const TURN_DIAGNOSTICS: &str = "cowboy.provider.turn_diagnostics";
pub const DISPATCH: &str = "cowboy.wait.dispatch";
pub const SETTLED: &str = "cowboy.wait.settled";
pub const DECISION: &str = "cowboy.cache_opt.decision";
pub const RECEIPT: &str = "cowboy.wait.receipt";
pub const NOTICE: &str = "cowboy.wait.notice";
pub const RECONCILIATION: &str = "cowboy.wait.reconciliation";
pub const COVERAGE: &str = "cowboy.provider.request_coverage";
pub(super) const LOSS: &str = "cowboy.wait_cache.local_loss";
pub const DAY_MS: i64 = 86_400_000;

pub fn utc_day(at_ms: i64) -> i64 {
    at_ms.saturating_sub(at_ms.rem_euclid(DAY_MS))
}

#[derive(Default)]
pub struct Fields(pub(super) Vec<KeyValue>);

impl Fields {
    pub fn string(mut self, key: &str, value: &str) -> Self {
        // Native IDs and model names come from diagnostics. Never serialize a
        // URL, error, request, prompt, or arbitrary native attribute map.
        if value.len() <= 256 {
            self.0.push(text(key, value));
        }
        self
    }

    pub fn integer(mut self, key: &str, value: impl Into<Option<u64>>) -> Self {
        if let Some(value) = value.into().and_then(|value| i64::try_from(value).ok()) {
            self.0.push(scalar(key, any_value::Value::IntValue(value)));
        }
        self
    }

    pub fn boolean(mut self, key: &str, value: impl Into<Option<bool>>) -> Self {
        if let Some(value) = value.into() {
            self.0.push(scalar(key, any_value::Value::BoolValue(value)));
        }
        self
    }

    #[cfg(all(test, feature = "full"))]
    pub(crate) fn as_slice(&self) -> &[KeyValue] {
        &self.0
    }
}

fn scalar(key: &str, value: any_value::Value) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue { value: Some(value) }),
        ..Default::default()
    }
}

/// Stable per-event identity when the producer can provide it. Source time is
/// optional, and observed time remains the retention clock.
pub fn record(name: &'static str, identity: &[&str], source_ms: Option<i64>, fields: Fields) {
    capture::wait_cache_record(name, identity, source_ms, fields.0);
}

#[derive(Clone, Copy, Default)]
pub struct Tokens {
    pub input: Option<u64>,
    pub read: Option<u64>,
    pub write: Option<u64>,
    pub write_5m: Option<u64>,
    pub write_1h: Option<u64>,
    pub output: Option<u64>,
    /// Codex's native Responses conversion defaults absent cache details to
    /// zero. Preserve those raw values without assuming a measured miss.
    pub cache_zero_is_unknown: bool,
}

impl Tokens {
    /// Responses input already includes cached input. A provider-reported write
    /// count is evidence about those inputs and is never added to the total.
    /// Messages input excludes both cache reads and writes.
    pub fn fields(self, fields: Fields, semantics: &str) -> Fields {
        let measured =
            |value: Option<u64>| value.filter(|value| !self.cache_zero_is_unknown || *value != 0);
        let read = measured(self.read);
        let write = measured(self.write);
        let unsplit = write.and_then(|total| {
            total
                .checked_sub(self.write_5m.unwrap_or_default())?
                .checked_sub(self.write_1h.unwrap_or_default())
        });
        let uncached = match semantics {
            "messages" => self.input,
            "responses" => self
                .input
                .zip(read)
                .and_then(|(input, read)| input.checked_sub(read)),
            _ => None,
        };
        fields
            .string("input_semantics", semantics)
            .integer("input_tokens", self.input)
            .integer("cache_read_input_tokens", read)
            .integer("cache_write_input_tokens", write)
            .integer(
                "native_reported_cache_read_input_tokens",
                self.cache_zero_is_unknown.then_some(self.read).flatten(),
            )
            .integer(
                "native_reported_cache_write_input_tokens",
                self.cache_zero_is_unknown.then_some(self.write).flatten(),
            )
            .integer("cache_write_5m_input_tokens", self.write_5m)
            .integer("cache_write_1h_input_tokens", self.write_1h)
            .integer("cache_write_unknown_ttl_input_tokens", unsplit)
            .integer("uncached_input_tokens", uncached)
            .integer("output_tokens", self.output)
    }
}

/// Both keys and values used in aggregates/metrics have finite vocabularies.
pub fn provider(value: &str) -> &'static str {
    match value {
        "claude-code" => "claude-code",
        "codex" => "codex",
        "antigravity" => "antigravity",
        "claude-deepseek" => "claude-deepseek",
        "codex-deepseek" => "codex-deepseek",
        "deepseek-harness" => "deepseek-harness",
        "grok" => "grok",
        _ => "other",
    }
}

/// Resolve the logical Cowboy Provider for a gateway usage event. The shared
/// DeepSeek Anthropic gateway reports `producer_id = "claude-deepseek"` with
/// `agent = "deepseek-harness"` for DeepSeek Harness sessions.
pub fn gateway_provider(producer_id: &str, agent: &str) -> &'static str {
    if producer_id == "claude-deepseek" && agent == "deepseek-harness" {
        "deepseek-harness"
    } else {
        provider(producer_id)
    }
}

pub(super) fn outcome(value: &str) -> &'static str {
    match value {
        "usage" => "usage",
        "no_usage" => "no_usage",
        "error" => "error",
        "incomplete" => "incomplete",
        "admitted" => "admitted",
        "completed" => "completed",
        "failed" => "failed",
        "cancelled" => "cancelled",
        "unconfirmed" => "unconfirmed",
        "dispatch_pending" => "dispatch_pending",
        "enqueued" => "enqueued",
        "duplicate" => "duplicate",
        "draft_path" => "draft_path",
        "parent_unavailable" => "parent_unavailable",
        "attached" => "attached",
        "closed" => "closed",
        "partial" => "partial",
        "renewed" => "renewed",
        "preferences_unavailable" => "preferences_unavailable",
        "context_unavailable" => "context_unavailable",
        "no_preset" => "no_preset",
        "disabled" => "disabled",
        "cache_warm" => "cache_warm",
        "usage_unknown" => "usage_unknown",
        "small_context" => "small_context",
        "would_pass_preflight" => "would_pass_preflight",
        "not_eligible" => "not_eligible",
        "live_worker" => "live_worker",
        "context_pending" => "context_pending",
        "no_history" => "no_history",
        "record_failed" => "record_failed",
        "raced" => "raced",
        "notify_held" => "notify_held",
        "ok" => "ok",
        "observed" => "observed",
        "truncated" => "truncated",
        "unavailable" => "unavailable",
        _ => "unknown",
    }
}

#[cfg(feature = "full")]
pub fn record_notice_admission(session: &str, cmid: Option<&str>, outcome: &'static str) {
    let Some(call) =
        cmid.and_then(|cmid| cmid.strip_prefix(crate::agent_model::CALL_NOTICE_PREFIX))
    else {
        return;
    };
    record(
        NOTICE,
        &[session, call, outcome],
        None,
        Fields::default()
            .string("cowboy.session.id", session)
            .string("call_id", call)
            .string("outcome", outcome),
    );
}

pub fn role(value: &str) -> &'static str {
    match value {
        "parent" | "main" => "parent",
        "subagent" => "subagent",
        "managed_child" => "managed_child",
        "keepalive" => "keepalive",
        "auxiliary" => "auxiliary",
        _ => "unknown",
    }
}

#[cfg(feature = "full")]
pub fn record_notice_receipt(session: &str, turn_id: &str, stop_reason: &str) {
    let Some(call_id) = turn_id.strip_prefix(crate::agent_model::CALL_NOTICE_PREFIX) else {
        return;
    };
    let outcome = match stop_reason {
        "EndTurn" | "MaxTokens" | "MaxTurnRequests" => "completed",
        "Cancelled" => "cancelled",
        _ => "unconfirmed",
    };
    record(
        RECEIPT,
        &[session, call_id, outcome],
        None,
        Fields::default()
            .string("cowboy.session.id", session)
            .string("call_id", call_id)
            .string("outcome", outcome),
    );
}

/// The gateway commits one accounting envelope before projecting it here.
/// Its input field is already normalized to total input across wire dialects.
#[cfg(any(test, feature = "machine-host"))]
fn legacy_gateway_replay_producer(producer_id: &str) -> &'static str {
    match producer_id {
        "codex" => "codex",
        "claude-code" => "claude-code",
        "codex-deepseek" => "codex-deepseek",
        "claude-deepseek" => "claude-deepseek",
        _ => "other",
    }
}

#[cfg(feature = "machine-host")]
pub fn gateway_usage(id: &str, event: &crate::machine_protocol::ProviderUsageEvent) {
    capture::provider_usage(event);
    let provider = gateway_provider(&event.producer_id, &event.agent);
    let keepalive = event.request_purpose == "cache_keepalive";
    // Cache-protection decisions that did not make a provider request are
    // diagnostics about coverage, not billable attempts.
    if keepalive && event.status == 0 && event.duration_ms.is_none() {
        return;
    }
    let role = if keepalive {
        "keepalive"
    } else {
        role(&event.request_role)
    };
    let outcome = if event.status >= 400 {
        "error"
    } else if event.completed == Some(false) {
        "incomplete"
    } else if event.usage_observed == Some(true) || event.input_tokens.is_some() {
        "usage"
    } else {
        "no_usage"
    };
    let mut fields = Fields::default()
        .string("provider", provider)
        .string("producer", "gateway")
        .string("session_attribution", &event.session_attribution)
        .integer("producer_schema_version", u64::from(event.schema_version))
        .string("model", &event.model)
        .string("role", role)
        .string("outcome", outcome)
        .string("retention_tier", "unknown")
        .string("cache_observation", &event.cache_observation)
        .integer("http.response.status_code", u64::from(event.status))
        .integer("cache_miss_input_tokens", event.cache_miss_tokens)
        .integer("cowboy.duration_ms", event.duration_ms);
    if let Some(fingerprint) = event.session_fingerprint.as_deref() {
        fields = fields.string("session_fingerprint", fingerprint);
    }
    if let Some(build) = event.gateway_build.as_deref() {
        fields = fields.string("producer_release", build);
    }
    let started = event
        .duration_ms
        .and_then(|ms| u64::try_from(event.occurred_at_ms).ok()?.checked_sub(ms));
    // Preserve the exact pre-upgrade gateway replay producer bucket so
    // replayed spooled events (including deepseek-harness and former "other"
    // producers) keep identical rollup deduplication hashes.
    let replay_producer = legacy_gateway_replay_producer(&event.producer_id);
    record(
        REQUEST,
        &[replay_producer, id],
        Some(event.occurred_at_ms),
        Tokens {
            input: event.input_tokens,
            read: event.cache_hit_tokens,
            output: event.output_tokens,
            ..Tokens::default()
        }
        .fields(
            fields.integer("request_started_at_ms_inferred", started),
            "normalized_total",
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(tokens: Tokens, semantics: &str) -> serde_json::Value {
        let attrs = tokens.fields(Fields::default(), semantics).0;
        let mut result = serde_json::Map::new();
        for attr in attrs {
            if let Some(any_value::Value::IntValue(value)) =
                attr.value.and_then(|value| value.value)
            {
                result.insert(attr.key, value.into());
            }
        }
        result.into()
    }

    #[test]
    fn responses_read_is_a_subset_and_writes_do_not_change_input() {
        let tokens = Tokens {
            input: Some(100),
            read: Some(80),
            write: Some(10),
            ..Tokens::default()
        };
        let v = values(tokens, "responses");
        assert_eq!(v["input_tokens"], 100);
        assert_eq!(v["uncached_input_tokens"], 20);
        assert_eq!(v["cache_write_unknown_ttl_input_tokens"], 10);
        assert!(
            values(
                Tokens {
                    read: Some(101),
                    ..tokens
                },
                "responses"
            )["uncached_input_tokens"]
                .is_null()
        );
    }

    #[test]
    fn positive_messages_write_with_zero_split_remains_unknown() {
        let tokens = Tokens {
            input: Some(15),
            write: Some(100),
            write_5m: Some(0),
            write_1h: Some(0),
            ..Tokens::default()
        };
        let v = values(tokens, "messages");
        assert_eq!(v["uncached_input_tokens"], 15);
        assert_eq!(v["cache_write_unknown_ttl_input_tokens"], 100);
        assert_eq!(
            values(
                Tokens {
                    write_5m: Some(20),
                    write_1h: Some(67),
                    ..tokens
                },
                "messages"
            )["cache_write_unknown_ttl_input_tokens"],
            13
        );
    }

    #[test]
    fn partial_cache_write_split_only_counts_unattributed_remainder() {
        let partial = Tokens {
            write: Some(100),
            write_5m: Some(20),
            ..Tokens::default()
        };
        let v = values(partial, "messages");
        assert_eq!(v["cache_write_5m_input_tokens"], 20);
        assert!(v["cache_write_1h_input_tokens"].is_null());
        assert_eq!(v["cache_write_unknown_ttl_input_tokens"], 80);
        assert!(
            values(
                Tokens {
                    write_1h: Some(81),
                    ..partial
                },
                "messages"
            )["cache_write_unknown_ttl_input_tokens"]
                .is_null()
        );
    }

    #[test]
    fn provider_and_gateway_provider_preserve_all_seven_first_party_providers() {
        for name in [
            "claude-code",
            "codex",
            "antigravity",
            "claude-deepseek",
            "codex-deepseek",
            "deepseek-harness",
            "grok",
        ] {
            assert_eq!(provider(name), name);
        }
        assert_eq!(provider("unknown-plugin"), "other");
        assert_eq!(
            gateway_provider("claude-deepseek", "deepseek-harness"),
            "deepseek-harness"
        );
        assert_eq!(
            gateway_provider("claude-deepseek", "claude"),
            "claude-deepseek"
        );
        assert_eq!(
            gateway_provider("codex-deepseek", "codex"),
            "codex-deepseek"
        );
    }

    #[test]
    fn gateway_provider_maps_deepseek_harness_and_keeps_other_producers() {
        assert_eq!(
            gateway_provider("claude-deepseek", "deepseek-harness"),
            "deepseek-harness"
        );
        assert_eq!(
            gateway_provider("claude-deepseek", "claude-deepseek"),
            "claude-deepseek"
        );
        assert_eq!(
            legacy_gateway_replay_producer("claude-deepseek"),
            "claude-deepseek"
        );
        assert_eq!(legacy_gateway_replay_producer("antigravity"), "other");
        assert_eq!(legacy_gateway_replay_producer("custom-gw"), "other");
    }
}

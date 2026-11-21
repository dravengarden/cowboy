use super::*;

struct Details {
    thread: String,
    query: String,
    trace: Option<String>,
    client_request_id: Option<String>,
    end_ms: i64,
    tokens: Tokens,
    cost: Option<u64>,
    status: Option<u64>,
    overage: Option<bool>,
    retention: &'static str,
    sequence: Option<u64>,
    error: bool,
}

struct Span {
    thread: String,
    query: String,
    trace: String,
    id: String,
    interaction: Option<String>,
    model: Option<String>,
    client_request_id: Option<String>,
    end_ms: i64,
    outcome: &'static str,
    tokens: Tokens,
    attempts: Vec<(u64, i64)>,
    arrived_ms: i64,
}

#[derive(Default)]
pub(super) struct Pending {
    logs: Vec<Details>,
    spans: Vec<Span>,
    recent_spans: VecDeque<(String, String)>,
    recent_logs: VecDeque<(String, u64, i64)>,
}

impl Pending {
    pub(super) fn ingest(&mut self, signal: &str, record: &Value, now: i64) -> u64 {
        if signal == "logs" {
            let Some(details) = details(record) else {
                return 0;
            };
            if let Some(sequence) = details.sequence
                && !first_seen(
                    &mut self.recent_logs,
                    (details.thread.clone(), sequence, details.end_ms),
                )
            {
                return 0;
            }
            if self.logs.len() >= MAX_PENDING {
                return 1;
            }
            self.logs.push(details);
        } else {
            let Some(span) = span(record, now) else {
                return 0;
            };
            if !first_seen(
                &mut self.recent_spans,
                (span.trace.clone(), span.id.clone()),
            ) {
                return 0;
            }
            if self.spans.len() >= MAX_PENDING {
                return span.attempts.len() as u64;
            }
            self.spans.push(span);
        }
        0
    }

    pub(super) fn drain(&mut self, now: i64, force: bool, unmatched: &mut u64) -> Vec<Attempt> {
        self.spans.sort_by_key(|span| span.end_ms);
        let mut result = Vec::new();
        let mut waiting = Vec::new();
        for span in std::mem::take(&mut self.spans) {
            if !force && now.saturating_sub(span.arrived_ms) < COALESCE_MS {
                waiting.push(span);
                continue;
            }
            // Prefer explicit native identity. The pinned fixture sometimes
            // omits it. A unique candidate with the same conversation, query
            // source and terminal edge can still supply cache splits and cost.
            let candidates = self
                .logs
                .iter()
                .enumerate()
                .filter(|(_, log)| span.matches(log));
            let mut candidates = candidates.map(|(index, _)| index);
            let first = candidates.next();
            let extra = match (first, candidates.next()) {
                (Some(index), None) => Some(self.logs.swap_remove(index)),
                _ => None,
            };
            if extra.is_none() && span.outcome == "usage" {
                *unmatched += 1;
            }
            result.extend(span.finish(extra.as_ref()));
        }
        self.spans = waiting;
        self.logs.retain(|log| {
            let keep = !force && now.saturating_sub(log.end_ms) < MAX_KEEP_MS;
            if !keep {
                *unmatched += 1;
            }
            keep
        });
        result
    }
}

fn first_seen<T: PartialEq>(recent: &mut VecDeque<T>, id: T) -> bool {
    if recent.contains(&id) {
        return false;
    }
    if recent.len() >= MAX_PENDING * 2 {
        recent.pop_front();
    }
    recent.push_back(id);
    true
}

impl Span {
    fn matches(&self, log: &Details) -> bool {
        if self.thread != log.thread || self.query != log.query {
            return false;
        }
        if let Some((span, log)) = self
            .client_request_id
            .as_ref()
            .zip(log.client_request_id.as_ref())
        {
            return span == log;
        }
        log.trace.as_ref().is_none_or(|trace| *trace == self.trace)
            && self.end_ms.abs_diff(log.end_ms) <= 30
    }

    fn finish(self, details: Option<&Details>) -> impl Iterator<Item = Attempt> {
        let last = self.attempts.len().saturating_sub(1);
        self.attempts
            .into_iter()
            .enumerate()
            .map(move |(position, (index, start))| {
                let terminal = position == last;
                // Only the final attempt owns the terminal usage. A subsequent
                // start proves a retry, but the previous status/usage may be absent.
                let supplemental = terminal.then_some(details).flatten();
                let tokens = if terminal {
                    supplemental.map_or(self.tokens, |log| merge_tokens(self.tokens, log.tokens))
                } else {
                    Tokens::default()
                };
                let outcome = if !terminal {
                    "incomplete"
                } else if self.outcome == "error" || supplemental.is_some_and(|log| log.error) {
                    "error"
                } else if has_usage(tokens) {
                    "usage"
                } else {
                    self.outcome
                };
                Attempt {
                    id: format!("{}:{}:{index}", self.trace, self.id),
                    thread: self.thread.clone(),
                    turn: self.interaction.clone(),
                    role: role(&self.query),
                    model: self.model.clone(),
                    native_release: None,
                    start_ms: start,
                    end_ms: if terminal { self.end_ms } else { start },
                    start_source: "native_attempt_span",
                    outcome,
                    tokens,
                    cost: supplemental.and_then(|log| log.cost),
                    status: supplemental.and_then(|log| log.status),
                    overage: supplemental.and_then(|log| log.overage),
                    retention: supplemental.map_or("unknown", |log| log.retention),
                    usage_source: match supplemental.filter(|log| has_usage(log.tokens)) {
                        Some(_) if has_usage(self.tokens) => "native_attempt_span_and_api_log",
                        Some(_) => "native_api_log",
                        None => "native_attempt_span",
                    },
                }
            })
    }
}

fn has_usage(tokens: Tokens) -> bool {
    [
        tokens.input,
        tokens.output,
        tokens.read,
        tokens.write,
        tokens.write_5m,
        tokens.write_1h,
    ]
    .iter()
    .any(Option::is_some)
}

fn merge_tokens(span: Tokens, log: Tokens) -> Tokens {
    Tokens {
        input: log.input.or(span.input),
        output: log.output.or(span.output),
        read: log.read.or(span.read),
        write: log.write.or(span.write),
        write_5m: log.write_5m.or(span.write_5m),
        write_1h: log.write_1h.or(span.write_1h),
        ..Tokens::default()
    }
}

fn role(query: &str) -> &'static str {
    match query {
        "sdk" | "repl_main_thread" => "parent",
        "compact"
        | "generate_session_title"
        | "prompt_suggestion"
        | "cache_touch"
        | "resume_cache_warm" => "auxiliary",
        value if value.starts_with("agent:") => "subagent",
        _ => "unknown",
    }
}

fn details(record: &Value) -> Option<Details> {
    let attrs = Attrs::of(record);
    if !matches!(attrs.text("event.name"), Some("api_request" | "api_error")) {
        return None;
    }
    let tokens = Tokens {
        input: attrs.count("input_tokens"),
        read: attrs.count("cache_read_tokens"),
        write: attrs.count("cache_creation_tokens"),
        write_5m: attrs.count("cache_creation_5m_tokens"),
        write_1h: attrs.count("cache_creation_1h_tokens"),
        output: attrs.count("output_tokens"),
        cache_zero_is_unknown: false,
    };
    let retention = match attrs.text("prompt_cache_ttl") {
        Some("5m") => "5m",
        Some("1h") => "1h",
        _ => "unknown",
    };
    Some(Details {
        thread: bounded(attrs.text("session.id"))?,
        query: bounded(attrs.text("query_source")).unwrap_or_default(),
        trace: bounded(record["traceId"].as_str()),
        client_request_id: bounded(attrs.text("client_request_id")),
        end_ms: millis(&record["timeUnixNano"])?,
        tokens,
        cost: attrs.count("cost_usd_micros"),
        status: attrs.count("status_code"),
        overage: attrs
            .boolean("isUsingOverage")
            .or_else(|| attrs.boolean("is_using_overage")),
        retention,
        sequence: attrs.count("event.sequence"),
        error: attrs.text("event.name") == Some("api_error"),
    })
}

fn span(record: &Value, now: i64) -> Option<Span> {
    if record["name"].as_str() != Some("claude_code.llm_request") {
        return None;
    }
    let attrs = Attrs::of(record);
    let mut attempts: Vec<_> = array(&record["events"])
        .filter_map(|event| {
            (event["name"].as_str() == Some("gen_ai.request.attempt")).then_some(())?;
            Some((
                Attrs::of(event).count("attempt")?,
                millis(&event["timeUnixNano"])?,
            ))
        })
        .take(65)
        .collect();
    if attempts.is_empty() || attempts.len() > 64 {
        return None;
    }
    attempts.sort_unstable();
    attempts.dedup_by_key(|(attempt, _)| *attempt);
    let tokens = Tokens {
        input: attrs.count("input_tokens"),
        read: attrs.count("cache_read_tokens"),
        write: attrs.count("cache_creation_tokens"),
        write_5m: attrs.count("cache_creation_5m_tokens"),
        write_1h: attrs.count("cache_creation_1h_tokens"),
        output: attrs.count("output_tokens"),
        ..Tokens::default()
    };
    let outcome = match attrs.boolean("success") {
        Some(true) if has_usage(tokens) => "usage",
        Some(true) => "no_usage",
        Some(false) => "error",
        None => "incomplete",
    };
    let query = bounded(
        attrs
            .text("query_source_safe")
            .or_else(|| attrs.text("query_source")),
    )
    .unwrap_or_default();
    let trace = bounded(record["traceId"].as_str())?;
    let interaction = match role(&query) {
        "parent" if attrs.text("llm_request.context") != Some("standalone") => Some(trace.clone()),
        _ => None,
    };
    Some(Span {
        thread: bounded(
            attrs
                .text("gen_ai.conversation.id")
                .or_else(|| attrs.text("session.id")),
        )?,
        query,
        trace,
        id: bounded(record["spanId"].as_str())?,
        interaction,
        model: bounded(attrs.text("model")),
        client_request_id: bounded(attrs.text("client_request_id")),
        end_ms: millis(&record["endTimeUnixNano"])?,
        outcome,
        tokens,
        attempts,
        arrived_ms: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_starts_are_distinct_and_only_terminal_gets_usage() {
        let span = Span {
            thread: "thread".into(),
            trace: "trace".into(),
            id: "span".into(),
            query: "sdk".into(),
            interaction: Some("turn".into()),
            model: None,
            client_request_id: None,
            end_ms: 20,
            outcome: "usage",
            tokens: Tokens {
                input: Some(10),
                ..Tokens::default()
            },
            attempts: vec![(1, 2), (2, 14)],
            arrived_ms: 21,
        };
        let result: Vec<_> = span.finish(None).collect();
        assert_eq!(result.len(), 2);
        assert_ne!(result[0].id, result[1].id);
        assert_eq!(result[0].start_ms, 2);
        assert_eq!(result[0].outcome, "incomplete");
        assert!(result[0].tokens.input.is_none());
        assert_eq!(result[1].start_ms, 14);
        assert_eq!(result[1].tokens.input, Some(10));
    }

    #[test]
    fn api_log_supplements_span_token_fields_and_native_errors() {
        let mut log = Details {
            thread: "thread".into(),
            query: "sdk".into(),
            trace: None,
            client_request_id: None,
            end_ms: 20,
            tokens: Tokens {
                write_5m: Some(20),
                ..Tokens::default()
            },
            cost: Some(1),
            status: Some(200),
            overage: None,
            retention: "unknown",
            sequence: None,
            error: false,
        };
        let finish = |tokens, outcome, log: &Details| {
            Span {
                thread: "thread".into(),
                trace: "trace".into(),
                id: "span".into(),
                query: "sdk".into(),
                interaction: Some("turn".into()),
                model: None,
                client_request_id: None,
                end_ms: 20,
                outcome,
                tokens,
                attempts: vec![(1, 2)],
                arrived_ms: 21,
            }
            .finish(Some(log))
            .next()
            .unwrap()
        };
        let tokens = Tokens {
            input: Some(10),
            output: Some(5),
            write: Some(100),
            ..Tokens::default()
        };
        let merged = finish(tokens, "usage", &log);
        assert_eq!(
            (merged.tokens.input, merged.tokens.output),
            (Some(10), Some(5))
        );
        assert_eq!(
            (merged.tokens.write, merged.tokens.write_5m),
            (Some(100), Some(20))
        );
        assert_eq!(merged.cost, Some(1));
        assert_eq!(merged.usage_source, "native_attempt_span_and_api_log");
        assert_eq!(finish(tokens, "error", &log).outcome, "error");
        assert_eq!(finish(Tokens::default(), "no_usage", &log).outcome, "usage");
        log.error = true;
        assert_eq!(finish(tokens, "usage", &log).outcome, "error");
    }
}

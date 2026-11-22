use super::*;
use std::collections::BTreeMap;

enum Event {
    Turn,
    Start {
        websocket: bool,
        compact: bool,
        failed: bool,
        status: Option<u64>,
        duration: u64,
    },
    StreamEnd {
        failed: bool,
        tokens: Option<Tokens>,
    },
}

struct Record {
    timestamp_ns: u64,
    thread: String,
    model: Option<String>,
    release: Option<String>,
    event: Event,
}

struct Open {
    attempt: Attempt,
    websocket: bool,
    terminal: Option<(i64, bool)>,
}

#[derive(Default)]
pub(super) struct Pending {
    records: BTreeMap<(u64, String, u8), Record>,
    open: HashMap<String, Open>,
    turns: HashMap<String, String>,
    // Transport logs have source timestamps; an OTLP retry can re-send an
    // entire batch. Older records cannot reopen an already reconciled stream.
    through_ms: i64,
}

impl Pending {
    pub(super) fn ingest(&mut self, record: &Value, _now: i64) -> u64 {
        let Some(record) = decode(record) else {
            return 0;
        };
        if record.time() <= self.through_ms {
            // Old native records may arrive after exporter backpressure.
            // Treat as coverage loss rather than consuming a later stream.
            return 1;
        }
        if self.records.len() >= MAX_PENDING {
            return 1;
        }
        let order = match record.event {
            Event::Turn => 0,
            Event::Start { .. } => 1,
            Event::StreamEnd { tokens: None, .. } => 2,
            Event::StreamEnd {
                tokens: Some(_), ..
            } => 3,
        };
        self.records
            .entry((record.timestamp_ns, record.thread.clone(), order))
            .or_insert(record);
        0
    }

    pub(super) fn drain(
        &mut self,
        now: i64,
        force: bool,
        native_root: Option<&str>,
        unmatched: &mut u64,
    ) -> Vec<Attempt> {
        let cutoff = if force {
            now
        } else {
            now.saturating_sub(COALESCE_MS)
        };
        let mut output = Vec::new();
        while let Some((_, record)) = self.records.first_key_value() {
            if !force && record.time() > cutoff {
                break;
            }
            let Some((_, record)) = self.records.pop_first() else {
                break;
            };
            self.through_ms = self.through_ms.max(record.time());
            self.apply(record, native_root, &mut output, unmatched);
        }
        self.open.retain(|_, open| {
            let expired = force
                || open.terminal.map_or_else(
                    || now.saturating_sub(open.attempt.end_ms) >= MAX_KEEP_MS,
                    |(end, _)| end <= cutoff.saturating_sub(COALESCE_MS),
                );
            if expired {
                output.push(finish_open(open));
            }
            !expired
        });
        output
    }

    fn apply(
        &mut self,
        record: Record,
        native_root: Option<&str>,
        output: &mut Vec<Attempt>,
        unmatched: &mut u64,
    ) {
        let time = record.time();
        let thread = record.thread;
        let root = native_root == Some(thread.as_str());
        match record.event {
            Event::Turn => {
                if self.turns.len() >= MAX_PENDING && !self.turns.contains_key(&thread) {
                    self.turns.clear();
                    *unmatched += 1;
                }
                self.turns.insert(thread, record.timestamp_ns.to_string());
            }
            Event::Start {
                websocket,
                compact,
                failed,
                status,
                duration,
            } => {
                if let Some(previous) = self.open.remove(&thread) {
                    output.push(finish_open(&previous));
                }
                let attempt = Attempt {
                    id: format!("{thread}:{}", record.timestamp_ns),
                    thread: thread.clone(),
                    turn: (!compact)
                        .then(|| self.turns.get(&thread))
                        .flatten()
                        .cloned(),
                    role: if compact {
                        "auxiliary"
                    } else if root && !websocket {
                        "parent"
                    } else {
                        "unknown"
                    },
                    model: record.model,
                    native_release: record.release,
                    start_ms: time
                        .saturating_sub(i64::try_from(duration).unwrap_or(i64::MAX))
                        .max(0),
                    end_ms: time,
                    start_source: "native_transport_duration",
                    outcome: if failed {
                        "error"
                    } else if compact {
                        "no_usage"
                    } else {
                        "incomplete"
                    },
                    tokens: Tokens::default(),
                    cost: None,
                    status,
                    overage: None,
                    retention: "unknown",
                    usage_source: "native_transport",
                };
                if failed || compact || self.open.len() >= MAX_PENDING {
                    output.push(attempt);
                } else {
                    self.open.insert(
                        thread,
                        Open {
                            attempt,
                            websocket,
                            terminal: None,
                        },
                    );
                }
            }
            Event::StreamEnd { failed, tokens } => {
                let Some(open) = self.open.get_mut(&thread) else {
                    if tokens.is_some() {
                        *unmatched += 1;
                    }
                    return;
                };
                if let Some(tokens) = tokens {
                    let Some(mut open) = self.open.remove(&thread) else {
                        return;
                    };
                    open.attempt.tokens = tokens;
                    open.attempt.outcome = "usage";
                    open.attempt.usage_source = "native_response_completed";
                    if root {
                        open.attempt.role = "parent";
                    }
                    open.attempt.end_ms = time;
                    output.push(open.attempt);
                } else if !open.websocket {
                    // WS OTLP only emits the final usage event. The bare SSE
                    // marker can arrive immediately before its usage record.
                    open.terminal = Some((time, failed));
                }
            }
        }
    }
}

fn finish_open(open: &Open) -> Attempt {
    let mut attempt = open.attempt.clone();
    if let Some((end, failed)) = open.terminal {
        attempt.end_ms = end;
        attempt.outcome = if failed { "incomplete" } else { "no_usage" };
    }
    // Successful websocket sends without associated usage include generate=false
    // prewarm and stream loss. They never advance the parent clock.
    if open.websocket {
        attempt.turn = None;
    }
    attempt
}

impl Record {
    fn time(&self) -> i64 {
        i64::try_from(self.timestamp_ns / 1_000_000).unwrap_or_default()
    }
}

fn decode(value: &Value) -> Option<Record> {
    let attrs = Attrs::of(value);
    let event = match attrs.text("event.name")? {
        "codex.user_prompt" => Event::Turn,
        "codex.api_request" => {
            let endpoint = attrs.text("endpoint")?;
            if !matches!(
                endpoint,
                "/responses" | "responses" | "/responses/compact" | "responses/compact"
            ) {
                return None;
            }
            let status = attrs.count("http.response.status_code");
            Event::Start {
                websocket: false,
                compact: endpoint.ends_with("/compact"),
                failed: status.is_none_or(|status| !(200..300).contains(&status))
                    || attrs.text("error.message").is_some(),
                status,
                duration: attrs.count("duration_ms")?,
            }
        }
        "codex.websocket_request" => Event::Start {
            websocket: true,
            compact: false,
            failed: attrs.boolean("success") != Some(true),
            duration: attrs.count("duration_ms")?,
            status: None,
        },
        "codex.sse_event"
            if matches!(
                attrs.text("event.kind"),
                Some("response.completed" | "response.failed" | "response.incomplete")
            ) =>
        {
            let tokens = Tokens {
                input: attrs.count("input_token_count"),
                read: attrs.count("cached_token_count"),
                write: attrs.count("cache_write_token_count"),
                output: attrs.count("output_token_count"),
                cache_zero_is_unknown: true,
                ..Tokens::default()
            };
            Event::StreamEnd {
                failed: attrs.text("event.kind") != Some("response.completed"),
                tokens: (tokens.input.is_some() || tokens.output.is_some()).then_some(tokens),
            }
        }
        _ => return None,
    };
    Some(Record {
        timestamp_ns: integer(&value["timeUnixNano"])?,
        thread: bounded(attrs.text("conversation.id"))?,
        model: bounded(attrs.text("model")),
        release: bounded(attrs.text("app.version")),
        event,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_fixture(time: u64, attributes: &[(&str, &str)]) -> Record {
        let mut attributes = attributes
            .iter()
            .map(|(key, value)| serde_json::json!({"key": key, "value": {"stringValue": value}}))
            .collect::<Vec<_>>();
        attributes.push(
            serde_json::json!({"key": "conversation.id", "value": {"stringValue": "native-root"}}),
        );
        decode(&serde_json::json!({"timeUnixNano":time * 1_000_000,"attributes":attributes}))
            .unwrap()
    }

    #[test]
    fn a_retry_and_a_completed_stream_get_one_record_each() {
        let mut state = Pending::default();
        let mut output = Vec::new();
        let mut unmatched = 0;
        let records = [
            decode_fixture(100, &[("event.name", "codex.user_prompt")]),
            decode_fixture(
                120,
                &[
                    ("event.name", "codex.api_request"),
                    ("endpoint", "/responses"),
                    ("http.response.status_code", "503"),
                    ("duration_ms", "20"),
                ],
            ),
            decode_fixture(
                150,
                &[
                    ("event.name", "codex.api_request"),
                    ("endpoint", "/responses"),
                    ("http.response.status_code", "200"),
                    ("duration_ms", "10"),
                ],
            ),
            decode_fixture(
                160,
                &[
                    ("event.name", "codex.sse_event"),
                    ("event.kind", "response.completed"),
                ],
            ),
            decode_fixture(
                160,
                &[
                    ("event.name", "codex.sse_event"),
                    ("event.kind", "response.completed"),
                    ("input_token_count", "10"),
                    ("cached_token_count", "8"),
                ],
            ),
        ];
        for record in records {
            state.apply(record, Some("native-root"), &mut output, &mut unmatched);
        }
        assert_eq!(unmatched, 0);
        assert_eq!(output.len(), 2);
        assert_eq!(output[0].outcome, "error");
        assert_eq!(output[1].start_ms, 140);
        assert_eq!(output[1].end_ms, 160);
        assert_eq!(output[1].tokens.read, Some(8));
        assert_eq!(output[0].turn, output[1].turn);
    }

    #[test]
    fn unpaired_websocket_send_does_not_advance_parent_clock() {
        let mut state = Pending::default();
        let mut output = Vec::new();
        let mut unmatched = 0;
        state.apply(
            decode_fixture(
                100,
                &[
                    ("event.name", "codex.websocket_request"),
                    ("success", "true"),
                    ("duration_ms", "2"),
                ],
            ),
            Some("native-root"),
            &mut output,
            &mut unmatched,
        );
        assert!(output.is_empty());
        let output = state.drain(200, true, Some("native-root"), &mut unmatched);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].role, "unknown");
        assert_eq!(output[0].outcome, "incomplete");
    }

    #[test]
    fn an_open_attempt_expires_once_even_without_shutdown() {
        let mut state = Pending::default();
        let mut output = Vec::new();
        let mut unmatched = 0;
        state.apply(
            decode_fixture(
                100,
                &[
                    ("event.name", "codex.websocket_request"),
                    ("success", "true"),
                    ("duration_ms", "2"),
                ],
            ),
            Some("native-root"),
            &mut output,
            &mut unmatched,
        );
        let drain = |state: &mut Pending, now, unmatched: &mut u64| {
            state.drain(now, false, Some("native-root"), unmatched)
        };
        assert!(drain(&mut state, 99 + MAX_KEEP_MS, &mut unmatched).is_empty());
        let expired = drain(&mut state, 100 + MAX_KEEP_MS, &mut unmatched);
        assert_eq!(expired.len(), 1);
        assert_eq!(
            (expired[0].role, expired[0].outcome),
            ("unknown", "incomplete")
        );
        state.apply(
            decode_fixture(
                u64::try_from(101 + MAX_KEEP_MS).unwrap(),
                &[
                    ("event.name", "codex.sse_event"),
                    ("event.kind", "response.completed"),
                    ("input_token_count", "10"),
                ],
            ),
            Some("native-root"),
            &mut output,
            &mut unmatched,
        );
        assert_eq!(unmatched, 1);
        assert!(output.is_empty());
        assert!(drain(&mut state, 102 + MAX_KEEP_MS, &mut unmatched).is_empty());
    }
}

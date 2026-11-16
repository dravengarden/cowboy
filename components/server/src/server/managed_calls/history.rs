//! Context asks (see `cowboy_wire::managed_calls::history`): export the named
//! sessions' recorded history as plain-text documents for the child's round,
//! and check the citations of its answer against the same rendering.
//!
//! The export is the whole authority the child receives. Visibility is the
//! parent's: same owner, not system, not closing, checked before and after
//! every storage read, exactly as `sessions.history` would allow the parent.
use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Value, json};

use super::AppState;
use crate::agent_model::Event;
use crate::core::{Envelope, Hub};
use crate::managed_calls::history::{
    ContextDocument, MAX_DOCUMENT_BYTES, SessionScope, valid_document_name,
};
use crate::managed_calls::{Request, lifecycle::Record};
use crate::store::Store;

/// Per-record character limits: what a reader needs to find and quote a
/// record, not whole tool outputs.
const USER_CHARS: usize = 8_000;
const ASSISTANT_CHARS: usize = 6_000;
const TOOL_CALL_CHARS: usize = 600;
const TOOL_RESULT_CHARS: usize = 1_500;
const REMINDER_CHARS: usize = 1_500;
/// Events read per storage page while exporting.
const EXPORT_PAGE: usize = 256;
/// Events one scope may scan; older history is reported as left out.
const MAX_SCANNED_EVENTS: usize = 200_000;
/// Room kept in each share for the document header and omission notice.
const HEADER_RESERVE: usize = 4096;
/// Transient failures the runner retries rather than failing the call.
pub(super) const RETRY: &str = "retry";
/// Citations checked per answer.
const MAX_CITATIONS: usize = 200;
const MAX_QUOTE_CHARS: usize = 400;

/// One rendered record: the seq it starts at, the last seq it covers, and
/// its text, already bounded by its kind's limit.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Rendered {
    pub seq: u64,
    pub end: u64,
    pub kind: &'static str,
    /// The streamed message id the record continues, if any.
    pub id: Option<String>,
    pub text: String,
}

fn limit(kind: &str) -> usize {
    match kind {
        "user" | "prompt" => USER_CHARS,
        "assistant" => ASSISTANT_CHARS,
        "reminder" => REMINDER_CHARS,
        "tool" => TOOL_CALL_CHARS,
        _ => TOOL_RESULT_CHARS,
    }
}

const TRUNCATED: &str = " … [truncated]";

/// Append to a record without ever holding more than its limit.
fn append(text: &mut String, separator: &str, more: &str, limit: usize) {
    if text.ends_with(TRUNCATED) {
        return;
    }
    text.push_str(separator);
    text.push_str(more);
    if text.chars().count() > limit {
        *text = bounded(text, limit);
    }
}

fn clipped(text: &str, limit: usize) -> String {
    bounded(text.trim(), limit)
}

/// At most `limit` characters, keeping whitespace so streamed chunks join.
fn bounded(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}{TRUNCATED}")
}

/// Text leaves of a tool result's content blocks, in order.
fn content_text(value: &Value, out: &mut String, limit: usize) {
    if out.len() > limit * 4 {
        return;
    }
    match value {
        Value::Object(fields) => {
            if let (Some(Value::String(kind)), Some(Value::String(path))) =
                (fields.get("type"), fields.get("path"))
                && kind == "diff"
            {
                let _ = writeln!(out, "diff {path}");
                return;
            }
            for (key, value) in fields {
                match (key.as_str(), value) {
                    ("text", Value::String(text)) => {
                        out.push_str(text);
                        out.push('\n');
                    }
                    (_, Value::Object(_) | Value::Array(_)) => content_text(value, out, limit),
                    _ => {}
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                content_text(value, out, limit);
            }
        }
        _ => {}
    }
}

/// A tool event's records: the attempt (title and input) for a new call, and
/// its outcome once a terminal status is recorded, both naming the call id.
fn tool_records(seq: u64, update: &Value, call: bool, records: &mut Vec<Rendered>) {
    let id = update
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or("?");
    if call {
        let mut text = format!("id={id} ");
        text.push_str(update.get("title").and_then(Value::as_str).unwrap_or(""));
        if let Some(input) = update.get("rawInput").filter(|input| !input.is_null()) {
            let _ = write!(text, " {input}");
        }
        records.push(Rendered {
            seq,
            end: seq,
            kind: "tool",
            id: None,
            text: clipped(&text, TOOL_CALL_CHARS),
        });
    }
    let status = update.get("status").and_then(Value::as_str);
    if !matches!(status, Some("completed" | "failed")) {
        return;
    }
    let mut text = format!("id={id} ");
    if let Some(content) = update.get("content") {
        content_text(content, &mut text, TOOL_RESULT_CHARS);
    }
    records.push(Rendered {
        seq,
        end: seq,
        kind: if status == Some("failed") {
            "tool_failed"
        } else {
            "tool_result"
        },
        id: None,
        text: clipped(&text, TOOL_RESULT_CHARS),
    });
}

/// Render a contiguous, ascending event slice. Streamed chunks of one
/// message become one record at the seq of their first chunk; thoughts,
/// usage and UI state are left out.
pub(super) fn render(events: &[Envelope]) -> Vec<Rendered> {
    let mut records: Vec<Rendered> = Vec::new();
    // Whether the last record may still receive streamed chunks.
    let mut open = false;
    for envelope in events {
        let Event::Update { update } = &envelope.event else {
            open = false;
            continue;
        };
        let kind = update.get("sessionUpdate").and_then(Value::as_str);
        match kind {
            Some("user_message_chunk" | "agent_message_chunk") => {
                let Some(text) = update.pointer("/content/text").and_then(Value::as_str) else {
                    continue;
                };
                let label = if kind == Some("agent_message_chunk") {
                    "assistant"
                } else if text.starts_with("<system-reminder") {
                    "reminder"
                } else if crate::prompt_origin::is_human_prompt_update(update) {
                    "user"
                } else {
                    "prompt"
                };
                let id = update
                    .get("messageId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let piece = Rendered {
                    seq: envelope.seq,
                    end: envelope.seq,
                    kind: label,
                    id,
                    text: bounded(text, limit(label)),
                };
                match records.last_mut() {
                    Some(last) if open && continues(last, &piece) => join(last, piece),
                    _ => records.push(piece),
                }
                open = true;
            }
            Some(tool @ ("tool_call" | "tool_call_update")) => {
                open = false;
                tool_records(envelope.seq, update, tool == "tool_call", &mut records);
            }
            Some(boundary @ ("context_cleared" | crate::context_rollover::MARKER)) => {
                open = false;
                records.push(Rendered {
                    seq: envelope.seq,
                    end: envelope.seq,
                    kind: "boundary",
                    id: None,
                    text: boundary.to_owned(),
                });
            }
            Some(
                "agent_thought_chunk"
                | "usage_update"
                | "available_commands_update"
                | "current_mode_update"
                | "config_option_update"
                | "session_info_update",
            ) => {}
            _ => open = false,
        }
    }
    records
}

/// Whether `next` continues the streamed message of `previous`.
fn continues(previous: &Rendered, next: &Rendered) -> bool {
    matches!(previous.kind, "user" | "prompt" | "reminder" | "assistant")
        && previous.kind == next.kind
        && (next.id.is_none() || previous.id == next.id)
}

fn join(previous: &mut Rendered, next: Rendered) {
    let separator = if previous.kind == "assistant" {
        ""
    } else {
        "\n"
    };
    append(
        &mut previous.text,
        separator,
        &next.text,
        limit(previous.kind),
    );
    previous.end = next.end;
}

/// One record as document lines. Continuation lines are indented, so text
/// inside a record can never start a forged `[seq]` record of its own.
fn record_line(record: &Rendered) -> String {
    let mut out = format!("[{}] {}: ", record.seq, record.kind);
    for (index, line) in record.text.lines().enumerate() {
        if index > 0 {
            out.push_str("\n    ");
        }
        out.push_str(line);
    }
    out.push('\n');
    out
}

fn visible(meta: &crate::core::SessionMeta, owner: Option<&str>) -> bool {
    !meta.system && !meta.closing && meta.owner_user_id.as_deref() == owner
}

fn check(hub: &Hub, owner: Option<&str>, id: &str) -> Result<(), &'static str> {
    hub.session_info(id)
        .filter(|info| visible(&info.meta, owner))
        .map(|_| ())
        .ok_or("history_unavailable")
}

/// One scope's records, newest first until `budget` bytes of document lines,
/// reading at most [`MAX_SCANNED_EVENTS`]. Only rendered, bounded records
/// are retained between pages. Also returns whether older in-scope history
/// was left out.
async fn export(
    hub: &Hub,
    store: Option<&Store>,
    owner: Option<&str>,
    scope: &SessionScope,
    budget: usize,
) -> Result<(Vec<Rendered>, bool), &'static str> {
    let id = scope.session_id.as_str();
    let allowed = |meta: &crate::core::SessionMeta| visible(meta, owner);
    check(hub, owner, id)?;
    let mut before = scope.before.unwrap_or(i64::MAX as u64);
    let mut records: std::collections::VecDeque<Rendered> = Default::default();
    let mut size = 0;
    let mut scanned = 0;
    loop {
        let (mut events, reached_start) = match store {
            Some(store) => {
                let (events, _, reached_start) = store
                    .history_page(id, before, EXPORT_PAGE)
                    .await
                    .map_err(|_| RETRY)?;
                (events, reached_start)
            }
            None => hub
                .log_window(id, None, Some(before), EXPORT_PAGE, allowed)
                .ok_or("history_unavailable")?,
        };
        // Recheck after storage I/O; a closed or transferred session's
        // archive must not be released by a stale check.
        check(hub, owner, id)?;
        let floor = events
            .iter()
            .position(|event| scope.after.is_none_or(|after| event.seq > after))
            .unwrap_or(events.len());
        let cut = floor > 0;
        events.drain(..floor);
        let Some(oldest) = events.first().map(|event| event.seq) else {
            break;
        };
        scanned += events.len();
        let mut page = render(&events);
        // A message streamed across the page boundary is one record.
        if let (Some(last), Some(first)) = (page.last_mut(), records.front())
            && last.end + 1 == first.seq
            && continues(last, first)
        {
            let first = records.pop_front().expect("front exists");
            size -= record_line(&first).len();
            join(last, first);
        }
        for record in page.into_iter().rev() {
            size += record_line(&record).len();
            records.push_front(record);
        }
        if size > budget || scanned >= MAX_SCANNED_EVENTS {
            // The oldest record may continue in an unread page; drop whole
            // records from the old end until the lines fit.
            records.pop_front();
            size = records.iter().map(|record| record_line(record).len()).sum();
            while size > budget {
                let Some(record) = records.pop_front() else {
                    break;
                };
                size -= record_line(&record).len();
            }
            return Ok((records.into(), true));
        }
        if cut || reached_start {
            break;
        }
        before = oldest;
    }
    Ok((records.into(), false))
}

fn share(scopes: &[SessionScope]) -> usize {
    MAX_DOCUMENT_BYTES / scopes.len().max(1) - HEADER_RESERVE
}

/// The documents of a context ask, one per scope, each within an equal share
/// of the round's byte bound. Every source is rechecked after the last read,
/// so no buffered history outlives a session that became invisible.
pub(super) async fn documents(
    hub: &Hub,
    store: Option<&Store>,
    owner: Option<&str>,
    scopes: &[SessionScope],
) -> Result<Vec<ContextDocument>, &'static str> {
    let mut documents = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let name = format!("{}.txt", scope.session_id);
        if !valid_document_name(&name) {
            return Err("history_unavailable");
        }
        let (records, omitted) = export(hub, store, owner, scope, share(scopes)).await?;
        let title = hub
            .session_info(&scope.session_id)
            .map(|info| info.meta.title)
            .unwrap_or_default();
        let mut text = format!(
            "# Cowboy session {} — {}\n# Records [after {}, before {}). One record per `[seq] kind:` line; indented lines continue it.\n",
            scope.session_id,
            clipped(&title, 200).replace(['\n', '\r'], " "),
            scope
                .after
                .map_or("start".into(), |after| after.to_string()),
            scope
                .before
                .map_or("now".into(), |before| before.to_string()),
        );
        if omitted {
            let _ = writeln!(
                text,
                "# Older records before seq {} were left out to fit the export bound.",
                records.first().map_or(0, |record| record.seq)
            );
        }
        for record in &records {
            text.push_str(&record_line(record));
        }
        documents.push(ContextDocument { name, text });
    }
    for scope in scopes {
        check(hub, owner, &scope.session_id)?;
    }
    Ok(documents)
}

/// A citation `[<session>#<seq>] "<quote>"` found in an answer.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Citation {
    pub session_id: String,
    pub seq: u64,
    pub quote: Option<String>,
}

pub(super) fn citations(answer: &str) -> Vec<Citation> {
    let mut found = Vec::new();
    let mut rest = answer;
    while let Some(start) = rest.find('[') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[..end];
        let after = &rest[end + 1..];
        let Some((session, seq)) = inner.split_once('#') else {
            continue;
        };
        let (Ok(seq), true) = (seq.parse::<u64>(), crate::managed_calls::valid_id(session)) else {
            continue;
        };
        let after = after.trim_start();
        let quote = after
            .strip_prefix('"')
            .or_else(|| after.strip_prefix('“'))
            .and_then(|tail| {
                let close = tail.find(['"', '”'])?;
                Some(tail[..close].to_owned())
            })
            .filter(|quote| !quote.trim().is_empty() && quote.chars().count() <= MAX_QUOTE_CHARS);
        found.push(Citation {
            session_id: session.to_owned(),
            seq,
            quote,
        });
        if found.len() == MAX_CITATIONS {
            break;
        }
    }
    found
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        // Answers often re-quote nested quotation marks in another style.
        .replace(['“', '”', '‘', '’', '\'', '「', '」'], "\"")
        .to_lowercase()
}

/// Check each citation against the exact records the child was given: the
/// same scopes and bounds re-exported, so a citation matches only a record
/// start that was in its document, with the quote inside that record
/// (whitespace- and case-insensitive).
pub(super) async fn verify(
    hub: &Hub,
    store: Option<&Store>,
    owner: Option<&str>,
    scopes: &[SessionScope],
    answer: &str,
) -> Value {
    let cited = citations(answer);
    let mut exported: BTreeMap<&str, Option<BTreeMap<u64, String>>> = BTreeMap::new();
    let mut verified = 0;
    let mut unverified = Vec::new();
    for citation in &cited {
        let reason = 'check: {
            let Some(scope) = scopes
                .iter()
                .find(|scope| scope.session_id == citation.session_id)
            else {
                break 'check Some("session_out_of_scope");
            };
            if !scope.contains(citation.seq) {
                break 'check Some("seq_out_of_scope");
            }
            if !exported.contains_key(scope.session_id.as_str()) {
                let records = export(hub, store, owner, scope, share(scopes))
                    .await
                    .ok()
                    .map(|(records, _)| {
                        records
                            .into_iter()
                            .map(|record| (record.seq, record.text))
                            .collect()
                    });
                exported.insert(scope.session_id.as_str(), records);
            }
            let Some(records) = exported
                .get(scope.session_id.as_str())
                .and_then(Option::as_ref)
            else {
                break 'check Some("history_unavailable");
            };
            let Some(text) = records.get(&citation.seq) else {
                break 'check Some("no_record_at_seq");
            };
            match &citation.quote {
                None => Some("no_quote"),
                Some(quote) if normalized(text).contains(&normalized(quote)) => None,
                Some(_) => Some("quote_not_found"),
            }
        };
        match reason {
            None => verified += 1,
            Some(reason) => unverified.push(json!({
                "cite": format!("{}#{}", citation.session_id, citation.seq),
                "reason": reason,
            })),
        }
    }
    json!({"checked": cited.len(), "verified": verified, "unverified": unverified})
}

/// Fix each scope's upper bound when the call is requested, so preparation
/// retries and citation checks read the same history while sessions keep
/// growing. A repeated operation reuses its stored bounds, so the request
/// still matches its original digest.
pub(in crate::server) async fn pin_bounds(
    state: &AppState,
    parent: &str,
    ledger: &crate::managed_calls::service::Ledger,
    request: &mut Request,
) {
    if request.context.sessions.is_empty() {
        return;
    }
    if let (Ok(Some(original)), Some(store)) = (
        ledger.observe(&request.request_id).await,
        state.store.as_ref(),
    ) && let Ok(Some(stored)) = store.managed_call_input(parent, &original.call_id).await
    {
        let same = |a: &SessionScope, b: &SessionScope| {
            a.session_id == b.session_id
                && a.after == b.after
                && a.before
                    .is_none_or(|before| b.before.is_some_and(|pinned| pinned <= before))
        };
        if stored.context.sessions.len() == request.context.sessions.len()
            && stored
                .context
                .sessions
                .iter()
                .zip(&request.context.sessions)
                .all(|(stored, requested)| same(requested, stored))
        {
            request.context.sessions = stored.context.sessions;
        }
        return;
    }
    for scope in &mut request.context.sessions {
        if let Some(last) = state.hub.last_event_seq(&scope.session_id) {
            let now = last.saturating_add(1);
            scope.before = Some(scope.before.map_or(now, |before| before.min(now)));
        }
    }
}

fn parent_owner(state: &AppState, record: &Record) -> Result<Option<String>, &'static str> {
    state
        .hub
        .session_info(&record.placement.parent_session_id)
        .map(|info| info.meta.owner_user_id)
        .ok_or("parent_unavailable")
}

/// The exported documents of a context ask's round, or none for any other
/// call. Its Machine must accept documents; an older Machine refuses the call
/// rather than silently capturing a worktree instead, while a disconnected one
/// is retried.
pub(super) async fn round_documents(
    state: &AppState,
    record: &Record,
    request: &Request,
) -> Result<Vec<ContextDocument>, &'static str> {
    if request.context.sessions.is_empty() {
        return Ok(Vec::new());
    }
    match state
        .machine_control
        .connection_protocol(&record.placement.machine_id)
    {
        None => return Err(RETRY),
        Some((_, protocol))
            if protocol < crate::machine_protocol::MANAGED_CONTEXT_PROTOCOL_VERSION =>
        {
            return Err("machine_upgrade_required");
        }
        Some(_) => {}
    }
    let owner = parent_owner(state, record)?;
    let started = std::time::Instant::now();
    let documents = documents(
        &state.hub,
        state.store.as_ref(),
        owner.as_deref(),
        &request.context.sessions,
    )
    .await?;
    // The third line is the export's own omission header, or the first record.
    let omitted = documents
        .iter()
        .filter(|document| {
            document
                .text
                .lines()
                .nth(2)
                .is_some_and(|line| line.starts_with("# Older records before seq"))
        })
        .count();
    tracing::info!(
        target: "cowboy::evidence",
        event_name = "cowboy.context_ask.export",
        session = %record.placement.parent_session_id,
        call_id = %record.call_id,
        ask_scopes = documents.len() as u64,
        bytes = documents.iter().map(|document| document.text.len() as u64).sum::<u64>(),
        ask_records = documents
            .iter()
            .map(|document| document.text.lines().filter(|line| line.starts_with('[')).count() as u64)
            .sum::<u64>(),
        ask_omitted = omitted as u64,
        duration_ms = started.elapsed().as_secs_f64() * 1000.0,
        "context ask history exported"
    );
    Ok(documents)
}

/// Attribution a caller needs to trust a result: a scoped review's input, and
/// a context ask's citation check.
pub(super) async fn annotate(
    state: &AppState,
    record: &Record,
    request: &Request,
    result: &mut Value,
) {
    if let Some(base) = request.labels.get(crate::managed_calls::review::BASE) {
        result["review_input"] = json!({
            "root": request.context.root,
            "base": base,
            "task": request.labels.get(crate::managed_calls::review::TASK),
        });
    }
    if !request.context.sessions.is_empty()
        && let Some(answer) = result["text"].as_str().map(str::to_owned)
        && let Ok(owner) = parent_owner(state, record)
    {
        result["citations"] = verify(
            &state.hub,
            state.store.as_ref(),
            owner.as_deref(),
            &request.context.sessions,
            &answer,
        )
        .await;
        measure_answer(record, request, result, answer.len());
    }
}

/// One context ask's outcome, so `cowboy logs metrics` compares its cost and
/// citation accuracy with the direct reads it replaces. The durable call
/// record keeps the answer itself.
fn measure_answer(record: &Record, request: &Request, result: &Value, answer_bytes: usize) {
    let usage = &result["usage"];
    let tokens = |key: &str| usage[key].as_u64().unwrap_or_default();
    let citations = &result["citations"];
    let duration_ms = record.started_at_ms.map_or(0.0, |started| {
        (chrono::Utc::now().timestamp_millis() - started).max(0) as f64
    });
    tracing::info!(
        target: "cowboy::evidence",
        event_name = "cowboy.context_ask.answered",
        session = %record.placement.parent_session_id,
        call_id = %record.call_id,
        provider = %record.provider,
        call_preset = record.preset.as_deref().unwrap_or(""),
        ask_scopes = request.context.sessions.len() as u64,
        ask_stop_reason = result["stop_reason"].as_str().unwrap_or(""),
        ask_answer_bytes = answer_bytes as u64,
        input_tokens = tokens("input_tokens"),
        cached_input_tokens = tokens("cached_input_tokens"),
        output_tokens = tokens("output_tokens"),
        ask_citations_checked = citations["checked"].as_u64().unwrap_or_default(),
        ask_citations_verified = citations["verified"].as_u64().unwrap_or_default(),
        duration_ms,
        "context ask answered"
    );
}

#[cfg(test)]
mod tests;

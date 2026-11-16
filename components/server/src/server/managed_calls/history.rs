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
/// Citations checked per answer.
const MAX_CITATIONS: usize = 200;
const MAX_QUOTE_CHARS: usize = 400;
/// Events read before a cited seq to find where its record starts.
const RECORD_LEAD: u64 = 256;

/// One rendered record: the seq it starts at and its single-paragraph text.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Rendered {
    pub seq: u64,
    pub kind: &'static str,
    pub text: String,
}

fn clipped(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head} … [truncated]")
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

/// Render a contiguous, ascending event slice. Streamed chunks of one
/// message become one record at the seq of their first chunk; thoughts,
/// usage and UI state are left out.
pub(super) fn render(events: &[Envelope]) -> Vec<Rendered> {
    let mut records: Vec<Rendered> = Vec::new();
    // The open streamed record: (kind, message id) it continues.
    let mut open: Option<(&'static str, Option<String>)> = None;
    for envelope in events {
        let Event::Update { update } = &envelope.event else {
            open = None;
            continue;
        };
        let kind = update.get("sessionUpdate").and_then(Value::as_str);
        let text = update.pointer("/content/text").and_then(Value::as_str);
        match kind {
            Some("user_message_chunk" | "agent_message_chunk") => {
                let Some(text) = text else { continue };
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
                let continues = open.as_ref().is_some_and(|(open_kind, open_id)| {
                    *open_kind == label && (id.is_none() || *open_id == id)
                });
                match records.last_mut() {
                    Some(last) if continues => {
                        if label == "user" || label == "prompt" {
                            last.text.push('\n');
                        }
                        last.text.push_str(text);
                    }
                    _ => records.push(Rendered {
                        seq: envelope.seq,
                        kind: label,
                        text: text.to_owned(),
                    }),
                }
                open = Some((label, id));
            }
            Some("tool_call") => {
                open = None;
                let title = update.get("title").and_then(Value::as_str).unwrap_or("");
                let mut text = title.to_owned();
                if let Some(input) = update.get("rawInput").filter(|input| !input.is_null()) {
                    let _ = write!(text, " {input}");
                }
                records.push(Rendered {
                    seq: envelope.seq,
                    kind: "tool",
                    text: clipped(&text, TOOL_CALL_CHARS),
                });
            }
            Some("tool_call_update") => {
                open = None;
                let status = update.get("status").and_then(Value::as_str);
                if !matches!(status, Some("completed" | "failed")) {
                    continue;
                }
                let mut text = String::new();
                if let Some(content) = update.get("content") {
                    content_text(content, &mut text, TOOL_RESULT_CHARS);
                }
                records.push(Rendered {
                    seq: envelope.seq,
                    kind: if status == Some("failed") {
                        "tool_failed"
                    } else {
                        "tool_result"
                    },
                    text: clipped(&text, TOOL_RESULT_CHARS),
                });
            }
            Some(boundary @ ("context_cleared" | crate::context_rollover::MARKER)) => {
                open = None;
                records.push(Rendered {
                    seq: envelope.seq,
                    kind: "boundary",
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
            _ => open = None,
        }
    }
    for record in &mut records {
        let limit = match record.kind {
            "user" | "prompt" => USER_CHARS,
            "assistant" => ASSISTANT_CHARS,
            "reminder" => REMINDER_CHARS,
            _ => continue,
        };
        record.text = clipped(&record.text, limit);
    }
    records
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

/// Ascending events of one scope, newest pages first until `budget` bytes
/// of rendered text, plus whether older in-scope history was left out.
async fn scope_events(
    hub: &Hub,
    store: Option<&Store>,
    owner: Option<&str>,
    scope: &SessionScope,
    budget: usize,
) -> Result<(Vec<Envelope>, bool), &'static str> {
    let id = scope.session_id.as_str();
    let allowed = |meta: &crate::core::SessionMeta| visible(meta, owner);
    let check = || {
        hub.session_info(id)
            .filter(|info| allowed(&info.meta))
            .map(|_| ())
            .ok_or("history_unavailable")
    };
    check()?;
    let mut before = scope.before.unwrap_or(i64::MAX as u64);
    let mut pages: Vec<Vec<Envelope>> = Vec::new();
    let mut size = 0;
    loop {
        let (mut events, reached_start) = match store {
            Some(store) => {
                let (events, _, reached_start) = store
                    .history_page(id, before, crate::core::HISTORY_PAGE)
                    .await
                    .map_err(|_| "history_unreadable")?;
                (events, reached_start)
            }
            None => hub
                .log_window(id, None, Some(before), crate::core::HISTORY_PAGE, allowed)
                .ok_or("history_unavailable")?,
        };
        // Recheck after storage I/O; a closed or transferred session's
        // archive must not be released by a stale check.
        check()?;
        let floor = events
            .iter()
            .position(|event| scope.after.is_none_or(|after| event.seq > after))
            .unwrap_or(events.len());
        let cut = floor > 0;
        events.drain(..floor);
        let Some(oldest) = events.first().map(|event| event.seq) else {
            break;
        };
        size += render(&events)
            .iter()
            .map(|record| record.text.len() + 16)
            .sum::<usize>();
        pages.push(events);
        if size > budget {
            let mut flat: Vec<Envelope> = pages.into_iter().rev().flatten().collect();
            // Drop whole oldest records until the rendering fits.
            while !flat.is_empty()
                && render(&flat)
                    .iter()
                    .map(|record| record.text.len() + 16)
                    .sum::<usize>()
                    > budget
            {
                let drop = (flat.len() / 8).max(1);
                flat.drain(..drop);
            }
            return Ok((flat, true));
        }
        if cut || reached_start {
            break;
        }
        before = oldest;
    }
    Ok((pages.into_iter().rev().flatten().collect(), false))
}

/// The documents of a context ask, one per scope, each within an equal share
/// of the round's byte bound.
pub(super) async fn documents(
    hub: &Hub,
    store: Option<&Store>,
    owner: Option<&str>,
    scopes: &[SessionScope],
) -> Result<Vec<ContextDocument>, &'static str> {
    let share = MAX_DOCUMENT_BYTES / scopes.len().max(1) - 4096;
    let mut documents = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let name = format!("{}.txt", scope.session_id);
        if !valid_document_name(&name) {
            return Err("history_unavailable");
        }
        let (events, omitted) = scope_events(hub, store, owner, scope, share).await?;
        let title = hub
            .session_info(&scope.session_id)
            .map(|info| info.meta.title)
            .unwrap_or_default();
        let mut text = format!(
            "# Cowboy session {} — {}\n# Records [after {}, before {}). One record per `[seq] kind:` line; indented lines continue it.\n",
            scope.session_id,
            clipped(&title, 200).replace('\n', " "),
            scope
                .after
                .map_or("start".into(), |after| after.to_string()),
            scope
                .before
                .map_or("now".into(), |before| before.to_string()),
        );
        let records = render(&events);
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
        // Rendering limits keep each record small; never exceed the share.
        if text.len() > share {
            let mut end = text.len() - share;
            while !text.is_char_boundary(end) {
                end += 1;
            }
            text.drain(..end);
        }
        documents.push(ContextDocument { name, text });
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
        .replace(['“', '”'], "\"")
        .to_lowercase()
}

/// Check each citation against the record it names, re-rendered from the
/// durable history: in scope, an existing record start, and a quote that
/// appears in that record (whitespace- and case-insensitive).
pub(super) async fn verify(
    hub: &Hub,
    store: Option<&Store>,
    owner: Option<&str>,
    scopes: &[SessionScope],
    answer: &str,
) -> Value {
    let cited = citations(answer);
    let mut records: BTreeMap<(String, u64), Option<String>> = BTreeMap::new();
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
            let key = (citation.session_id.clone(), citation.seq);
            if !records.contains_key(&key) {
                let text = record_at(hub, store, owner, scope, citation.seq).await;
                records.insert(key.clone(), text);
            }
            let Some(text) = records.get(&key).and_then(Option::as_deref) else {
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

/// The rendered record starting at `seq`, or `None` when no record starts
/// there or the history cannot be read.
async fn record_at(
    hub: &Hub,
    store: Option<&Store>,
    owner: Option<&str>,
    scope: &SessionScope,
    seq: u64,
) -> Option<String> {
    // A streamed record spans its chunks: read a little before its start, so
    // a cited mid-record chunk is not mistaken for a record start, and one
    // page after it.
    let lead = seq.checked_sub(RECORD_LEAD);
    let window = SessionScope {
        session_id: scope.session_id.clone(),
        after: match (lead, scope.after) {
            (Some(lead), Some(after)) => Some(lead.max(after)),
            (lead, after) => lead.or(after),
        },
        before: Some(
            seq.saturating_add(crate::core::HISTORY_PAGE as u64)
                .min(scope.before.unwrap_or(u64::MAX)),
        ),
    };
    let (events, _) = scope_events(hub, store, owner, &window, usize::MAX)
        .await
        .ok()?;
    render(&events)
        .into_iter()
        .find(|record| record.seq == seq)
        .map(|record| record.text)
}

fn parent_owner(state: &AppState, record: &Record) -> Result<Option<String>, &'static str> {
    state
        .hub
        .session_info(&record.placement.parent_session_id)
        .map(|info| info.meta.owner_user_id)
        .ok_or("parent_unavailable")
}

/// The exported documents of a context ask's round, or none for any other
/// call. Its Machine must accept documents; older Machines refuse the call
/// rather than silently capturing a worktree instead.
pub(super) async fn round_documents(
    state: &AppState,
    record: &Record,
    request: &Request,
) -> Result<Vec<ContextDocument>, &'static str> {
    if request.context.sessions.is_empty() {
        return Ok(Vec::new());
    }
    let supported = state
        .machine_control
        .connection_protocol(&record.placement.machine_id)
        .is_some_and(|(_, protocol)| {
            protocol >= crate::machine_protocol::MANAGED_CONTEXT_PROTOCOL_VERSION
        });
    if !supported {
        return Err("machine_upgrade_required");
    }
    let owner = parent_owner(state, record)?;
    documents(
        &state.hub,
        state.store.as_ref(),
        owner.as_deref(),
        &request.context.sessions,
    )
    .await
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
    }
}

#[cfg(test)]
mod tests;

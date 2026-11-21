//! Lossless, bounded access to one raw event behind a transcript summary.
//! Selection precedes serialization to the model; attachments stay references.
use super::*;

/// One transcript read (history, search or a raw event) by an agent. A
/// renewed session reading its own earlier context is recorded, refusals
/// included, so the on-demand cost and coverage of a renewal can be compared
/// with the cold resume it avoided (`cowboy.context_rollover.history_read`).
pub(super) async fn transcript(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    parent: &SessionMeta,
    op: &str,
    args: Value,
) -> Result<Value, String> {
    let started = std::time::Instant::now();
    let (namespace, method) = op.split_once('.').ok_or("unknown_operation")?;
    let other = namespace == "sessions";
    let target = if other {
        args.get("session_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
    } else {
        Some(parent.id.clone())
    };
    let request = ReadRequest::of(&args);
    let result = match method {
        "event" => read(hub, store, parent, other, &args).await,
        "search" => search(hub, store, parent, other, &args).await,
        _ => session_reads::history(hub, store, parent, other, args).await,
    };
    if target.as_deref() == Some(parent.id.as_str())
        && let Ok(context) = crate::session_context::read(hub, &parent.id)
        && let Some(rollover) = context.rollover
        && observed_rollover(hub, store, &parent.id, rollover.before).await
    {
        record_read(
            &parent.id,
            method,
            rollover.before,
            &request,
            &result,
            started,
        );
    }
    result
}

async fn observed_rollover(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    session: &str,
    before: u64,
) -> bool {
    if let Some(matches) = hub.observes_rollover_boundary(session, before) {
        return matches;
    }
    let Some(store) = store else {
        return false;
    };
    let lookup = store.latest_context_rollover(session);
    if let Ok(Ok(committed)) =
        tokio::time::timeout(std::time::Duration::from_millis(150), lookup).await
    {
        hub.restore_observed_rollover_boundary(session, committed, before)
    } else {
        false
    }
}

/// The bounded shape of a read request: sizes and positions, never the
/// query text, which can quote anything the session held.
#[derive(Default)]
pub(super) struct ReadRequest {
    pub(super) before: Option<u64>,
    pub(super) seq: Option<u64>,
    pub(super) limit: u64,
    pub(super) chars: u64,
    pub(super) query_chars: u64,
    pub(super) kinds: String,
}

impl ReadRequest {
    pub(super) fn of(args: &Value) -> Self {
        let number = |key: &str| args.get(key).and_then(Value::as_u64);
        let mut kinds: Vec<&str> = args
            .get("kinds")
            .and_then(Value::as_array)
            .map(|kinds| kinds.iter().filter_map(Value::as_str).take(8).collect())
            .unwrap_or_default();
        kinds.sort_unstable();
        Self {
            before: number("before"),
            seq: number("seq"),
            limit: number("limit").unwrap_or_default(),
            chars: number("chars").unwrap_or_default(),
            query_chars: args
                .get("query")
                .and_then(Value::as_str)
                .map_or(0, |query| query.chars().count() as u64),
            kinds: kinds
                .iter()
                .map(|kind| kind.chars().take(16).collect::<String>())
                .collect::<Vec<_>>()
                .join(","),
        }
    }
}

/// `depth` is how far behind the renewal boundary the read starts, so later
/// analysis can tell recent-tail reads from deep searches; `more` says the
/// answer left older in-range history unread.
fn record_read(
    session: &str,
    operation: &str,
    boundary: u64,
    request: &ReadRequest,
    result: &Result<Value, String>,
    started: std::time::Instant,
) {
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let start = request.seq.or(request.before).unwrap_or(boundary);
    let depth = boundary.saturating_sub(start.min(boundary));
    match result {
        Ok(value) => {
            let items = ["items", "matches"]
                .iter()
                .find_map(|key| value.get(*key).and_then(Value::as_array))
                .map_or(0, Vec::len);
            let more = value.get("more").and_then(Value::as_bool).unwrap_or(false)
                || value.get("next_before").is_some_and(|next| !next.is_null());
            // Read before the macro, whose expansion shadows `Value`.
            let scanned = value
                .get("scanned")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let reached_start = value
                .get("reached_start")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let source = value.get("source").and_then(Value::as_str).unwrap_or("");
            let bytes = serde_json::to_vec(value).map_or(0, |bytes| bytes.len() as u64);
            tracing::info!(
                target: "cowboy::evidence",
                event_name = "cowboy.context_rollover.history_read",
                session,
                operation,
                outcome = "ok",
                bytes,
                boundary,
                read_depth = depth,
                read_limit = request.limit,
                read_chars = request.chars,
                read_query_chars = request.query_chars,
                read_kinds = %request.kinds,
                read_items = items as u64,
                read_scanned = scanned,
                read_more = more,
                read_reached_start = reached_start,
                read_source = source,
                duration_ms,
                "renewed session read its earlier context"
            );
        }
        Err(error) => tracing::warn!(
            target: "cowboy::evidence",
            event_name = "cowboy.context_rollover.history_read",
            session,
            operation,
            // A code, never echoed input: `invalid_argument: <key> ...`.
            outcome = %error
                .split([':', ' '])
                .next()
                .unwrap_or("error")
                .chars()
                .take(40)
                .collect::<String>(),
            boundary,
            read_depth = depth,
            read_limit = request.limit,
            read_chars = request.chars,
            read_query_chars = request.query_chars,
            read_kinds = %request.kinds,
            duration_ms,
            "renewed session read of its earlier context refused"
        ),
    }
}

pub(super) async fn search(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    parent: &SessionMeta,
    other: bool,
    args: &Value,
) -> Result<Value, String> {
    only_keys(
        args,
        if other {
            &["session_id", "query", "before", "limit", "chars"]
        } else {
            &["query", "before", "limit", "chars"]
        },
    )?;
    let id = if other {
        arg_str(args, "session_id")?
    } else {
        &parent.id
    };
    let query = args["query"]
        .as_str()
        .filter(|q| !q.is_empty() && q.len() <= 256 && !q.contains('\0'))
        .ok_or("invalid_argument")?;
    let before = arg_u64(args, "before", i64::MAX as u64, i64::MAX as u64)?;
    let limit = page_limit(args, 20, 64)?;
    let chars = arg_chars(args, 200, 1000)? as usize;
    if chars < query.chars().count() {
        return Err("invalid_argument".into());
    }
    let visible = |meta: &SessionMeta| {
        !meta.system && !meta.closing && meta.owner_user_id == parent.owner_user_id
    };
    hub.session_info(id)
        .filter(|info| visible(&info.meta))
        .ok_or("not_found")?;
    let started = std::time::Instant::now();
    let source = if store.is_some() { "durable" } else { "memory" };
    let mut matches = Vec::new();
    let mut scanned = 0;
    let mut cursor = before;
    let mut reached_start = false;
    let mut exhausted = false;
    // Scan back page by page: one 64-record page covered under 1% of a long
    // session, so most searches came back empty and agents rarely paged on.
    'pages: while scanned < SEARCH_SCAN_EVENTS && started.elapsed() < SEARCH_SCAN_TIME {
        let page = SEARCH_PAGE.min(SEARCH_SCAN_EVENTS - scanned);
        let (events, start) = if let Some(store) = store {
            let (events, _, start) = store
                .history_page(id, cursor, page)
                .await
                .map_err(|_| "unavailable")?;
            (events, start)
        } else {
            hub.log_window(id, None, Some(cursor), page, visible)
                .ok_or("not_found")?
        };
        // Pages are also bounded by bytes, so a short page is not the start.
        reached_start = start;
        for event in events.iter().rev() {
            scanned += 1;
            cursor = event.seq;
            let value = serde_json::to_value(&event.event).map_err(|_| "unavailable")?;
            if let Some((path, text, offset)) = find_text(&value, query, chars, String::new()) {
                matches.push(json!({"seq":event.seq,"path":path,"text":text,"offset":offset,"revision":crate::managed_calls::canonical_digest(&value)}));
                if matches.len() == limit {
                    exhausted = start && event.seq == events[0].seq;
                    break 'pages;
                }
            }
        }
        if start || events.is_empty() {
            exhausted = true;
            break;
        }
    }
    hub.session_info(id)
        .filter(|info| visible(&info.meta))
        .ok_or("not_found")?;
    let more = !exhausted;
    Ok(
        json!({"matches":matches,"scanned":scanned,"next_before":if more {Some(cursor)} else {None},"reached_start":reached_start && !more,"source":source}),
    )
}

/// Records fetched per storage round trip while searching.
const SEARCH_PAGE: usize = 256;
/// Records one search call may scan before handing back `next_before`.
const SEARCH_SCAN_EVENTS: usize = 4_096;
/// Wall time one search call may spend scanning.
const SEARCH_SCAN_TIME: std::time::Duration = std::time::Duration::from_millis(1_500);

// Literal smart-case search of stored string values: a query without an
// uppercase letter ignores case. One match locates an event; it is not a claim
// that a streamed message is one stored string.
fn find_text(
    value: &Value,
    query: &str,
    chars: usize,
    path: String,
) -> Option<(String, String, usize)> {
    match value {
        Value::String(text) => {
            let found = find_smart_case(text, query)?;
            let context = 40.min(chars.saturating_sub(query.chars().count()) / 2);
            let offset = found.saturating_sub(context);
            Some((
                path,
                text.chars().skip(offset).take(chars).collect(),
                offset,
            ))
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(i, v)| find_text(v, query, chars, format!("{path}/{i}"))),
        Value::Object(items) => items.iter().find_map(|(k, v)| {
            find_text(
                v,
                query,
                chars,
                format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
            )
        }),
        _ => None,
    }
}

/// Unicode scalar offset of the first match of `query` in `text`.
fn find_smart_case(text: &str, query: &str) -> Option<usize> {
    if query.chars().any(char::is_uppercase) {
        let byte = text.find(query)?;
        return Some(text[..byte].chars().count());
    }
    if query.is_ascii() {
        // ASCII bytes never occur inside a multi-byte scalar, so a match
        // always starts on a character boundary.
        let byte = text
            .as_bytes()
            .windows(query.len())
            .position(|window| window.eq_ignore_ascii_case(query.as_bytes()))?;
        return Some(text[..byte].chars().count());
    }
    let folds = |c: char| c.to_lowercase();
    let query: Vec<char> = query.chars().flat_map(folds).collect();
    let text: Vec<(usize, char)> = text
        .chars()
        .enumerate()
        .flat_map(|(at, c)| folds(c).map(move |folded| (at, folded)))
        .collect();
    text.windows(query.len())
        .find(|window| window.iter().map(|(_, c)| *c).eq(query.iter().copied()))
        .map(|window| window[0].0)
}

pub(super) async fn read(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    parent: &SessionMeta,
    other: bool,
    args: &Value,
) -> Result<Value, String> {
    only_keys(
        args,
        if other {
            &[
                "session_id",
                "seq",
                "path",
                "offset",
                "limit",
                "chars",
                "describe",
                "expected",
            ]
        } else {
            &[
                "seq", "path", "offset", "limit", "chars", "describe", "expected",
            ]
        },
    )?;
    let id = if other {
        arg_str(args, "session_id")?
    } else {
        &parent.id
    };
    let seq = args["seq"]
        .as_u64()
        .filter(|seq| *seq < i64::MAX as u64)
        .ok_or("invalid_argument")?;
    let allowed = |meta: &SessionMeta| {
        !meta.system && !meta.closing && meta.owner_user_id == parent.owner_user_id
    };
    let (mut events, _) = hub
        .log_window(id, None, Some(seq + 1), 1, allowed)
        .ok_or("not_found")?;
    events.retain(|event| event.seq == seq);
    let mut source = "memory";
    if events.is_empty()
        && let Some(store) = store
    {
        events = store
            .history_page(id, seq + 1, 1)
            .await
            .map_err(|_| "unavailable")?
            .0;
        source = "durable";
    }
    // A close or ownership change during storage I/O must not release content.
    hub.session_info(id)
        .filter(|info| allowed(&info.meta))
        .ok_or("not_found")?;
    let event = events
        .into_iter()
        .find(|event| event.seq == seq)
        .ok_or("not_found")?;
    let value = serde_json::to_value(event.event).map_err(|_| "unavailable")?;
    let revision = crate::managed_calls::canonical_digest(&value);
    if args
        .get("expected")
        .is_some_and(|expected| expected.as_str() != Some(revision.as_str()))
    {
        return Err("revision_conflict".into());
    }
    let page = json_reads::select(&value, args)?;
    Ok(json!({"seq":seq,"source":source,"revision":revision,"page":page}))
}

#[cfg(test)]
use super::json_reads::select;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_search_excerpts_include_the_unicode_match() {
        let value = json!(format!(
            "{}中🙂needle{}",
            "前".repeat(100),
            "后".repeat(100)
        ));
        for chars in [8, 12, 20, 100] {
            let (_, text, offset) = find_text(&value, "中🙂needle", chars, String::new()).unwrap();
            assert!(text.contains("中🙂needle"));
            assert!(text.chars().count() <= chars);
            assert_eq!(
                text,
                value
                    .as_str()
                    .unwrap()
                    .chars()
                    .skip(offset)
                    .take(chars)
                    .collect::<String>()
            );
        }
    }

    #[test]
    fn search_ignores_case_only_for_a_lowercase_query() {
        let text = "前Ünïcode 中 ContextRollover failed";
        assert_eq!(find_smart_case(text, "contextrollover"), Some(11));
        assert_eq!(find_smart_case(text, "ContextRollover"), Some(11));
        assert_eq!(find_smart_case(text, "Contextrollover"), None);
        assert_eq!(find_smart_case(text, "ünïcode 中"), Some(1));
        assert_eq!(find_smart_case(text, "missing"), None);
    }

    #[test]
    fn event_selection_does_not_dump_large_tool_outputs_or_attachment_bytes() {
        let value = json!({"type":"update","update":{"content":[{"text":"a中🙂b".repeat(10000)},{"uri":"cowboy-attachment://sha256"}]}});
        let summary = select(&value, &json!({})).unwrap();
        assert!(summary.to_string().len() < 300);
        assert_eq!(summary["data"]["update"]["type"], "object");
        let part = select(
            &value,
            &json!({"path":"/update/content/0/text","offset":1,"chars":2}),
        )
        .unwrap();
        assert_eq!(part["data"], "中🙂");
        assert_eq!(part["range"], json!({"offset":1,"next":3,"total":40000}));
        assert_eq!(
            select(&value, &json!({"path":"/update/content"})),
            Err("selection_too_large".into())
        );
        let attachment = select(&value, &json!({"path":"/update/content/1/uri"})).unwrap();
        assert_eq!(attachment["data"], "cowboy-attachment://sha256");
        assert_eq!(attachment["range"]["next"], Value::Null);
        let relative = select(&value, &json!({"path":"update/content/1/uri"})).unwrap();
        assert_eq!(relative["path"], "/update/content/1/uri");
        assert_eq!(relative["data"], "cowboy-attachment://sha256");
        assert_eq!(
            select(&value, &json!({"path":"bad"})),
            Err("not_found".into())
        );
        for args in [json!({"path":"/bad~2"}), json!({"path":0})] {
            assert_eq!(select(&value, &args), Err("invalid_argument".into()));
        }
    }
}

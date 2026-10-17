//! Lossless, bounded access to one raw event behind a transcript summary.
//! Selection precedes serialization to the model; attachments stay references.
use super::*;

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
    let chars = arg_u64(args, "chars", 200, 1000)? as usize;
    if chars < query.chars().count() {
        return Err("invalid_argument".into());
    }
    let visible = |meta: &SessionMeta| {
        !meta.system && !meta.closing && meta.owner_user_id == parent.owner_user_id
    };
    hub.session_info(id)
        .filter(|info| visible(&info.meta))
        .ok_or("not_found")?;
    let (events, reached_start, source) = if let Some(store) = store {
        let (events, _, start) = store
            .history_page(id, before, crate::core::HISTORY_PAGE)
            .await
            .map_err(|_| "unavailable")?;
        (events, start, "durable")
    } else {
        let (events, start) = hub
            .log_window(id, None, Some(before), crate::core::HISTORY_PAGE, visible)
            .ok_or("not_found")?;
        (events, start, "memory")
    };
    hub.session_info(id)
        .filter(|info| visible(&info.meta))
        .ok_or("not_found")?;
    let mut matches = Vec::new();
    let mut scanned = 0;
    let mut cursor = None;
    for event in events.iter().rev() {
        scanned += 1;
        cursor = Some(event.seq);
        let value = serde_json::to_value(&event.event).map_err(|_| "unavailable")?;
        if let Some((path, text, offset)) = find_text(&value, query, chars, String::new()) {
            matches.push(json!({"seq":event.seq,"path":path,"text":text,"offset":offset,"revision":crate::managed_calls::canonical_digest(&value)}));
            if matches.len() == limit {
                break;
            }
        }
    }
    let more = scanned < events.len() || !reached_start;
    Ok(
        json!({"matches":matches,"scanned":scanned,"next_before":if more {cursor} else {None},"reached_start":reached_start && !more,"source":source}),
    )
}

// Literal, case-sensitive search of stored string values. One match locates an
// event; it is not a claim that a streamed message is one stored string.
fn find_text(
    value: &Value,
    query: &str,
    chars: usize,
    path: String,
) -> Option<(String, String, usize)> {
    match value {
        Value::String(text) => {
            let byte = text.find(query)?;
            let context = 40.min(chars.saturating_sub(query.chars().count()) / 2);
            let offset = text[..byte].chars().count().saturating_sub(context);
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
        for args in [
            json!({"path":"bad"}),
            json!({"path":"/bad~2"}),
            json!({"path":0}),
        ] {
            assert_eq!(select(&value, &args), Err("invalid_argument".into()));
        }
    }
}

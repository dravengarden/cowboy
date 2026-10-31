//! Same-owner session observations. Queue/log visibility is checked under the
//! Hub lock that copies the data; no worker is started or woken by these reads.
use super::*;

#[cfg(test)]
mod tests;

fn visible(meta: &SessionMeta, owner: Option<&str>) -> bool {
    !meta.system && !meta.closing && meta.owner_user_id.as_deref() == owner
}

pub(super) struct ListFilter {
    query: Option<String>,
    exact: Vec<(&'static str, String)>,
}

impl ListFilter {
    pub(super) fn parse(args: &Value) -> Result<Self, String> {
        let query = match args.get("query") {
            None => None,
            Some(Value::String(text))
                if !text.trim().is_empty() && text.len() <= 256 && !text.contains('\0') =>
            {
                Some(text.to_lowercase())
            }
            _ => {
                return Err(
                    "invalid_argument: query must be non-empty text up to 256 bytes".into(),
                );
            }
        };
        let mut exact = Vec::new();
        for key in ["provider", "machine", "workspace"] {
            if args.get(key).is_some() {
                exact.push((key, arg_str(args, key)?.to_owned()));
            }
        }
        Ok(Self { query, exact })
    }

    pub(super) fn matches(&self, meta: &SessionMeta) -> bool {
        self.exact.iter().all(|(key, value)| match *key {
            "provider" => meta.provider == *value,
            "machine" => meta.machine_id == *value,
            _ => {
                meta.workspace_id.as_deref() == Some(value)
                    || meta.workspace_name.as_deref() == Some(value)
            }
        }) && self.query.as_ref().is_none_or(|query| {
            [
                &meta.id,
                &meta.title,
                &meta.cwd,
                &meta.provider,
                &meta.machine_id,
            ]
            .into_iter()
            .chain(meta.workspace_name.iter())
            .chain(meta.workspace_id.iter())
            .any(|field| field.to_lowercase().contains(query))
        })
    }
}

pub(super) async fn history(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    parent: &SessionMeta,
    other_session: bool,
    mut args: Value,
) -> Result<Value, String> {
    let id = if other_session {
        let id = arg_str(&args, "session_id")?.to_owned();
        args.as_object_mut()
            .ok_or("invalid_argument")?
            .remove("session_id");
        id
    } else {
        parent.id.clone()
    };
    only_keys(&args, &["before", "limit", "chars", "kinds"])?;
    let query = EventQuery::parse(&args)?;
    let before = query.before.unwrap_or(i64::MAX as u64);
    if before > i64::MAX as u64 {
        return Err("invalid_argument".into());
    }
    let allowed = |meta: &SessionMeta| visible(meta, parent.owner_user_id.as_deref());
    let (events, reached_start, source) = if let Some(store) = store {
        hub.session_info(&id)
            .filter(|info| allowed(&info.meta))
            .ok_or("not_found")?;
        let (events, _, reached_start) = store
            .history_page(&id, before, crate::core::HISTORY_PAGE)
            .await
            .map_err(|_| "unavailable")?;
        // Recheck after storage I/O; closing or transferring the target must
        // not let a stale authorization release its archived conversation.
        hub.session_info(&id)
            .filter(|info| allowed(&info.meta))
            .ok_or("not_found")?;
        (events, reached_start, "durable")
    } else {
        let (events, reached_start) = hub
            .log_window(&id, None, Some(before), crate::core::HISTORY_PAGE, allowed)
            .ok_or("not_found")?;
        (events, reached_start, "memory")
    };
    let mut value = query.page(&events, reached_start);
    value["source"] = json!(source);
    Ok(value)
}

pub(super) fn read(
    hub: &crate::core::Hub,
    catalog: &crate::provider_catalog::ProviderCatalog,
    parent: &SessionMeta,
    op: &str,
    mut args: Value,
) -> Result<Value, String> {
    let (namespace, method) = op.split_once('.').ok_or("unknown_operation")?;
    let id = if namespace == "sessions" {
        let key = if matches!(method, "info" | "usage") {
            "id"
        } else {
            "session_id"
        };
        let id = arg_str(&args, key)?.to_owned();
        args.as_object_mut().ok_or("invalid_argument")?.remove(key);
        id
    } else {
        parent.id.clone()
    };
    let owner = parent.owner_user_id.as_deref();
    match method {
        "info" | "usage" => {
            only_keys(&args, &[])?;
            let info = hub
                .session_info(&id)
                .filter(|info| visible(&info.meta, owner))
                .ok_or("not_found")?;
            if method == "info" {
                return Ok(session_view(&info));
            }
            let meta = &info.meta;
            Ok(json!({
                "session_id": id,
                "provider": meta.provider,
                "account": catalog.account_usage_provider(&meta.provider, &meta.provider_version, &meta.provider_generation_digest),
                "context": session_view(&info)["context"],
                "usage": meta.usage,
            }))
        }
        "queue" | "drafts" | "message" => {
            if method == "message" {
                only_keys(&args, &["id", "offset", "chars"])?;
            } else {
                only_keys(&args, &["after", "limit", "chars"])?;
            }
            let (queue, drafts) = hub
                .queue_and_drafts(&id, |meta| visible(meta, owner))
                .ok_or("not_found")?;
            if method == "message" {
                let id = arg_str(&args, "id")?;
                let message = queue
                    .iter()
                    .chain(&drafts)
                    .find(|message| message.id == id)
                    .ok_or("not_found")?;
                let mut value = message_view(message);
                text_window(&mut value, "text", &args)?;
                return Ok(value);
            }
            message_page(if method == "queue" { &queue } else { &drafts }, &args)
        }
        "events" => {
            only_keys(&args, &["after", "before", "limit", "chars", "kinds"])?;
            let query = EventQuery::parse(&args)?;
            let (window, reached_start) = hub
                .log_window(&id, query.after, query.before, EVENT_WINDOW, |meta| {
                    visible(meta, owner)
                })
                .ok_or("not_found")?;
            Ok(query.page(&window, reached_start))
        }
        _ => Err("unknown_operation".into()),
    }
}

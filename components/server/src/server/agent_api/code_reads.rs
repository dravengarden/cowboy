//! `cowboy.code.*` reads: the calling session's own worktree only.
//!
//! The scope is always the parent Session's current execution worktree. A
//! code-context string (another session, a project checkout or a task
//! worktree) is not accepted, and content search walks only that root, never
//! the registered checkouts an aggregate worktree's Code tree projects. The
//! Session observation is rechecked after I/O; a retargeted or replaced
//! Session discards the answer.

use serde_json::{Value, json};

use super::super::code_api::{
    ContentSearchParams, content_search, structure_outline, workspace_symbols,
};
use super::super::code_reads::session;
use super::super::{AppState, CodeReadScope};
use super::only_keys;
use crate::core::SessionMeta;

/// Smaller than Review's bound: replies pass the 256 KiB broker limit.
const MAX_AGENT_LIMIT: usize = 200;

pub(super) async fn read(
    state: &AppState,
    meta: &SessionMeta,
    op: &str,
    args: &Value,
) -> Result<Value, String> {
    let request = match op {
        "code.search" => Request::Search(params(args)?),
        "code.outline" => {
            only_keys(args, &["path"])?;
            Request::Outline(text(args, "path")?)
        }
        "code.symbols" => {
            only_keys(args, &["query", "limit"])?;
            Request::Symbols(text(args, "query")?, limit(args)?.unwrap_or(50))
        }
        _ => return Err("unknown_operation".into()),
    };
    let scope = session::resolve(
        &state.hub,
        &state.machine_control,
        &state.service_id,
        &meta.id,
    )
    .ok_or("unavailable")?;
    let CodeReadScope::Session(observed) = &scope else {
        return Err("unavailable".into());
    };
    let cwd = observed.session().cwd().to_owned();
    let failed = |(status, message): (axum::http::StatusCode, String)| {
        if status == axum::http::StatusCode::SERVICE_UNAVAILABLE {
            "unavailable".to_owned()
        } else {
            format!("invalid_argument: {message}")
        }
    };
    let value = match request {
        Request::Search(params) => {
            let result = content_search(state, &scope, cwd, params.query())
                .await
                .map_err(failed)?;
            json!({
                "state": if result.stopped.is_some() { "partial" } else { "complete" },
                "precision": "exact",
                "result": result,
            })
        }
        Request::Outline(path) => {
            let symbols = structure_outline(state, &scope, cwd, path.clone())
                .await
                .map_err(failed)?;
            json!({"state":"complete","precision":"heuristic","path":path,"symbols":symbols})
        }
        Request::Symbols(query, limit) => {
            let found = workspace_symbols(state, &scope, cwd, query, limit)
                .await
                .map_err(failed)?;
            json!({
                "state": if found.stopped.is_some() { "partial" } else { "complete" },
                "precision": "heuristic",
                "result": found,
            })
        }
    };
    if !session::current(&state.hub, &state.machine_control, observed) {
        return Err("code_context_changed".into());
    }
    Ok(value)
}

enum Request {
    Search(ContentSearchParams),
    Outline(String),
    Symbols(String, usize),
}

fn text(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("invalid_argument: {key} must be a string"))
}

fn limit(args: &Value) -> Result<Option<usize>, &'static str> {
    match args.get("limit") {
        None | Some(Value::Null) => Ok(None),
        Some(limit) => Ok(Some(
            usize::try_from(limit.as_u64().ok_or("invalid_argument")?)
                .map_err(|_| "invalid_argument")?
                .clamp(1, MAX_AGENT_LIMIT),
        )),
    }
}

/// Free-text query, never an identifier: it may hold spaces and punctuation.
fn params(args: &Value) -> Result<ContentSearchParams, String> {
    only_keys(args, &["query", "regex", "case_sensitive", "path", "limit"])?;
    Ok(ContentSearchParams {
        q: text(args, "query")?,
        regex: optional_bool(args, "regex")?.unwrap_or(false),
        case_sensitive: optional_bool(args, "case_sensitive")?,
        path: match args.get("path") {
            None | Some(Value::Null) => None,
            Some(path) => Some(path.as_str().ok_or("invalid_argument")?.to_owned()),
        },
        limit: limit(args)?,
    })
}

fn optional_bool(args: &Value, key: &str) -> Result<Option<bool>, &'static str> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or("invalid_argument"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_text_queries_and_bounded_options_parse() {
        let parsed = params(&json!({"query":"Lasso instance (workspace)","regex":false,
            "case_sensitive":true,"path":"docs","limit":5000}))
        .unwrap();
        assert_eq!(parsed.q, "Lasso instance (workspace)");
        assert_eq!(parsed.case_sensitive, Some(true));
        assert_eq!(parsed.path.as_deref(), Some("docs"));
        assert_eq!(parsed.limit, Some(MAX_AGENT_LIMIT));
        for bad in [
            json!({}),
            json!({"query":7}),
            json!({"query":"x","regex":"yes"}),
            json!({"query":"x","other":1}),
        ] {
            assert!(params(&bad).is_err(), "{bad}");
        }
    }
}

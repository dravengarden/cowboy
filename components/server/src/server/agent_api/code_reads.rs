//! `cowboy.code.*` reads: the calling session's own worktree only.
//!
//! The scope is always the parent Session's current execution worktree. A
//! code-context string (another session, a project checkout or a task
//! worktree) is not accepted, and content search walks only that root, never
//! the registered checkouts an aggregate worktree's Code tree projects. The
//! Session observation is rechecked after I/O; a retargeted or replaced
//! Session discards the answer.

use serde_json::{Value, json};

use super::super::code_api::{ContentSearchParams, content_search};
use super::super::code_reads::session;
use super::super::{AppState, CodeReadScope};
use super::{arg_str, only_keys};
use crate::core::SessionMeta;

/// Smaller than Review's bound: replies pass the 256 KiB broker limit.
const MAX_AGENT_LIMIT: usize = 200;

pub(super) async fn read(
    state: &AppState,
    meta: &SessionMeta,
    op: &str,
    args: &Value,
) -> Result<Value, String> {
    if op != "code.search" {
        return Err("unknown_operation".into());
    }
    only_keys(args, &["query", "regex", "case_sensitive", "path", "limit"])?;
    let params = ContentSearchParams {
        q: arg_str(args, "query")?.to_owned(),
        regex: optional_bool(args, "regex")?.unwrap_or(false),
        case_sensitive: optional_bool(args, "case_sensitive")?,
        path: match args.get("path") {
            None | Some(Value::Null) => None,
            Some(path) => Some(path.as_str().ok_or("invalid_argument")?.to_owned()),
        },
        limit: match args.get("limit") {
            None | Some(Value::Null) => None,
            Some(limit) => Some(
                usize::try_from(limit.as_u64().ok_or("invalid_argument")?)
                    .map_err(|_| "invalid_argument")?
                    .clamp(1, MAX_AGENT_LIMIT),
            ),
        },
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
    let result = content_search(state, &scope, cwd, params.query())
        .await
        .map_err(|(status, message)| {
            if status == axum::http::StatusCode::BAD_REQUEST {
                format!("invalid_argument: {message}")
            } else {
                "unavailable".to_owned()
            }
        })?;
    if !session::current(&state.hub, &state.machine_control, observed) {
        return Err("code_context_changed".into());
    }
    Ok(json!({
        "state": if result.stopped.is_some() { "partial" } else { "complete" },
        "precision": "exact",
        "result": result,
    }))
}

fn optional_bool(args: &Value, key: &str) -> Result<Option<bool>, &'static str> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_bool().map(Some).ok_or("invalid_argument"),
    }
}

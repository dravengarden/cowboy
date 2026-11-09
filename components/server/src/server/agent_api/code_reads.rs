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
    CheckStarted, ContentSearchParams, check_runs, content_search, declared_checks, start_check,
    structure_outline, workspace_symbols,
};
use super::super::code_reads::session;
use super::super::{AppState, CodeReadScope};
use super::only_keys;
use crate::core::SessionMeta;

mod acquire;
mod baselines;
mod codex_push;
mod deliveries;
mod diagnostics;
mod hover;
mod post_edit;
mod summary;

pub(in crate::server) async fn maintain_holdings(state: std::sync::Arc<AppState>) {
    // Codex post-edit delivery shares this Controller-lifetime task.
    let push = tokio::spawn(codex_push::run(std::sync::Arc::clone(&state)));
    let mut shutdown = state.shutdown.clone();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = tick.tick() => acquire::release_stale(&state).await,
        }
    }
    push.abort();
}

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
        "code.checks" => {
            only_keys(args, &[])?;
            return checks(state, meta).await;
        }
        "code.diagnostics" => {
            only_keys(args, &["paths"])?;
            Request::Diagnostics(paths(args)?)
        }
        "code.hover" => Request::Hover(hover::Locator::parse(args)?),
        "code.summary" => {
            only_keys(args, &[])?;
            Request::Summary
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
    if let Request::Hover(locator) = request {
        let value = hover::read(state, observed, locator).await;
        if !session::current(&state.hub, &state.machine_control, observed) {
            return Err("code_context_changed".into());
        }
        return Ok(value);
    }
    if let Request::Diagnostics(paths) = request {
        super::super::managed_calls::touch_code_observation(state, observed).await;
        let value = diagnostics::read(state, observed, paths).await;
        if !session::current(&state.hub, &state.machine_control, observed) {
            return Err("code_context_changed".into());
        }
        return Ok(value);
    }
    if let Request::Summary = request {
        let paths = acquire::observed_paths(observed.session());
        let value = diagnostics::read_existing(state, observed, paths).await;
        if !session::current(&state.hub, &state.machine_control, observed) {
            return Err("code_context_changed".into());
        }
        let mut projected = summary::project(value);
        projected["deliveries"] = deliveries::list(&meta.id)["deliveries"].take();
        return Ok(projected);
    }
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
        Request::Diagnostics(_) | Request::Summary | Request::Hover(_) => {
            unreachable!("answered above")
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
    Diagnostics(Vec<String>),
    Hover(hover::Locator),
    Summary,
    Outline(String),
    Symbols(String, usize),
}

fn text(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("invalid_argument: {key} must be a string"))
}

fn paths(args: &Value) -> Result<Vec<String>, String> {
    let paths = args
        .get("paths")
        .and_then(Value::as_array)
        .filter(|paths| !paths.is_empty() && paths.len() <= diagnostics::MAX_PATHS)
        .ok_or_else(|| {
            format!(
                "invalid_argument: paths must list 1..={} worktree-relative files",
                diagnostics::MAX_PATHS
            )
        })?;
    paths
        .iter()
        .map(|path| {
            path.as_str()
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| "invalid_argument: paths must be strings".to_owned())
        })
        .collect()
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

/// Declared checks and the latest run of each. Declarations are read through
/// the same Session observation as every other code read.
async fn checks(state: &AppState, meta: &SessionMeta) -> Result<Value, String> {
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
    let declared = declared_checks(state, &scope, observed.session().cwd().to_owned()).await;
    if !session::current(&state.hub, &state.machine_control, observed) {
        return Err("code_context_changed".into());
    }
    Ok(json!({
        "allowed": state.hub.language_servers_allowed(&meta.id),
        "declared": declared.as_ref().map(|checks| checks.iter().map(|check| json!({
            "id": check.id, "title": check.title, "format": check.format,
            "timeoutSeconds": check.timeout().as_secs(),
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "declaration_error": declared.err(),
        "runs": check_runs(&meta.id),
    }))
}

/// The bounded post-edit summary a Provider adapter delivers to its model
/// (`cowboy_execute` action `code.post_edit`, read-only, no operation needed).
pub(super) async fn post_edit(
    state: &AppState,
    meta: &SessionMeta,
    args: &Value,
) -> Result<Value, &'static str> {
    let mut value = post_edit_value(state, meta, args).await?;
    if value["deliver"] == true {
        let files = changed_files(&value);
        match deliveries::begin(
            &meta.id,
            deliveries::Route::Adapter,
            files,
            deliveries::digest(&value),
        ) {
            Some(id) => value["delivery"] = Value::String(id),
            None => {
                value["deliver"] = Value::Bool(false);
                value["suppressed"] = Value::String("repeat".into());
            }
        }
    }
    Ok(value)
}

fn session_cwd(state: &AppState, session: &str) -> Option<String> {
    match session::resolve(
        &state.hub,
        &state.machine_control,
        &state.service_id,
        session,
    )? {
        CodeReadScope::Session(observed) => {
            Some(observed.session().cwd().trim_end_matches('/').to_owned())
        }
        _ => None,
    }
}

/// Files whose entry a delivery would show.
fn changed_files(value: &Value) -> Vec<String> {
    value["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|file| file.get("diagnostics").is_some())
        .filter_map(|file| file["path"].as_str().map(str::to_owned))
        .collect()
}

/// The `code.post_edit` read without delivery bookkeeping.
async fn post_edit_value(
    state: &AppState,
    meta: &SessionMeta,
    args: &Value,
) -> Result<Value, &'static str> {
    only_keys(args, &["paths", "changed"]).map_err(|_| "invalid_argument")?;
    let paths = if args.get("changed") == Some(&Value::Bool(true)) && args.get("paths").is_none() {
        // Files a shell command changed since the last read, from the
        // standing observer; a session without one reports nothing.
        let root = format!("{}/", session_cwd(state, &meta.id).ok_or("unavailable")?);
        let changed: Vec<String> = codex_push::shell_changes(state, &meta.id)
            .await
            .into_iter()
            .filter_map(|path| path.strip_prefix(&root).map(str::to_owned))
            .collect();
        if changed.is_empty() {
            return Ok(serde_json::json!({"deliver":false,"files":[],"truncated":false}));
        }
        changed
    } else {
        paths(args).map_err(|_| "invalid_argument")?
    };
    if paths.len() > post_edit::MAX_PATHS {
        return Err("invalid_argument");
    }
    let scope = session::resolve(
        &state.hub,
        &state.machine_control,
        &state.service_id,
        &meta.id,
    )
    .ok_or("unavailable")?;
    let CodeReadScope::Session(observed) = &scope else {
        return Err("unavailable");
    };
    super::super::managed_calls::touch_code_observation(state, observed).await;
    let value = diagnostics::read(state, observed, paths).await;
    if !session::current(&state.hub, &state.machine_control, observed) {
        return Err("code_context_changed");
    }
    // Later shell-made changes are reported from this position on.
    codex_push::sync_cursor(state, &meta.id).await;
    let mut projected = post_edit::project(&value);
    post_edit::attach_writes(&mut projected, &value, |path| {
        let target = format!("{}/{path}", observed.session().cwd().trim_end_matches('/'));
        super::super::execution::writes::latest(&meta.id, &target).map(|write| write.sha256)
    });
    Ok(projected)
}

/// Explicitly run one declared check (`cowboy_execute` action `code.check`).
/// The same operation observes its run instead of starting another.
pub(super) async fn check(
    state: &std::sync::Arc<AppState>,
    meta: &SessionMeta,
    args: &Value,
    operation: &str,
) -> Result<Value, &'static str> {
    only_keys(args, &["check"]).map_err(|_| "invalid_argument")?;
    let check = args
        .get("check")
        .and_then(Value::as_str)
        .ok_or("invalid_argument")?;
    match start_check(state, &meta.id, check, operation).await {
        Ok(CheckStarted::New(run)) => Ok(json!({"status":"accepted","run":run})),
        Ok(CheckStarted::Existing(run)) => Ok(json!({"status":"existing","run":run})),
        Err(error) if error.starts_with("restricted") => Err("restricted"),
        Err(error) if error.starts_with("no declared check") => Err("not_found"),
        Err(_) => Err("unavailable"),
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
        assert_eq!(
            paths(&json!({"paths":["a.rs","b.rs"]})).unwrap(),
            ["a.rs", "b.rs"]
        );
        for bad in [
            json!({}),
            json!({"paths":[]}),
            json!({"paths":[1]}),
            json!({"paths":"a"}),
        ] {
            assert!(paths(&bad).is_err(), "{bad}");
        }
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

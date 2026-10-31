//! Controller side of the Agent API (`docs/agent-api.md`).
//!
//! An action arrives through the managed-call gateway after the coordinator
//! authorized the parent, so every operation here acts for exactly that
//! session and its owner. A read program runs in a fresh contained runtime
//! (Bubblewrap namespaces without network, a read-only system view, resource
//! limits and a wall-clock deadline); it reaches Cowboy only through the
//! line-delimited broker below, which validates every operation and bounds
//! every reply. There is no unsandboxed fallback.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Semaphore;
use tokio::time::Instant;

use super::AppState;
use crate::agent_api::{
    API_VERSION, MAX_BROKER_BYTES, MAX_CALLS, MAX_LOG_BYTES, MAX_REPLY_BYTES, MAX_RESULT_BYTES,
};
use crate::core::{Envelope, Event, QueuedMessage, SessionInfo, SessionMeta};
use crate::managed_calls::protocol::{Action, Grant};
use crate::managed_calls::service::Ledger;

const RUNNER: &str = include_str!("agent_api/runner.js");
mod actions;
mod control;
mod event_reads;
mod json_reads;
mod observations;
mod review_defaults;
mod session_reads;
mod usage_reads;
pub(super) use actions::{invoke, read_parent};
/// Concurrent programs across the Controller; excess is refused, not queued.
static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));
/// Raw hot-log events one `session.events` call may coalesce.
const EVENT_WINDOW: usize = 2000;

pub(super) struct Program {
    pub code: String,
    pub typescript: bool,
    pub module: bool,
    pub timeout_ms: u64,
}

fn error(code: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code}})
}

/// The compact status: no program, no runtime start.
pub(super) fn status(state: &Arc<AppState>, meta: &SessionMeta, grant: &Grant) -> Value {
    let Some(info) = state.hub.session_info(&meta.id) else {
        return error("not_found");
    };
    let mut value = status_view(
        &info,
        super::agent_tools::effective(state, meta).calls.enabled,
        grant,
    );
    value["control"] = json!({"writes": control_supported(state,meta),"minimum_execution_protocol":crate::machine_protocol::AGENT_CONTROL_PROTOCOL_VERSION});
    value
}

fn control_supported(state: &AppState, meta: &SessionMeta) -> bool {
    state
        .runtime_router
        .runtime(&meta.machine_id)
        .and_then(|runtime| runtime.managed_call_authority(&state.service_id, meta).ok())
        .and_then(|authority| {
            state
                .machine_control
                .connection_protocol(&authority.placement().machine_id)
        })
        .is_some_and(|(_, protocol)| {
            protocol >= crate::machine_protocol::AGENT_CONTROL_PROTOCOL_VERSION
        })
}

fn status_view(info: &SessionInfo, calls_enabled: bool, grant: &Grant) -> Value {
    json!({
        "schema": 1,
        "state": "ok",
        "api": API_VERSION,
        "revision": crate::agent_api::revision(),
        // The authenticated grant describes the execution Machine. The
        // session's runtime Machine can have a different protocol version.
        "actions": grant.agent_actions,
        "session": session_view(info),
        "calls": {"enabled": calls_enabled},
    })
}

pub(super) async fn read(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    ledger: &Ledger,
    program: Program,
) -> Value {
    let Ok(_slot) = SLOTS.try_acquire() else {
        return error("capacity");
    };
    let sandbox = match Sandbox::resolve() {
        Ok(sandbox) => sandbox,
        Err(code) => return error(code),
    };
    let host = SessionHost {
        state,
        meta,
        ledger,
    };
    sandbox.execute(&program, &host).await
}

// ---------------------------------------------------------------------------
// Operations

trait Host: Sync {
    fn call(
        &self,
        op: &str,
        args: Value,
        remaining: Duration,
    ) -> impl std::future::Future<Output = Result<Value, String>> + Send;
}

struct SessionHost<'a> {
    state: &'a Arc<AppState>,
    meta: &'a SessionMeta,
    ledger: &'a Ledger,
}

fn arg_str<'v>(args: &'v Value, key: &str) -> Result<&'v str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| crate::managed_calls::valid_id(value))
        .ok_or_else(|| format!("invalid_argument: {key} must be an id string"))
}

// Name the violated bound: a bare code made callers guess and retry.
fn arg_u64(args: &Value, key: &str, default: u64, max: u64) -> Result<u64, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|value| *value <= max)
            .ok_or_else(|| format!("invalid_argument: {key} must be an integer 0..={max}")),
    }
}

fn page_limit(args: &Value, default: u64, max: u64) -> Result<usize, String> {
    let limit = arg_u64(args, "limit", default, max)?;
    if limit == 0 {
        return Err(format!("invalid_argument: limit must be 1..={max}"));
    }
    usize::try_from(limit).map_err(|_| "invalid_argument".into())
}

fn explicit_arg(args: &Value, key: &str) -> bool {
    args.get(key).is_some_and(|value| !value.is_null())
}

fn require_page(args: &Value, count: usize, limit: usize) -> Result<(), String> {
    if count > limit && !explicit_arg(args, "limit") {
        return Err("page_required".into());
    }
    Ok(())
}

fn after_id(args: &Value) -> Result<Option<&str>, String> {
    args.get("after")
        .map(|_| arg_str(args, "after"))
        .transpose()
}

// Name the rejected key and the accepted ones; a bare code was the most
// common reason agents retried a read with another guessed option.
fn only_keys(args: &Value, allowed: &[&str]) -> Result<(), String> {
    let Value::Object(map) = args else {
        return Err("invalid_argument: options must be an object".to_owned());
    };
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        None => Ok(()),
        Some(key) if allowed.is_empty() => Err(format!(
            "invalid_argument: {} is not accepted; this read takes no options",
            bounded(key, 40)
        )),
        Some(key) => Err(format!(
            "invalid_argument: {} is not accepted; options are {}",
            bounded(key, 40),
            allowed.join(", ")
        )),
    }
}

impl Host for SessionHost<'_> {
    async fn call(&self, op: &str, args: Value, remaining: Duration) -> Result<Value, String> {
        // Programs can run for a minute. A parent may close, change owner or
        // lose its account while a preceding operation is awaiting a reply.
        actions::read_parent(&self.state.hub, self.state.store.as_ref(), self.meta).await?;
        match op {
            "control.list" | "control.describe" | "control.read" => {
                control::read(self.state, self.meta, op, args).await
            }
            "machines.list"
            | "machines.read"
            | "configuration.schema"
            | "configuration.read"
            | "usage.diagnostics" => observations::read(self.state, self.meta, op, &args).await,
            "api.help" => {
                only_keys(&args, &["topic"])?;
                crate::agent_api::help(arg_str(&args, "topic")?).ok_or("invalid_help".into())
            }
            "session.event" | "sessions.event" => {
                let value = event_reads::read(
                    &self.state.hub,
                    self.state.store.as_ref(),
                    self.meta,
                    op == "sessions.event",
                    &args,
                )
                .await?;
                actions::read_parent(&self.state.hub, self.state.store.as_ref(), self.meta).await?;
                Ok(value)
            }
            "session.search" | "sessions.search" => {
                let value = event_reads::search(
                    &self.state.hub,
                    self.state.store.as_ref(),
                    self.meta,
                    op == "sessions.search",
                    &args,
                )
                .await?;
                actions::read_parent(&self.state.hub, self.state.store.as_ref(), self.meta).await?;
                Ok(value)
            }
            "calls.list" => {
                only_keys(&args, &["before", "limit"])?;
                let before = args
                    .get("before")
                    .map(|_| arg_str(&args, "before"))
                    .transpose()?;
                let limit = page_limit(&args, 20, 100)?;
                let rows = self.ledger.list(before).await.map_err(|_| "unavailable")?;
                actions::read_parent(&self.state.hub, self.state.store.as_ref(), self.meta).await?;
                let next = (rows.len() >= limit).then(|| rows[limit - 1].call_id.clone());
                let calls = rows
                    .iter()
                    .take(limit)
                    .map(|row| {
                        let summary = row.summary();
                        json!({"call_id":row.call_id,"request_id":row.request_id,"provider":row.provider,
                            "state":row.state,"purpose":row.purpose,"updated_at_ms":row.updated_at_ms,
                            "created_at_ms":row.created_at_ms,"started_at_ms":row.started_at_ms,"finished_at_ms":row.finished_at_ms,
                            "configuration":row.configuration,"preset":row.preset,"provider_account_id":row.provider_account_id,
                            "labels":row.labels,"input_revision":row.input_revision,
                            "usage":summary.usage,"reused_from":summary.reused_from,
                            "has_result":summary.has_result,"verdict":summary.verdict,"findings":summary.finding_count})
                    })
                    .collect::<Vec<_>>();
                Ok(json!({"calls":calls,"next_before":next}))
            }
            "session.history" | "sessions.history" => {
                let value = session_reads::history(
                    &self.state.hub,
                    self.state.store.as_ref(),
                    self.meta,
                    op == "sessions.history",
                    args,
                )
                .await?;
                actions::read_parent(&self.state.hub, self.state.store.as_ref(), self.meta).await?;
                Ok(value)
            }
            "session.info" | "session.usage" | "session.queue" | "session.drafts"
            | "session.message" | "session.events" | "sessions.info" | "sessions.usage"
            | "sessions.queue" | "sessions.drafts" | "sessions.message" | "sessions.events" => {
                session_reads::read(
                    &self.state.hub,
                    &self.state.provider_catalog,
                    self.meta,
                    op,
                    args,
                )
            }
            "usage.list" | "usage.read" => {
                usage_reads::read(self.state, self.meta, op, &args).await
            }
            "preferences.schema" => {
                only_keys(&args, &[])?;
                Ok(crate::store::preferences::schema())
            }
            "accounts.list" => {
                only_keys(&args, &[])?;
                let owner = actions::account_owner(self.state, self.meta).await?;
                super::provider_accounts::read_value(self.state, &owner)
                    .await
                    .map_err(str::to_owned)
            }
            "accounts.transition_status" => {
                only_keys(&args, &["operation"])?;
                let operation = arg_str(&args, "operation")?;
                if !crate::managed_calls::valid_id(operation) {
                    return Err("invalid_argument".into());
                }
                let owner = actions::account_owner(self.state, self.meta).await?;
                let mut value = super::provider_accounts::transitions::status(
                    self.state,
                    &owner,
                    &actions::document_operation(&self.meta.id, operation),
                )
                .await
                .map_err(str::to_owned)?;
                value["operation"] = json!(operation);
                Ok(value)
            }
            "accounts.revocation_status" => {
                only_keys(&args, &["account_id"])?;
                let owner = actions::account_owner(self.state, self.meta).await?;
                let account = crate::provider_account::ProviderAccountId::try_from(
                    arg_str(&args, "account_id")?.to_owned(),
                )
                .map_err(|_| "invalid_argument".to_owned())?;
                super::provider_accounts::lifecycle::status(self.state, &owner, &account)
                    .await
                    .map_err(str::to_owned)
            }
            "accounts.logins" | "accounts.login_status" => {
                only_keys(
                    &args,
                    if op == "accounts.logins" {
                        &["account_id", "after_machine", "limit"]
                    } else {
                        &["account_id", "request_id"]
                    },
                )?;
                let owner = actions::account_owner(self.state, self.meta).await?;
                let account = crate::provider_account::ProviderAccountId::try_from(
                    arg_str(&args, "account_id")?.to_owned(),
                )
                .map_err(|_| "invalid_argument".to_owned())?;
                if op == "accounts.login_status" {
                    return super::provider_accounts::login::continue_request(
                        self.state,
                        &owner,
                        &account,
                        arg_str(&args, "request_id")?,
                        false,
                    )
                    .await
                    .map_err(str::to_owned);
                }
                let mut query = args;
                query
                    .as_object_mut()
                    .ok_or("invalid_argument")?
                    .remove("account_id");
                let query =
                    serde_json::from_value(query).map_err(|_| "invalid_argument".to_owned())?;
                super::provider_accounts::login::logins_value(self.state, &owner, &account, query)
                    .await
                    .map_err(str::to_owned)
            }
            "accounts.usage" => {
                only_keys(&args, &["account_id"])?;
                let owner = actions::account_owner(self.state, self.meta).await?;
                let account = arg_str(&args, "account_id")?
                    .parse()
                    .map_err(str::to_owned)?;
                let value = super::provider_accounts::usage::read_one(self.state, &owner, &account)
                    .await
                    .map_err(str::to_owned)?;
                actions::read_parent(&self.state.hub, self.state.store.as_ref(), self.meta).await?;
                Ok(value)
            }
            "accounts.login_options" => {
                only_keys(&args, &["account_id", "machine_id"])?;
                let owner = actions::account_owner(self.state, self.meta).await?;
                let account = crate::provider_account::ProviderAccountId::try_from(
                    arg_str(&args, "account_id")?.to_owned(),
                )
                .map_err(|_| "invalid_argument".to_owned())?;
                super::provider_accounts::login::options_value(
                    self.state,
                    &owner,
                    &account,
                    arg_str(&args, "machine_id")?,
                )
                .await
                .map_err(str::to_owned)
            }
            "preferences.get" => {
                only_keys(&args, &["scope"])?;
                let owner = self
                    .meta
                    .owner_user_id
                    .as_deref()
                    .ok_or("permission_denied")?;
                if args["scope"] == "zed" {
                    return review_defaults::read(self.state, self.meta, owner)
                        .await
                        .map_err(str::to_owned);
                }
                super::preferences::read_value(self.state, owner, arg_str(&args, "scope")?)
                    .await
                    .map_err(str::to_owned)
            }
            "calls.capabilities" => {
                only_keys(&args, &["capability", "if_revision"])?;
                if let Some(capability) = args.get("capability") {
                    let capability = capability.as_str().ok_or("invalid_argument")?;
                    if !cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1
                        .capabilities()
                        .contains(&capability)
                    {
                        return Err("unsupported_capability".into());
                    }
                    let if_revision = args
                        .get("if_revision")
                        .map(|value| value.as_str().ok_or("invalid_argument"))
                        .transpose()?;
                    let value = super::managed_calls::image_capabilities(
                        self.state,
                        self.meta,
                        self.ledger,
                        capability,
                        if_revision,
                    )
                    .await;
                    actions::read_parent(&self.state.hub, self.state.store.as_ref(), self.meta)
                        .await?;
                    return Ok(value);
                }
                if args.get("if_revision").is_some() {
                    return Err("invalid_argument".into());
                }
                self.calls(Action::Capabilities {}).await
            }
            "calls.inspect" => {
                only_keys(&args, &["call_id"])?;
                let call_id = arg_str(&args, "call_id")?.to_owned();
                self.calls(Action::Inspect { call_id }).await
            }
            "calls.result" => {
                only_keys(&args, &["call_id", "offset", "chars"])?;
                let call_id = arg_str(&args, "call_id")?.to_owned();
                let mut value = self.calls(Action::Result { call_id }).await?;
                text_window(&mut value["result"], "text", &args)?;
                Ok(value)
            }
            "calls.wait" => {
                only_keys(&args, &["call_id", "timeout_ms"])?;
                let call_id = arg_str(&args, "call_id")?.to_owned();
                let requested = arg_u64(
                    &args,
                    "timeout_ms",
                    30_000,
                    crate::managed_calls::protocol::MAX_WAIT_MS,
                )?;
                // Leave the program time to use what it waited for.
                let budget = remaining.saturating_sub(Duration::from_millis(1000));
                let timeout_ms = requested.min(u64::try_from(budget.as_millis()).unwrap_or(0));
                self.calls(Action::Wait {
                    call_id,
                    timeout_ms,
                })
                .await
            }
            "calls.observe" => {
                only_keys(&args, &["request_id"])?;
                let request_id = arg_str(&args, "request_id")?.to_owned();
                self.calls(Action::Observe { request_id }).await
            }
            "sessions.list" => {
                only_keys(
                    &args,
                    &[
                        "status",
                        "limit",
                        "after",
                        "query",
                        "provider",
                        "machine",
                        "workspace",
                    ],
                )?;
                let filter = session_reads::ListFilter::parse(&args)?;
                let status = match args.get("status") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(status)) if status.len() <= 32 => Some(status.clone()),
                    Some(_) => return Err("invalid_argument".into()),
                };
                let limit = page_limit(&args, 50, 200)?;
                let after = after_id(&args)?;
                let owner = self.meta.owner_user_id.clone();
                let mut rows = self
                    .state
                    .hub
                    .session_list_filtered(|row| row == owner.as_deref());
                rows.sort_by(|a, b| a.id.cmp(&b.id));
                let rows = rows
                    .into_iter()
                    .filter(|meta| {
                        !meta.system
                            && !meta.closing
                            && after.is_none_or(|after| meta.id.as_str() > after)
                    })
                    .filter_map(|meta| self.state.hub.session_info(&meta.id))
                    .filter(|info| {
                        !info.meta.system && !info.meta.closing && info.meta.owner_user_id == owner
                    })
                    .filter(|info| filter.matches(&info.meta))
                    .map(|info| session_view(&info))
                    .filter(|view| {
                        status
                            .as_deref()
                            .is_none_or(|status| view["status"] == status)
                    })
                    .take(limit)
                    .collect();
                Ok(Value::Array(rows))
            }
            "documents.list" | "documents.read" => {
                let owner = self.meta.owner_user_id.as_deref().ok_or("unavailable")?;
                let store = self.state.store.as_ref().ok_or("unavailable")?;
                if op == "documents.list" {
                    document_page(store, owner, &args).await
                } else {
                    only_keys(&args, &["id", "offset", "chars", "include_deleted"])?;
                    let id = arg_str(&args, "id")?;
                    let deleted = optional_bool(&args, "include_deleted")?;
                    let document = store
                        .draft_document(owner, id)
                        .await
                        .map_err(|_| "unavailable")?
                        .filter(|document| deleted || !document.deleted)
                        .ok_or("not_found")?;
                    let mut value = document_view(&document, true);
                    text_window(&mut value, "body", &args)?;
                    Ok(value)
                }
            }
            _ => Err("unknown_operation".into()),
        }
    }
}

impl SessionHost<'_> {
    /// Call observations share the gateway's exact contract.
    async fn calls(&self, action: Action) -> Result<Value, String> {
        let mut value =
            super::managed_calls::dispatch_calls(self.state, self.meta, self.ledger, action).await;
        if value.get("state").and_then(Value::as_str) == Some("error")
            && value.get("call_id").is_none()
        {
            let code = value
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("unavailable");
            return Err(code.to_owned());
        }
        if let Some(object) = value.as_object_mut() {
            // CLI-only fields: the program needs no argv or wire schema.
            object.remove("schema");
            object.remove("next");
        }
        Ok(value)
    }
}

fn session_view(info: &SessionInfo) -> Value {
    let meta = &info.meta;
    json!({
        "id": meta.id,
        "title": meta.title,
        "provider": meta.provider,
        "provider_account_id": meta.provider_account_id,
        "provider_version": meta.provider_version,
        "status": meta.status,
        "machine": meta.machine_id,
        "workspace": meta.workspace_name.as_ref().or(meta.workspace_id.as_ref()),
        "cwd": meta.cwd,
        "execution": meta.execution_binding.as_ref().and_then(|b| b.for_runtime(&meta.machine_id, &meta.cwd).ok())
            .map(|b| json!({"machine":b.environment.machine_id,"workspace":b.workspace.id,"cwd":b.workspace.cwd})),
        "paused": meta.paused,
        "queued": info.queue_count,
        "drafts": info.drafts_count,
        "events": info.event_count,
        "background_tasks": meta.background_tasks,
        "context": (meta.context_size > 0)
            .then(|| json!({"used": meta.context_used, "size": meta.context_size})),
    })
}

fn message_view(message: &QueuedMessage) -> Value {
    let mut view = json!({
        "id": message.id,
        "text": message.text,
        "attachments": message.content.iter()
            .filter(|block| block.get("type").and_then(Value::as_str) != Some("text"))
            .count(),
    });
    if message.text.is_empty() {
        view["text"] = json!(content_text(&message.content));
    }
    if let Some(schedule) = &message.schedule {
        view["scheduled_at_ms"] = json!(schedule.fire_at_ms);
    }
    view
}

fn message_page(messages: &[QueuedMessage], args: &Value) -> Result<Value, String> {
    let limit = page_limit(args, 20, 100)?;
    let start = after_id(args)?
        .map(|id| {
            messages
                .iter()
                .position(|message| message.id == id)
                .map(|index| index + 1)
                .ok_or_else(|| "invalid_cursor".to_owned())
        })
        .transpose()?
        .unwrap_or(0);
    require_page(args, messages.len().saturating_sub(start), limit)?;
    let chars = arg_u64(args, "chars", 400, 4000)?;
    if chars == 0 {
        return Err("invalid_argument".into());
    }
    messages
        .iter()
        .skip(start)
        .take(limit)
        .map(|message| {
            let mut value = message_view(message);
            if !explicit_arg(args, "chars")
                && value["text"]
                    .as_str()
                    .is_some_and(|text| text.chars().count() > chars as usize)
            {
                return Err("text_window_required".into());
            }
            text_window(&mut value, "text", &json!({"chars":chars}))?;
            Ok(value)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

async fn document_page(
    store: &crate::store::Store,
    owner: &str,
    args: &Value,
) -> Result<Value, String> {
    only_keys(args, &["after", "limit", "include_deleted"])?;
    let after = after_id(args)?;
    let deleted = optional_bool(args, "include_deleted")?;
    let limit = page_limit(args, 50, 200)?;
    // One lookahead row distinguishes a complete legacy read from a page.
    // The default is 50, so lookahead stays inside the store's 200-row bound.
    let fetch = limit + usize::from(!explicit_arg(args, "limit"));
    let documents = store
        .draft_documents_page(owner, after, fetch as i64, deleted)
        .await
        .map_err(|_| "unavailable")?;
    require_page(args, documents.len(), limit)?;
    Ok(Value::Array(
        documents
            .iter()
            .map(|document| document_view(document, false))
            .collect(),
    ))
}

fn document_view(document: &crate::store::draft_documents::DraftDocument, body: bool) -> Value {
    let mut view = serde_json::to_value(document).unwrap_or(Value::Null);
    if let Some(object) = view.as_object_mut() {
        let attachments = object
            .remove("attachments")
            .and_then(|value| value.as_array().map(Vec::len))
            .unwrap_or(0);
        if body {
            object.insert("attachments".into(), json!(attachments));
        } else {
            object.remove("body");
        }
    }
    view
}

fn optional_bool(args: &Value, key: &str) -> Result<bool, String> {
    match args.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err("invalid_argument".into()),
    }
}

/// Unicode scalar offsets, never byte slices. Explicit windows let a program
/// read a large retained value without first exceeding the broker's budget.
fn text_window(value: &mut Value, field: &str, args: &Value) -> Result<(), String> {
    let offset = usize::try_from(arg_u64(args, "offset", 0, u64::from(u32::MAX))?)
        .map_err(|_| "invalid_argument")?;
    let count =
        usize::try_from(arg_u64(args, "chars", 1000, 4000)?).map_err(|_| "invalid_argument")?;
    if count == 0 {
        return Err("invalid_argument".into());
    }
    if let Some(text) = value[field].as_str() {
        let total = text.chars().count();
        // Older callers may append to an unwindowed document or treat a call
        // result as complete. Never silently turn that full read into a prefix.
        if !explicit_arg(args, "offset") && !explicit_arg(args, "chars") && total > count {
            return Err("text_window_required".into());
        }
        let offset = offset.min(total);
        let body: String = text.chars().skip(offset).take(count).collect();
        let end = offset + body.chars().count();
        value[field] = json!(body);
        value["range"] = json!({"offset":offset,"next":(end<total).then_some(end),"total":total});
    }
    Ok(())
}

fn content_text(content: &[Value]) -> String {
    content
        .iter()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Transcript projection: chunks coalesce into messages, tool updates into
// their call, thoughts are omitted.

const EVENT_KINDS: &[&str] = &[
    "user",
    "agent",
    "tool",
    "plan",
    "permission",
    "turn_end",
    "lifecycle",
];

struct EventQuery {
    after: Option<u64>,
    before: Option<u64>,
    limit: usize,
    chars: usize,
    kinds: Vec<String>,
}

impl EventQuery {
    fn parse(args: &Value) -> Result<Self, String> {
        let optional = |key: &str| match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| format!("invalid_argument: {key} must be an event seq integer")),
        };
        let invalid_kinds = || {
            format!(
                "invalid_argument: kinds must be an array of {}",
                EVENT_KINDS.join(", ")
            )
        };
        let kinds = match args.get("kinds") {
            None | Some(Value::Null) => EVENT_KINDS
                .iter()
                .filter(|kind| **kind != "lifecycle")
                .map(|kind| (*kind).to_owned())
                .collect(),
            Some(Value::Array(kinds)) => kinds
                .iter()
                .map(|kind| {
                    kind.as_str()
                        .filter(|kind| EVENT_KINDS.contains(kind))
                        .map(str::to_owned)
                        .ok_or_else(invalid_kinds)
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(invalid_kinds()),
        };
        Ok(Self {
            after: optional("after")?,
            before: optional("before")?,
            limit: page_limit(args, 20, 100)?,
            chars: usize::try_from(arg_u64(args, "chars", 400, 4000)?).unwrap_or(4000),
            kinds,
        })
    }

    fn page(&self, window: &[Envelope], reached_start: bool) -> Value {
        // Select raw boundaries before folding tools across messages. Otherwise
        // reverse paging can discard a tool's final update with its older start.
        // Contiguous message chunks still count as one group, so streaming does
        // not turn a message into hundreds of tiny pages.
        let mut groups: Vec<Item> = coalesce(window, false)
            .into_iter()
            .filter(|item| self.kinds.iter().any(|kind| kind == item.kind))
            .collect();
        let (more, next, selected) = if self.after.is_some() {
            let more = groups.len() > self.limit || window.len() == EVENT_WINDOW;
            let cursor = groups
                .get(self.limit)
                .map(|item| item.seq.saturating_sub(1))
                .or_else(|| window.last().map(|event| event.seq));
            let end = cursor.map_or(window.len(), |seq| {
                window.partition_point(|event| event.seq <= seq)
            });
            (
                more,
                cursor.map(|after| json!({"after":after})),
                &window[..end],
            )
        } else {
            let more = groups.len() > self.limit || !reached_start;
            let trimmed = groups.len() > self.limit;
            groups.drain(..groups.len().saturating_sub(self.limit));
            let cursor = if trimmed {
                groups.first().map(|item| item.seq)
            } else {
                window.first().map(|event| event.seq)
            };
            let start = cursor.map_or(0, |seq| window.partition_point(|event| event.seq < seq));
            (
                more,
                cursor.map(|before| json!({"before":before})),
                &window[start..],
            )
        };
        let items: Vec<Item> = coalesce(selected, true)
            .into_iter()
            .filter(|item| self.kinds.iter().any(|kind| kind == item.kind))
            .collect();
        let more = more && next.is_some();
        json!({
            "items": items.iter().map(|item| item.view(self.chars)).collect::<Vec<_>>(),
            "reached_start": reached_start && !more,
            "more": more,
            "next": if more { next } else { None },
        })
    }
}

struct Item {
    seq: u64,
    to: u64,
    kind: &'static str,
    text: String,
    tool: Option<Map<String, Value>>,
}

impl Item {
    fn view(&self, chars: usize) -> Value {
        let mut view = json!({"seq": self.seq, "to": self.to, "kind": self.kind});
        if !self.text.is_empty() {
            let mut text: String = self.text.chars().take(chars).collect();
            if text.len() < self.text.len() {
                view["truncated"] = json!(true);
            } else {
                text = self.text.clone();
            }
            view["text"] = json!(text);
        }
        if let Some(tool) = &self.tool {
            view["tool"] = Value::Object(tool.clone());
        }
        view
    }
}

fn block_text(block: &Value) -> String {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        Some(other) => format!("[{other}]"),
        None => String::new(),
    }
}

fn coalesce(window: &[Envelope], fold_tools: bool) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut tools: HashMap<String, usize> = HashMap::new();
    for envelope in window {
        let seq = envelope.seq;
        let (kind, text, tool): (&'static str, String, Option<Map<String, Value>>) = match &envelope
            .event
        {
            Event::Update { update } => match update.get("sessionUpdate").and_then(Value::as_str) {
                Some("user_message_chunk") => ("user", block_text(&update["content"]), None),
                Some("agent_message_chunk") => ("agent", block_text(&update["content"]), None),
                Some(tag @ ("tool_call" | "tool_call_update")) => {
                    let id = update
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let mut fields = Map::new();
                    for (from, to) in [("title", "title"), ("status", "status"), ("kind", "kind")] {
                        if let Some(value) = update.get(from).filter(|v| !v.is_null()) {
                            fields.insert(to.into(), value.clone());
                        }
                    }
                    if fold_tools && let Some(index) = tools.get(&id).copied() {
                        let item = &mut items[index];
                        item.to = seq;
                        if let Some(tool) = &mut item.tool {
                            tool.extend(fields);
                        }
                        continue;
                    }
                    if tag == "tool_call_update" && id.is_empty() {
                        continue;
                    }
                    fields.insert("id".into(), json!(id));
                    tools.insert(id, items.len());
                    ("tool", String::new(), Some(fields))
                }
                Some("plan") => {
                    let text = update
                        .get("entries")
                        .and_then(Value::as_array)
                        .map(|entries| {
                            entries
                                .iter()
                                .map(|entry| {
                                    format!(
                                        "[{}] {}",
                                        entry["status"].as_str().unwrap_or("?"),
                                        entry["content"].as_str().unwrap_or_default()
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    ("plan", text, None)
                }
                _ => continue,
            },
            Event::PermissionRequest { tool_call, .. } => (
                "permission",
                tool_call["title"].as_str().unwrap_or_default().to_owned(),
                None,
            ),
            Event::Lifecycle { status, detail } => (
                "lifecycle",
                format!(
                    "{}{}",
                    serde_json::to_value(status)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    detail
                        .as_ref()
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default()
                ),
                None,
            ),
            Event::TurnEnd { stop_reason, .. } => ("turn_end", stop_reason.clone(), None),
            Event::PermissionResolved { .. } => continue,
        };
        if matches!(kind, "user" | "agent")
            && let Some(last) = items.last_mut()
            && last.kind == kind
        {
            last.text.push_str(&text);
            last.to = seq;
            continue;
        }
        items.push(Item {
            seq,
            to: seq,
            kind,
            text,
            tool,
        });
    }
    items
}

// ---------------------------------------------------------------------------
// Contained runtime

struct Sandbox {
    prlimit: PathBuf,
    bwrap: PathBuf,
    bun: PathBuf,
}

/// A runtime override, then the path the Nix build pinned, then PATH.
fn executable(variable: &str, pinned: Option<&str>, name: &str) -> Option<PathBuf> {
    let found = std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| pinned.map(PathBuf::from))
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .map(|directory| directory.join(name))
                .find(|path| path.is_file())
        })?;
    std::fs::canonicalize(found).ok()
}

impl Sandbox {
    fn resolve() -> Result<Self, &'static str> {
        Ok(Self {
            prlimit: executable(
                "COWBOY_AGENT_SANDBOX_PRLIMIT",
                option_env!("COWBOY_AGENT_SANDBOX_PRLIMIT"),
                "prlimit",
            )
            .ok_or("runtime_unavailable")?,
            bwrap: executable(
                "COWBOY_AGENT_SANDBOX_BWRAP",
                option_env!("COWBOY_AGENT_SANDBOX_BWRAP"),
                "bwrap",
            )
            .ok_or("runtime_unavailable")?,
            bun: executable(
                "COWBOY_AGENT_SANDBOX_BUN",
                option_env!("COWBOY_AGENT_SANDBOX_BUN"),
                "bun",
            )
            .ok_or("runtime_unavailable")?,
        })
    }

    async fn execute(&self, program: &Program, host: &impl Host) -> Value {
        let Ok(directory) = private_directory() else {
            return error("runtime_unavailable");
        };
        let result = self.run(&directory, program, host).await;
        let _ = std::fs::remove_dir_all(&directory);
        result
    }

    async fn run(&self, directory: &Path, program: &Program, host: &impl Host) -> Value {
        let extension = if program.typescript { "ts" } else { "js" };
        let source = if program.module {
            program.code.clone()
        } else {
            format!(
                "export default async function ({{ cowboy }}) {{\n{}\n}}\n",
                program.code
            )
        };
        let entry = format!("/sandbox/program.{extension}");
        if std::fs::write(directory.join("runner.js"), RUNNER).is_err()
            || std::fs::write(directory.join(format!("program.{extension}")), source).is_err()
        {
            return error("runtime_unavailable");
        }
        let mut command = tokio::process::Command::new(&self.prlimit);
        command
            .args([
                "--cpu=4",
                "--data=536870912",
                "--nofile=64",
                "--fsize=1048576",
                "--core=0",
                "--",
            ])
            .arg(&self.bwrap)
            .args([
                "--unshare-all",
                "--die-with-parent",
                "--new-session",
                "--cap-drop",
                "ALL",
                "--clearenv",
                "--setenv",
                "HOME",
                "/tmp",
                "--setenv",
                "BUN_RUNTIME_TRANSPILER_CACHE_PATH",
                "0",
                "--ro-bind-try",
                "/nix/store",
                "/nix/store",
                "--ro-bind-try",
                "/usr",
                "/usr",
                "--ro-bind-try",
                "/lib",
                "/lib",
                "--ro-bind-try",
                "/lib64",
                "/lib64",
                "--ro-bind",
            ])
            .arg(directory)
            .args([
                "/sandbox",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                // tmpfs pages are host memory outside every process limit:
                // /dev stays read-only and /tmp holds at most 1 MiB.
                "--remount-ro",
                "/dev",
                "--size",
                "1048576",
                "--tmpfs",
                "/tmp",
                "--chdir",
                "/sandbox",
                "--",
            ])
            .arg(&self.bun)
            .args(["--no-install", "--smol", "run", "/sandbox/runner.js"])
            .arg(&entry)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let Ok(mut child) = command.spawn() else {
            return error("runtime_unavailable");
        };
        let (Some(mut stdin), Some(stdout), Some(mut stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return error("runtime_unavailable");
        };
        let diagnostics = tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = (&mut stderr).take(8192).read_to_end(&mut bytes).await;
            // Keep draining so the runtime never blocks on a full pipe.
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
            bytes
        });
        let deadline = Instant::now() + Duration::from_millis(program.timeout_ms);
        let broker = async {
            let mut reader = BufReader::new(stdout);
            let mut calls = 0usize;
            let mut traffic = 0usize;
            loop {
                let mut line = Vec::new();
                let read = (&mut reader)
                    .take(MAX_REPLY_BYTES as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .await;
                match read {
                    Ok(0) | Err(_) => return None,
                    Ok(count) if count > MAX_REPLY_BYTES => return Some(error("budget_exceeded")),
                    // A truncated final line: the runtime died mid-frame.
                    Ok(_) if !line.ends_with(b"\n") => return None,
                    Ok(count) => traffic += count,
                }
                let Ok(frame) = serde_json::from_slice::<Value>(&line) else {
                    return Some(error("runtime_protocol"));
                };
                if let Some(done) = frame.get("done").and_then(Value::as_bool) {
                    return Some(finish(done, frame));
                }
                let (Some(id), Some(op)) = (
                    frame.get("id").and_then(Value::as_u64),
                    frame.get("op").and_then(Value::as_str),
                ) else {
                    return Some(error("runtime_protocol"));
                };
                calls += 1;
                let reply = if calls > MAX_CALLS {
                    json!({"id": id, "error": {"code": "call_limit"}})
                } else {
                    let args = frame.get("args").cloned().unwrap_or_else(|| json!({}));
                    let echo = args.to_string();
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    match host.call(op, args, remaining).await {
                        Ok(value) => json!({"id": id, "ok": value}),
                        Err(code) => json!({"id": id, "error": call_error(op, &echo, code)}),
                    }
                };
                let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
                if bytes.len() > MAX_REPLY_BYTES {
                    bytes = serde_json::to_vec(
                        &json!({"id": id, "error": {"code": "reply_too_large"}}),
                    )
                    .unwrap_or_default();
                }
                bytes.push(b'\n');
                traffic += bytes.len();
                if traffic > MAX_BROKER_BYTES {
                    return Some(error("budget_exceeded"));
                }
                if stdin.write_all(&bytes).await.is_err() {
                    return None;
                }
            }
        };
        let outcome = tokio::time::timeout_at(deadline, broker).await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        match outcome {
            Err(_) => error("timeout"),
            Ok(Some(value)) => value,
            Ok(None) => {
                let stderr = diagnostics.await.unwrap_or_default();
                let mut value = error("runtime_failed");
                let text = String::from_utf8_lossy(&stderr);
                if !text.trim().is_empty() {
                    value["error"]["message"] = json!(text.chars().take(1000).collect::<String>());
                }
                value
            }
        }
    }
}

/// A failed operation keeps its stable code and gains a bounded message that
/// names the operation and, for argument errors, what the broker received.
/// Without it agents retried the same malformed call under a bare code.
fn call_error(op: &str, echo: &str, error: String) -> Value {
    // An operation may append "code: detail"; the code alone stays stable.
    let (code, detail) = match error.split_once(": ") {
        Some((code, detail)) => (code.to_owned(), format!(": {}", bounded(detail, 300))),
        None => (error, String::new()),
    };
    let hint = match code.as_str() {
        "invalid_argument" | "not_found" => {
            let namespace = op
                .split('.')
                .next()
                .filter(|namespace| crate::agent_api::HELP_TOPICS.contains(namespace))
                .unwrap_or("index");
            format!(
                "; received {}; see cowboy_read({{help:\"{namespace}\"}})",
                bounded(echo, 300)
            )
        }
        "invalid_help" => format!("; topics: {}", crate::agent_api::HELP_TOPICS.join(", ")),
        "unknown_operation" => "; discover names with cowboy.control.list(domain)".to_owned(),
        "selection_too_large" => {
            "; pass {describe:true} first, then a deeper path or smaller limit/chars".to_owned()
        }
        "page_required" => {
            "; pass an explicit limit and continue from the returned cursor".to_owned()
        }
        "text_window_required" => {
            "; pass {offset:0,chars:1000} (max 4000) and continue from range.next".to_owned()
        }
        _ => String::new(),
    };
    json!({"code": code, "message": format!("{op}: {code}{detail}{hint}")})
}

/// The three largest top-level fields (or array items) of an oversized
/// result, so the caller can project it in one retry instead of guessing.
fn largest_fields(result: &Value) -> Value {
    let size = |value: &Value| serde_json::to_vec(value).map_or(0, |bytes| bytes.len());
    let mut fields: Vec<(String, usize)> = match result {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| (bounded(key, 40), size(value)))
            .collect(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, value)| (format!("[{index}]"), size(value)))
            .collect(),
        _ => Vec::new(),
    };
    fields.sort_by_key(|field| std::cmp::Reverse(field.1));
    fields.truncate(3);
    Value::Array(
        fields
            .into_iter()
            .map(|(field, bytes)| json!({"field":field,"bytes":bytes}))
            .collect(),
    )
}

fn bounded(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

fn finish(done: bool, mut frame: Value) -> Value {
    let logs = frame
        .get("logs")
        .and_then(Value::as_str)
        .filter(|logs| !logs.is_empty())
        .map(|logs| logs[..logs.floor_char_boundary(MAX_LOG_BYTES.min(logs.len()))].to_owned());
    let mut value = if done {
        let result = frame.get_mut("value").map_or(Value::Null, Value::take);
        let size = serde_json::to_vec(&result).map_or(usize::MAX, |bytes| bytes.len());
        if size > MAX_RESULT_BYTES {
            json!({"schema":1,"state":"error","error":{"code":"result_too_large","bytes":size,
                "limit":MAX_RESULT_BYTES,"largest":largest_fields(&result),
                "message":"return a projection: map to the needed fields or pass a smaller limit"}})
        } else {
            json!({"schema":1,"state":"ok","value":result})
        }
    } else {
        // Only a bounded code and message cross back; a thrown object is
        // program-controlled and must not bypass the result budget.
        let failure = frame.get("error");
        let code = failure
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            .filter(|code| {
                !code.is_empty()
                    && code.len() <= 64
                    && code
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            })
            .unwrap_or("program_error");
        let mut error = json!({"code": code});
        if let Some(message) = failure
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
        {
            error["message"] = json!(message.chars().take(2000).collect::<String>());
        }
        json!({"schema":1,"state":"error","error":error})
    };
    if let Some(logs) = logs {
        value["logs"] = json!(logs);
    }
    value
}

fn private_directory() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt as _;
    let mut entropy = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut entropy);
    let name: String = entropy.iter().map(|b| format!("{b:02x}")).collect();
    let directory = std::env::temp_dir().join(format!("cowboy-agent-{name}"));
    std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
    Ok(directory)
}

#[cfg(test)]
mod tests;

//! Explicit effects. Generated read programs cannot reach this dispatcher.
//! Each domain retains its own durable identity and recovery contract.
use super::*;
use crate::managed_calls::Request;
use crate::store::draft_documents::{DraftChange, DraftMutation, DraftResult};
use serde::Deserialize;

// Serialize this small, infrequent configuration surface across its two
// domain writers so one operation cannot race into About and Zed together.
static PREFERENCE_WRITES: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallInput {
    #[serde(default = "auto")]
    provider: String,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    focus: Option<String>,
    #[serde(default)]
    design: bool,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    capability: Option<String>,
    #[serde(default)]
    child_session_id: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    fresh: bool,
    #[serde(default)]
    notify: bool,
}

fn auto() -> String {
    "auto".into()
}

/// A refused call names the violated rule. Messages are fixed text: they
/// never echo instructions, paths or other caller input.
#[derive(Debug)]
pub(super) struct CallRefusal {
    pub code: &'static str,
    pub message: &'static str,
}

const fn refuse(message: &'static str) -> CallRefusal {
    CallRefusal {
        code: "invalid_argument",
        message,
    }
}

impl CallRefusal {
    fn value(&self) -> Value {
        json!({"schema":1,"state":"error","error":{"code":self.code,"message":self.message}})
    }
}

fn input_refusal(error: &crate::managed_calls::InputError) -> CallRefusal {
    use crate::managed_calls::InputError;
    refuse(match error {
        InputError::RequestTooLarge => "request exceeds the size limit",
        InputError::InvalidInstruction => "instruction must be non-empty text up to 128 KiB",
        InputError::InvalidContext => {
            "root must be an absolute directory other than /; files must be up to 32 distinct relative paths (images: up to 4, and image.edit needs files or child_session_id)"
        }
        InputError::InvalidConversation => "child_session_id is not a valid session id",
        InputError::InvalidLabels => "title and reason must be short single-line text",
        InputError::InvalidOutput => "the requested output format is not supported",
        InputError::InvalidJson
        | InputError::InvalidContract
        | InputError::UnsupportedSchema
        | InputError::InvalidRequestId => "operation must be a valid id; check argument types",
    })
}

fn call_action(action: &str, args: Value, operation: &str) -> Result<Action, CallRefusal> {
    let input: CallInput = serde_json::from_value(args)
        .map_err(|_| refuse("unknown or mistyped argument; see cowboy_read({help:\"actions\"})"))?;
    let review = action == "review.start";
    if review
        && (input.instruction.is_some()
            || input.purpose.is_some()
            || input.capability.is_some()
            || input.child_session_id.is_some())
    {
        return Err(refuse(
            "review.start takes focus, not instruction, purpose, capability or child_session_id",
        ));
    }
    if !review
        && (input.focus.is_some()
            || input.design
            || input.task.is_some()
            || input.base.is_some()
            || input.fresh)
    {
        return Err(refuse(
            "focus, design, task, base and fresh belong to review.start",
        ));
    }
    if input.task.is_some() != input.base.is_some() {
        return Err(refuse("task and base must be given together"));
    }
    if input.fresh && input.task.is_none() {
        return Err(refuse("fresh requires task and base"));
    }
    if input.capability.is_some() && input.purpose.is_some() {
        return Err(refuse("pass capability or purpose, not both"));
    }
    if input
        .base
        .as_deref()
        .is_some_and(|base| !crate::managed_calls::review::valid_base(base))
    {
        return Err(refuse(
            "base must be a full 40- or 64-hex commit id, not a short or symbolic ref",
        ));
    }
    // The request contract reports root and file faults together; split the
    // common mistake of attaching absolute paths instead of root-relative ones.
    if input
        .files
        .iter()
        .any(|file| !crate::managed_calls::relative_file(file))
    {
        return Err(refuse(
            "files must be paths relative to root, without a leading /, '.' or '..' segments; capture files outside root into the worktree first",
        ));
    }
    if review && input.task.is_some() && input.root.is_none() {
        return Err(refuse("a base-bound review needs root"));
    }
    // A description passed as task fails later as an opaque context error.
    if input
        .task
        .as_deref()
        .is_some_and(|task| !crate::managed_calls::valid_id(task))
    {
        return Err(refuse(
            "task must be a short id (letters, digits, '-', '_' or '.', up to 128); put the description in focus",
        ));
    }
    let purpose = if review {
        if input.design {
            "design_review"
        } else {
            "review"
        }
        .to_owned()
    } else if let Some(capability) = input.capability {
        match capability.as_str() {
            "image.generate" => "image_generate",
            "image.edit" => "image_edit",
            _ => {
                return Err(CallRefusal {
                    code: "unsupported_capability",
                    message: "capability must be image.generate or image.edit",
                });
            }
        }
        .to_owned()
    } else {
        input.purpose.unwrap_or_else(|| "analysis".into())
    };
    let instruction = if review {
        crate::managed_calls::review_instruction(
            input.design,
            input.focus.as_deref().unwrap_or(""),
            input.base.as_deref(),
        )
    } else {
        input
            .instruction
            .ok_or(refuse("calls.start needs instruction"))?
    };
    let mut labels = std::collections::BTreeMap::new();
    if let Some(title) = input.title {
        labels.insert("title", title);
    }
    if let Some(reason) = input.reason {
        labels.insert("selection_reason", reason);
    }
    if input.notify {
        labels.insert(crate::managed_calls::review::NOTIFY, "parent".to_owned());
    }
    let mut output = json!({"format":"text"});
    if let (Some(task), Some(base)) = (input.task, input.base) {
        labels.insert(crate::managed_calls::review::TASK, task);
        labels.insert(crate::managed_calls::review::BASE, base);
        labels.insert(
            crate::managed_calls::review::REUSE,
            (!input.fresh).to_string(),
        );
        output =
            json!({"format":"json_schema","schema":crate::managed_calls::review::output_schema()});
    }
    let conversation = match input.child_session_id {
        Some(child) => json!({"mode":"continue","child_session_id":child}),
        None => json!({"mode":"fresh"}),
    };
    let request = Request::parse(
        serde_json::to_string(&json!({
            "schema":1,"request_id":operation,"purpose":purpose,"instruction":instruction,
            "context":{"scope":"current-worktree","files":input.files,"root":input.root},
            "access":"read-only","conversation":conversation,"labels":labels,"output":output
        }))
        .map_err(|_| refuse("arguments could not be encoded"))?
        .as_bytes(),
    )
    .map_err(|error| input_refusal(&error))?;
    let action = Action::Start {
        provider: input.provider,
        request: Box::new(request),
        wait_ms: 0,
        preset: input.preset,
    };
    action
        .validate()
        .map_err(|_| refuse("provider or preset is not a valid id"))?;
    Ok(action)
}

pub(super) fn document_operation(parent: &str, operation: &str) -> String {
    format!(
        "agent-{}",
        crate::admin::hex_sha256(
            &serde_json::to_vec(&(parent, operation)).expect("strings serialize")
        )
    )
}

fn review_operation(owner: &str, parent: &str, operation: &str) -> String {
    document_operation(&format!("{owner}:{parent}"), operation)
}

fn success(value: Value) -> Value {
    json!({"schema":1,"state":"ok","value":value})
}

fn call_result(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.remove("schema");
        map.remove("next");
    }
    if value["state"] == "error" && value.get("call_id").is_none() {
        value
    } else {
        success(value)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentInput {
    document_id: String,
    expected_revision: i64,
    change: DraftChange,
}

fn document_input(args: Value) -> Result<DocumentInput, &'static str> {
    // The legacy product writer defaults omitted attachments to an empty list.
    // Agent replacement must be explicit; ordinary edits use write_text.
    if args["change"]["type"] == "write" && args["change"].get("attachments").is_none() {
        return Err("invalid_argument");
    }
    serde_json::from_value(args).map_err(|_| "invalid_argument")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptInput {
    domain: String,
    operation: String,
}

/// Derive the current product permission; a session owner id alone is not a
/// grant to write and a disabled/deleted user must never retain access.
async fn document_owner(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    meta: &SessionMeta,
) -> Result<String, &'static str> {
    let owner = meta.owner_user_id.as_deref().ok_or("permission_denied")?;
    let store = store.ok_or("unavailable")?;
    let user = store
        .user_by_id(owner)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("permission_denied")?;
    let principal = super::super::product_principal(hub, &user);
    if meta.system
        || user.disabled_at_ms.is_some()
        || !principal.can_reorder()
        || hub
            .session_info(&meta.id)
            .is_none_or(|info| info.meta.closing || info.meta.owner_user_id != meta.owner_user_id)
    {
        return Err("permission_denied");
    }
    Ok(owner.to_owned())
}

pub(super) async fn preference_owner(
    state: &AppState,
    meta: &SessionMeta,
) -> Result<String, &'static str> {
    active_preference_owner(&state.hub, state.store.as_ref(), meta).await
}

pub(in crate::server) async fn read_parent(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    meta: &SessionMeta,
) -> Result<(), &'static str> {
    if hub.session_info(&meta.id).is_none_or(|info| {
        info.meta.system || info.meta.closing || info.meta.owner_user_id != meta.owner_user_id
    }) {
        return Err("permission_denied");
    }
    if meta.owner_user_id.is_some() {
        active_preference_owner(hub, store, meta).await?;
    }
    Ok(())
}

pub(super) async fn account_owner(
    state: &AppState,
    meta: &SessionMeta,
) -> Result<String, &'static str> {
    document_owner(&state.hub, state.store.as_ref(), meta).await
}

async fn active_preference_owner(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    meta: &SessionMeta,
) -> Result<String, &'static str> {
    let owner = meta.owner_user_id.as_deref().ok_or("permission_denied")?;
    let user = store
        .ok_or("unavailable")?
        .user_by_id(owner)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("permission_denied")?;
    if meta.system
        || user.disabled_at_ms.is_some()
        || hub
            .session_info(&meta.id)
            .is_none_or(|info| info.meta.closing || info.meta.owner_user_id != meta.owner_user_id)
    {
        return Err("permission_denied");
    }
    Ok(owner.to_owned())
}

pub(in crate::server) async fn invoke(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    ledger: &Ledger,
    action: &str,
    args: Value,
    operation: Option<&str>,
) -> Value {
    match execute(state, meta, ledger, action, args, operation).await {
        Ok(value) => value,
        Err(code) => error(code),
    }
}

async fn execute(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    ledger: &Ledger,
    action: &str,
    args: Value,
    operation: Option<&str>,
) -> Result<Value, &'static str> {
    if action == "receipts.get" {
        if operation.is_some() {
            return Err("invalid_argument");
        }
        let input: ReceiptInput = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
        if !crate::managed_calls::valid_id(&input.operation) {
            return Err("invalid_argument");
        }
        return match input.domain.as_str() {
            "control" => Ok(success(
                super::control::receipt(state, meta, &input.operation).await?,
            )),
            "logins" => {
                let owner = account_owner(state, meta).await?;
                Ok(success(
                    super::super::provider_accounts::login::operation_receipt(
                        state,
                        &owner,
                        &document_operation(&meta.id, &input.operation),
                    )
                    .await?,
                ))
            }
            "accounts" => {
                let owner = account_owner(state, meta).await?;
                let receipt = state
                    .store
                    .as_ref()
                    .ok_or("unavailable")?
                    .provider_account_receipt(
                        &owner,
                        &document_operation(&meta.id, &input.operation),
                    )
                    .await
                    .map_err(|_| "unavailable")?;
                Ok(success(match receipt {
                    Some(receipt) => super::super::provider_accounts::receipt_value(&receipt),
                    None => json!({"status":"unknown"}),
                }))
            }
            "preferences" => {
                let owner = preference_owner(state, meta).await?;
                let receipt = state
                    .store
                    .as_ref()
                    .ok_or("unavailable")?
                    .preference_receipt(&owner, &document_operation(&meta.id, &input.operation))
                    .await
                    .map_err(|_| "unavailable")?;
                Ok(success(match receipt {
                    Some(prefs) => json!({"status":"committed","preferences":prefs}),
                    None if state.hub.review_default_receipt(&review_operation(
                        &owner,
                        &meta.id,
                        &input.operation,
                    )) =>
                    {
                        json!({"status":"accepted","scope":"zed","durable_receipt":false})
                    }
                    None => json!({"status":"unknown"}),
                }))
            }
            "calls" => {
                let record = ledger
                    .observe(&input.operation)
                    .await
                    .map_err(|_| "unavailable")?;
                let record = match record {
                    Some(record) => Some(record),
                    None => ledger
                        .inspect(&input.operation)
                        .await
                        .map_err(|_| "unavailable")?,
                };
                match record {
                    Some(record) => Ok(call_result(
                        super::super::managed_calls::dispatch_calls(
                            state,
                            meta,
                            ledger,
                            Action::Inspect {
                                call_id: record.call_id,
                            },
                        )
                        .await,
                    )),
                    None => Ok(success(json!({"status":"unknown"}))),
                }
            }
            "documents" => {
                let owner = document_owner(&state.hub, state.store.as_ref(), meta).await?;
                let id = state
                    .store
                    .as_ref()
                    .ok_or("unavailable")?
                    .draft_operation_document(
                        &owner,
                        &document_operation(&meta.id, &input.operation),
                    )
                    .await
                    .map_err(|_| "unavailable")?;
                Ok(success(match id {
                    Some(id) => json!({"status":"committed","document_id":id}),
                    None => json!({"status":"unknown"}),
                }))
            }
            _ => Err("invalid_argument"),
        };
    }
    let operation = operation
        .filter(|id| crate::managed_calls::valid_id(id))
        .ok_or("invalid_operation")?;
    match action {
        "control.apply" => Ok(success(
            super::control::apply(state, meta, args, operation).await?,
        )),
        "accounts.transition_plan" => {
            let mut input: crate::store::provider_accounts::transitions::Request =
                serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            if !crate::managed_calls::valid_id(&input.target_login_operation) {
                return Err("invalid_argument");
            }
            input.target_login_operation =
                document_operation(&meta.id, &input.target_login_operation);
            let owner = account_owner(state, meta).await?;
            let operation = document_operation(&meta.id, operation);
            super::super::provider_accounts::transitions::plan(state, &owner, &operation, input)
                .await?;
            Ok(success(
                super::super::provider_accounts::transitions::status(state, &owner, &operation)
                    .await?,
            ))
        }
        "accounts.transition_reconcile" => {
            if args.as_object().is_none_or(|args| !args.is_empty()) {
                return Err("invalid_argument");
            }
            let owner = account_owner(state, meta).await?;
            Ok(success(
                super::super::provider_accounts::transitions::reconcile(
                    state,
                    &owner,
                    &document_operation(&meta.id, operation),
                )
                .await?,
            ))
        }
        "accounts.reconcile" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Resume {
                account_id: Option<crate::provider_account::ProviderAccountId>,
            }
            let input: Resume = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            let owner = account_owner(state, meta).await?;
            if let Some(account) = input.account_id {
                return Ok(success(
                    super::super::provider_accounts::lifecycle::resume(state, &owner, &account)
                        .await?,
                ));
            }
            Ok(success(
                super::super::provider_accounts::reconcile(
                    state,
                    &owner,
                    &document_operation(&meta.id, operation),
                )
                .await?,
            ))
        }
        "accounts.login_cancel_request" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Reference {
                account_id: crate::provider_account::ProviderAccountId,
                request_id: String,
            }
            let input: Reference = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            let owner = account_owner(state, meta).await?;
            Ok(success(
                super::super::provider_accounts::login::continue_request(
                    state,
                    &owner,
                    &input.account_id,
                    &input.request_id,
                    true,
                )
                .await?,
            ))
        }
        "accounts.login_cancel" => {
            if args.as_object().is_none_or(|args| !args.is_empty()) {
                return Err("invalid_argument");
            }
            let owner = account_owner(state, meta).await?;
            Ok(success(
                super::super::provider_accounts::login::cancel_operation(
                    state,
                    &owner,
                    &document_operation(&meta.id, operation),
                )
                .await?,
            ))
        }
        "accounts.revoke" => {
            let input = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            let owner = account_owner(state, meta).await?;
            let value = super::super::provider_accounts::lifecycle::start(
                state,
                &owner,
                &document_operation(&meta.id, operation),
                input,
            )
            .await?;
            if value.get("error").is_some() {
                Ok(json!({"state":"error","error":value["error"],"current":value["current"]}))
            } else {
                Ok(success(value))
            }
        }
        "accounts.login_start" => {
            let input = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            let owner = account_owner(state, meta).await?;
            Ok(success(
                super::super::provider_accounts::login::start_operation(
                    state,
                    &owner,
                    &document_operation(&meta.id, operation),
                    input,
                )
                .await?,
            ))
        }
        "accounts.update" => {
            let input: crate::store::provider_accounts::Mutation =
                serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            let owner = account_owner(state, meta).await?;
            let value = super::super::provider_accounts::update_value(
                state,
                &owner,
                &document_operation(&meta.id, operation),
                &input,
            )
            .await?;
            if value.get("error").is_some() {
                Ok(json!({"state":"error","error":value["error"],"current":value["current"]}))
            } else {
                Ok(success(value))
            }
        }
        "preferences.update" => {
            let _guard = PREFERENCE_WRITES.lock().await;
            if args["scope"] == "zed" {
                let input: crate::review_directory::Change =
                    serde_json::from_value(args).map_err(|_| "invalid_argument")?;
                let owner = document_owner(&state.hub, state.store.as_ref(), meta).await?;
                let review_operation = review_operation(&owner, &meta.id, operation);
                let operation = document_operation(&meta.id, operation);
                // Prevent reusing an About operation for this distinct domain.
                if state
                    .store
                    .as_ref()
                    .ok_or("unavailable")?
                    .preference_receipt(&owner, &operation)
                    .await
                    .map_err(|_| "unavailable")?
                    .is_some()
                {
                    return Err("operation_conflict");
                }
                state.hub.review_directory_update(
                    &review_defaults::actor(&owner),
                    &meta.id,
                    &review_operation,
                    &input,
                )?;
                return Ok(success(
                    json!({"status":"accepted","scope":"zed","durable_receipt":false}),
                ));
            }
            let input: crate::store::preferences::PreferenceChange =
                serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            let owner = preference_owner(state, meta).await?;
            if state
                .hub
                .review_default_receipt(&review_operation(&owner, &meta.id, operation))
            {
                return Err("operation_conflict");
            }
            let value = super::super::preferences::update_value(
                state,
                &owner,
                &document_operation(&meta.id, operation),
                &input,
            )
            .await?;
            if value.get("error").is_some() {
                Ok(json!({"state":"error","error":value["error"],"current":value["current"]}))
            } else {
                Ok(success(value))
            }
        }
        "calls.start" | "review.start" => {
            let action = match call_action(action, args, operation) {
                Ok(action) => action,
                Err(refusal) => return Ok(refusal.value()),
            };
            Ok(call_result(
                super::super::managed_calls::dispatch_calls(state, meta, ledger, action).await,
            ))
        }
        "calls.cancel" => {
            only_keys(&args, &["call_id"]).map_err(|_| "invalid_argument")?;
            let id = arg_str(&args, "call_id").map_err(|_| "invalid_argument")?;
            // Cancellation's idempotent identity is the immutable call itself.
            if operation != id {
                return Err("operation_must_be_call_id");
            }
            Ok(call_result(
                super::super::managed_calls::dispatch_calls(
                    state,
                    meta,
                    ledger,
                    Action::Cancel { call_id: id.into() },
                )
                .await,
            ))
        }
        "documents.mutate" => {
            let input = document_input(args)?;
            let owner = document_owner(&state.hub, state.store.as_ref(), meta).await?;
            let mutation = DraftMutation {
                operation_id: document_operation(&meta.id, operation),
                document_id: input.document_id,
                expected_revision: input.expected_revision,
                change: input.change,
            };
            let folders = state.hub.sync_value("folders")["folders"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| {
                    serde_json::from_value::<crate::session_folders::SessionFolder>(v).ok()
                })
                .filter(|f| f.owner_user_id.as_deref().is_none_or(|id| id == owner))
                .collect::<Vec<_>>();
            match state
                .store
                .as_ref()
                .ok_or("unavailable")?
                .mutate_draft_document_in_workspace(&owner, &mutation, &folders)
                .await
                .map_err(|_| "outcome_unknown")?
            {
                DraftResult::Applied(document) => {
                    let view = document_view(&document, false);
                    // Use the existing product notification projection, including tombstones.
                    let mut notification = view.clone();
                    notification["deleted"] = json!(document.deleted);
                    state.hub.announce_draft(&owner, notification);
                    Ok(success(
                        json!({"status":"committed","document":view,"deleted":document.deleted}),
                    ))
                }
                DraftResult::Conflict(current) => Ok(
                    json!({"state":"error","error":{"code":"revision_conflict"},"current":current.as_ref().map(|d|document_view(d,false))}),
                ),
                DraftResult::Invalid(message) => Ok(
                    json!({"state":"error","error":{"code":"invalid_mutation","message":message}}),
                ),
            }
        }
        _ => Err("unknown_operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_replacement_requires_an_explicit_list() {
        let mut input = json!({"document_id":"doc","expected_revision":1,
            "change":{"type":"write","body":"edited"}});
        assert!(document_input(input.clone()).is_err());
        input["change"]["attachments"] = json!([]);
        assert!(document_input(input).is_ok());
        assert!(
            document_input(json!({"document_id":"doc","expected_revision":1,
            "change":{"type":"write_text","body":"edited"}}))
            .is_ok()
        );
    }

    #[tokio::test]
    async fn document_permission_uses_current_account_role_and_parent_ownership() {
        use crate::admin::{AdminRole, PERMISSIONS_SETTING, PermissionPolicy};
        use crate::core::{Hub, SessionOrigin, SessionRegistration};
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let user = crate::store::ProductUser {
            id: "0123456789abcdef0123456789abcdef".into(),
            username: "writer".into(),
            password_algo: "fixture".into(),
            password_hash: "unused".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
            disabled_at_ms: None,
        };
        store.insert_user(&user).await.unwrap();
        let hub = Hub::new();
        hub.create_session(SessionRegistration {
            id: "parent".into(),
            provider: "codex".into(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "machine".into(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/tasks/project".into(),
            title: String::new(),
            origin: SessionOrigin::Web,
            system: false,
            owner_user_id: Some(user.id.clone()),
            owner_username: Some(user.username.clone()),
        });
        let mut meta = hub.session_info("parent").unwrap().meta;
        assert_eq!(read_parent(&hub, Some(&store), &meta).await, Ok(()));
        // Every active product account may arrange its own display, even a viewer.
        assert_eq!(
            active_preference_owner(&hub, Some(&store), &meta)
                .await
                .unwrap(),
            user.id
        );
        assert_eq!(
            document_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        let mut policy = PermissionPolicy::default();
        policy.upsert_grant(&user.username, AdminRole::Operator);
        hub.set_setting(
            PERMISSIONS_SETTING.into(),
            serde_json::to_value(&policy).unwrap(),
        );
        assert_eq!(
            document_owner(&hub, Some(&store), &meta).await.unwrap(),
            user.id
        );
        store.set_user_disabled_at(&user.id, Some(2)).await.unwrap();
        assert_eq!(
            read_parent(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        assert_eq!(
            active_preference_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        assert_eq!(
            document_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        store.set_user_disabled_at(&user.id, None).await.unwrap();
        hub.set_closing("parent", true);
        assert_eq!(
            read_parent(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        hub.set_closing("parent", false);
        assert_eq!(read_parent(&hub, Some(&store), &meta).await, Ok(()));
        meta.id = "other-parent".into();
        assert_eq!(
            active_preference_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        assert_eq!(
            document_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        meta.id = "parent".into();
        meta.owner_user_id = Some("other-owner".into());
        assert_eq!(
            active_preference_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        assert_eq!(
            document_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        meta.owner_user_id = None;
        assert_eq!(
            read_parent(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
        assert_eq!(
            document_owner(&hub, Some(&store), &meta).await,
            Err("permission_denied")
        );
    }
    #[test]
    fn call_inputs_never_supply_authority_or_change_read_only_profile() {
        for args in [
            json!({"instruction":"inspect","parent":"victim"}),
            json!({"instruction":"inspect","access":"write"}),
            json!({"focus":"inspect","purpose":"analysis"}),
        ] {
            assert!(call_action("review.start", args, "op").is_err());
        }
        let Action::Start {
            request, wait_ms, ..
        } = call_action(
            "review.start",
            json!({"focus":"check ownership","root":"/tasks/example","title":"Ownership review","reason":"Ample model quota"}),
            "op",
        )
        .unwrap()
        else {
            panic!()
        };
        assert_eq!(wait_ms, 0);
        assert_eq!(request.request_id, "op");
        assert!(request.instruction.contains("check ownership"));
        assert_eq!(request.labels["title"], "Ownership review");
        assert_eq!(request.labels["selection_reason"], "Ample model quota");
        assert!(!request.labels.contains_key("notify"));
        let Action::Start { request, .. } = call_action(
            "calls.start",
            json!({"instruction":"inspect","notify":true}),
            "op",
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(request.labels["notify"], "parent");
        assert!(matches!(
            request.access,
            crate::managed_calls::Access::ReadOnly
        ));
    }
    #[test]
    fn call_refusals_name_the_rule_without_echoing_input() {
        let message =
            |action: &str, args: Value| call_action(action, args, "op").unwrap_err().message;
        let root = "/srv/private/source";
        let short = message(
            "review.start",
            json!({"root":root,"base":"1ef0ea6f","task":"t"}),
        );
        assert!(short.contains("full 40- or 64-hex"), "{short}");
        let task = message(
            "review.start",
            json!({"root":root,"base":"a".repeat(40),"task":"Review commits a..HEAD in apps/x"}),
        );
        assert!(task.contains("task must be a short id"), "{task}");
        assert!(message("review.start", json!({"root":root,"task":"t"})).contains("together"));
        let absolute = message(
            "review.start",
            json!({"root":root,"files":["/srv/private/source/design.md"]}),
        );
        assert!(absolute.contains("relative to root"), "{absolute}");
        assert!(message("calls.start", json!({"title":"x"})).contains("needs instruction"));
        assert!(
            message("calls.start", json!({"instruction":"x","base":"a"})).contains("review.start")
        );
        let unknown = message("calls.start", json!({"instruction":"secret","nope":1}));
        assert!(unknown.contains("help"), "{unknown}");
        let refusal = call_action(
            "calls.start",
            json!({"instruction":"x","capability":"video"}),
            "op",
        )
        .unwrap_err();
        assert_eq!(refusal.code, "unsupported_capability");
        let context = call_action(
            "calls.start",
            json!({"instruction":"x","root":"/","files":[]}),
            "op",
        )
        .unwrap_err();
        assert_eq!(context.code, "invalid_argument");
        assert!(context.message.contains("absolute directory"));
        for value in [short, task, absolute, unknown, context.message] {
            assert!(!value.contains(root) && !value.contains("secret"));
        }
    }

    #[test]
    fn document_operation_identity_is_parent_scoped_and_stable() {
        assert_eq!(
            document_operation("a", "one"),
            document_operation("a", "one")
        );
        assert_ne!(
            document_operation("a", "one"),
            document_operation("b", "one")
        );
        assert_ne!(
            document_operation("a", "one"),
            document_operation("a", "two")
        );
    }

    #[test]
    fn review_reuse_needs_an_explicit_task_root_and_immutable_base() {
        for args in [
            json!({"task":"task"}),
            json!({"base":"a".repeat(40)}),
            json!({"task":"task","base":"main","root":"/tasks/project"}),
            json!({"task":"task","base":"a".repeat(40)}),
            json!({"fresh":true}),
        ] {
            assert!(call_action("review.start", args, "op").is_err());
        }
        let args = json!({"task":"task","base":"a".repeat(40),"root":"/tasks/project"});
        assert!(call_action("calls.start", args.clone(), "op").is_err());
        let Action::Start { request, .. } = call_action("review.start", args, "op").unwrap() else {
            panic!("expected start");
        };
        assert_eq!(request.labels[crate::managed_calls::review::REUSE], "true");
        assert!(!request.output.is_text());
        assert!(request.prompt().contains(&"a".repeat(40)));
    }
}

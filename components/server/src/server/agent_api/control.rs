//! Bounded adapters to the product's existing handlers and command dispatcher.
//! No URL, HTTP headers, cookie, host delegation or arbitrary command is accepted.
use super::*;
use crate::admin::AdminRole;
use crate::agent_api::control as catalog;
use crate::server as web;
use crate::store::agent_api_operations::Admission;
use axum::{
    Router,
    body::Body,
    http::{Method, Request},
    routing::{MethodRouter, delete, get, patch, post, put},
};
use serde::Deserialize;
use tower::ServiceExt as _;

static EFFECTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(16)));

async fn identity(
    state: &AppState,
    parent: &SessionMeta,
    write: bool,
) -> Result<web::AuthenticatedProductRequest, &'static str> {
    if !parent_current(&state.hub, parent) {
        return Err("permission_denied");
    }
    let owner = parent.owner_user_id.as_deref().ok_or("permission_denied")?;
    let user = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .user_by_id(owner)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("permission_denied")?;
    if user.disabled_at_ms.is_some() {
        return Err("permission_denied");
    }
    if !parent_current(&state.hub, parent) {
        return Err("permission_denied");
    }
    let auth = web::AuthenticatedProductRequest {
        principal: web::product_principal(&state.hub, &user),
        cookie_session: None,
        device_identity: None,
        permissions: None,
    }
    .observe_permissions(&state.hub)
    .map_err(|()| "permission_denied")?;
    if write && !auth.principal.can_reorder() {
        return Err("permission_denied");
    }
    Ok(auth)
}

fn target(hub: &crate::core::Hub, parent: &SessionMeta, input: &Value) -> Result<(), &'static str> {
    if let Some(id) = input.get("session_id") {
        let id = id
            .as_str()
            .filter(|id| crate::managed_calls::valid_id(id))
            .ok_or("invalid_argument")?;
        if hub.session_info(id).is_none_or(|info| {
            info.meta.system || info.meta.closing || info.meta.owner_user_id != parent.owner_user_id
        }) {
            return Err("not_found");
        }
    }
    Ok(())
}

fn parent_current(hub: &crate::core::Hub, parent: &SessionMeta) -> bool {
    hub.session_info(&parent.id).is_some_and(|info| {
        let current = &info.meta;
        !current.system
            && !current.closing
            && current.owner_user_id == parent.owner_user_id
            && current.provider == parent.provider
            && current.provider_version == parent.provider_version
            && current.provider_generation_digest == parent.provider_generation_digest
            && current.provider_auth_generation == parent.provider_auth_generation
            && current.cwd == parent.cwd
            && current.machine_id == parent.machine_id
            && current.execution_binding == parent.execution_binding
            && current.agent_session_id == parent.agent_session_id
    })
}

async fn current_identity(
    state: &AppState,
    parent: &SessionMeta,
    auth: &web::AuthenticatedProductRequest,
) -> Result<(), &'static str> {
    actions::read_parent(&state.hub, state.store.as_ref(), parent).await?;
    if !parent_current(&state.hub, parent)
        || !auth
            .permissions
            .as_ref()
            .is_some_and(|observation| observation.current(&state.hub, &auth.principal.username))
    {
        return Err("permission_denied");
    }
    Ok(())
}

fn route_permission(
    name: &str,
    principal: &crate::product_auth::ProductPrincipal,
    method: &Method,
    path: &str,
) -> Result<(), &'static str> {
    use web::RouteAuth;
    let allowed = match web::classify_route(method, path) {
        // Shared Tools defaults have an owner-only writer; reject before
        // dispatch like the Web handler does.
        _ if name == "tools.defaults_set" => principal.role == AdminRole::Owner,
        RouteAuth::Public | RouteAuth::Product | RouteAuth::ProductSessionSee => true,
        RouteAuth::ProductOperator
        | RouteAuth::ProductOrAdminOperator
        | RouteAuth::ProductSessionMutate => principal.can_reorder(),
        _ => false,
    };
    // Shared Tools defaults have an owner-only writer, whichever plane the
    // Web route is classified on.
    let allowed = allowed && (name != "tools.defaults_set" || principal.role == AdminRole::Owner);
    if allowed {
        Ok(())
    } else {
        Err("permission_denied")
    }
}

fn permitted(name: &str, principal: &crate::product_auth::ProductPrincipal) -> bool {
    let Some(spec) = catalog::find(name) else {
        return false;
    };
    if spec.write && !principal.can_reorder() {
        return false;
    }
    if name == "configuration.service_set" {
        return principal.role == AdminRole::Owner;
    }
    route(name).is_none_or(|(method, pattern, _)| {
        let path = pattern
            .split('/')
            .map(|part| {
                if part.starts_with('{') {
                    "target"
                } else {
                    part
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        route_permission(
            name,
            principal,
            &method.parse().expect("static method"),
            &path,
        )
        .is_ok()
    })
}

fn scoped_targets(
    hub: &crate::core::Hub,
    parent: &SessionMeta,
    name: &str,
    input: &Value,
) -> Result<(), &'static str> {
    target(hub, parent, input)?;
    if let Some(id) = input.get("to_session") {
        target(hub, parent, &json!({"session_id":id}))?;
    }
    if let Some(ids) = input.get("session_ids") {
        for id in ids.as_array().ok_or("invalid_argument")? {
            target(hub, parent, &json!({"session_id":id}))?;
        }
    }
    if name != "sync.apply" {
        return Ok(());
    }
    let state = input["state"].as_str().ok_or("invalid_argument")?;
    let args = &input["args"];
    if let Some(id) = web::session_id_from_sync_state(state) {
        return target(hub, parent, &json!({"session_id":id}));
    }
    match state {
        "title" => target(hub, parent, &json!({"session_id":args["session_id"]})),
        "order" | "folders" if state == "order" || input["name"] == "place" => {
            let field = if state == "order" {
                "order"
            } else {
                "session_ids"
            };
            for id in args[field].as_array().ok_or("invalid_argument")? {
                target(hub, parent, &json!({"session_id":id}))?;
            }
            Ok(())
        }
        "folders" | "workspace-order" => Ok(()), // scoped FolderActor in the existing writer
        _ => Err("invalid_argument"),
    }
}

fn route(name: &str) -> Option<(&'static str, &'static str, MethodRouter<Arc<AppState>>)> {
    Some(match name {
        "machines.refresh" => (
            "POST",
            "/api/machines/{machine}/refresh",
            post(web::api_machine_refresh),
        ),
        "sessions.placements" => (
            "GET",
            "/api/execution-environments",
            get(web::execution::sessions::availability),
        ),
        "plugins.machine_receipt" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/operations/{operation}/machine-receipt",
            get(web::api_machine_plugin_operation_receipt),
        ),
        "plugins.recovery_assessment" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/operations/{operation}/recovery-assessment",
            get(web::api_machine_plugin_recovery_assessment),
        ),
        "sessions.calls" => (
            "GET",
            "/api/sessions/{session_id}/calls",
            get(web::managed_calls::list),
        ),
        "queue.delivery_receipt" => (
            "GET",
            "/api/sessions/{session_id}/delivery/{operation}",
            get(web::send_delivery::receipt),
        ),
        "sessions.call" => (
            "GET",
            "/api/sessions/{session_id}/calls/{call}",
            get(web::managed_calls::inspect),
        ),
        "sessions.call_cancel" => (
            "POST",
            "/api/sessions/{session_id}/calls/{call}/cancel",
            post(web::managed_calls::cancel),
        ),
        "sessions.calls_dismiss" => (
            "POST",
            "/api/sessions/{session_id}/calls/dismiss",
            post(web::managed_calls::dismiss),
        ),
        "plugins.history" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/lifecycle-history",
            get(web::plugin_history::get_history),
        ),
        "projects.workspaces" => ("GET", "/api/workspaces", get(web::api_workspaces)),
        "sessions.questions" => (
            "GET",
            "/api/sessions/{session_id}/question-pages",
            get(web::api_question_pages),
        ),
        "sessions.question" => (
            "GET",
            "/api/sessions/{session_id}/question-pages/{page}",
            get(web::api_question_page),
        ),
        "diagnostics.metrics" => ("GET", "/api/metrics", get(web::api_metrics)),
        "documents.undo_copy" => (
            "DELETE",
            "/api/sessions/{session_id}/draft-copies/{cmid}",
            delete(web::draft_documents::undo_copy),
        ),
        "queue.options" => (
            "GET",
            "/api/sessions/{session_id}/delivery-options",
            get(web::send_delivery::options),
        ),
        "sessions.context" => (
            "GET",
            "/api/sessions/{session_id}/context",
            get(web::session_context::get),
        ),
        "sessions.context_set" => (
            "PUT",
            "/api/sessions/{session_id}/context",
            put(web::session_context::set),
        ),
        "sessions.context_send" => (
            "POST",
            "/api/sessions/{session_id}/context/send",
            post(web::session_context::send),
        ),
        "sessions.fork" => (
            "POST",
            "/api/sessions/{session_id}/fork",
            post(web::session_context::fork),
        ),
        "queue.policy_set" => (
            "PUT",
            "/api/sessions/{session_id}/delivery-policy",
            put(web::send_delivery::set_policy),
        ),
        "queue.release_hold" => (
            "POST",
            "/api/sessions/{session_id}/delivery-hold/release",
            post(web::send_delivery::release),
        ),
        "plugins.catalog" => ("GET", "/api/plugins", get(web::api_plugins)),
        "plugins.call" => (
            "POST",
            "/api/plugins/{plugin}/call",
            post(web::api_plugin_call),
        ),
        "plugins.installations" => (
            "GET",
            "/api/machines/{machine}/plugins",
            get(web::api_machine_plugins),
        ),
        "plugins.operations" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/operations",
            get(web::api_machine_plugin_operations),
        ),
        "plugins.install_operations" => (
            "GET",
            "/api/machines/{machine}/plugins/{plugin}/installation-operations",
            get(web::plugin_install::api_machine_plugin_install_operations),
        ),
        "machines.health" => (
            "GET",
            "/api/machines/{machine}/deployment-health",
            get(web::api_machine_deployment_health),
        ),
        "machines.durable_state" => (
            "GET",
            "/api/machines/{machine}/durable-state",
            get(web::api_machine_durable_state),
        ),
        "machines.maintenance" => (
            "GET",
            "/api/session-maintenance",
            get(web::session_maintenance::inventory),
        ),
        "machines.maintenance_set" => (
            "PUT",
            "/api/machines/{machine}/maintenance",
            put(web::session_maintenance::configure),
        ),
        "projects.list" => (
            "GET",
            "/api/machines/{machine}/projects",
            get(web::projects::list),
        ),
        "projects.policies" => ("GET", "/api/project-policies", get(web::projects::policies)),
        "projects.placements" => (
            "GET",
            "/api/project-placements",
            get(web::projects::placements),
        ),
        "projects.request" => (
            "POST",
            "/api/machines/{machine}/projects",
            post(web::projects::request),
        ),
        "projects.policy_set" => (
            "PUT",
            "/api/machines/{machine}/project-policy",
            put(web::projects::update_policy),
        ),
        "folders.list" => ("GET", "/api/folders", get(web::folder_defaults::list)),
        "folders.defaults" => (
            "GET",
            "/api/folders/{folder}/defaults",
            get(web::folder_defaults::get_defaults),
        ),
        "folders.create" => ("POST", "/api/folders", post(web::folder_defaults::create)),
        "folders.defaults_set" => (
            "PATCH",
            "/api/folders/{folder}/defaults",
            patch(web::folder_defaults::update),
        ),
        "tools.defaults" => ("GET", "/api/agent-tools", get(web::agent_tools::list)),
        "tools.session" => (
            "GET",
            "/api/sessions/{session_id}/tools",
            get(web::agent_tools::session),
        ),
        "tools.defaults_set" => (
            "PUT",
            "/api/agent-tools/{provider}",
            put(web::agent_tools::configure_agent),
        ),
        "tools.session_set" => (
            "PUT",
            "/api/sessions/{session_id}/tools",
            put(web::agent_tools::configure_session),
        ),
        "sessions.create" => ("POST", "/api/sessions", post(web::api_new_session)),
        "sessions.reload_plan" => (
            "GET",
            "/api/sessions/{session_id}/reload",
            get(web::api_session_reload_plan),
        ),
        "sessions.reload" => (
            "POST",
            "/api/sessions/{session_id}/reload",
            post(web::api_session_reload),
        ),
        "sessions.update_policy" => (
            "PUT",
            "/api/sessions/{session_id}/reload",
            put(web::session_provider_updates::configure),
        ),
        "sessions.hibernate" => (
            "POST",
            "/api/sessions/{session_id}/hibernate",
            post(web::api_session_hibernate),
        ),
        "sessions.cache_protection" => (
            "GET",
            "/api/sessions/{session_id}/cache-protection",
            get(web::api_session_cache_protection),
        ),
        "usage.executor_set" => (
            "PUT",
            "/api/usage/{account}/executor",
            put(web::api_usage_executor_update),
        ),
        "usage.reset" => (
            "POST",
            "/api/usage/{account}/reset",
            post(web::api_provider_reset),
        ),
        "usage.reset_schedule" => (
            "PUT",
            "/api/usage/{account}/reset/schedule",
            put(web::api_provider_reset_schedule),
        ),
        "usage.reset_cancel" => (
            "DELETE",
            "/api/usage/{account}/reset/schedule",
            delete(web::api_provider_reset_cancel),
        ),
        "usage.activity" => (
            "GET",
            "/api/usage/{account}/activity",
            get(web::api_usage_activity),
        ),
        "usage.logs" => ("GET", "/api/usage/logs", get(web::api_usage_logs)),
        "diagnostics.logs" => ("GET", "/api/logs", get(web::api_diagnostic_logs)),
        "diagnostics.log" => ("GET", "/api/logs/{id}", get(web::api_diagnostic_log_detail)),
        "diagnostics.incidents" => (
            "GET",
            "/api/observability/incidents",
            get(web::api_observability_incidents),
        ),
        "documents.history" => (
            "GET",
            "/api/drafts/{id}/history/index",
            get(web::draft_documents::history_index),
        ),
        "documents.history_snapshot" => (
            "GET",
            "/api/drafts/{id}/history/{revision}",
            get(web::draft_documents::history_snapshot),
        ),
        _ => return None,
    })
}

fn command(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "sessions.wake" => ("open_session", "session_id"),
        "sessions.rename" => ("rename_session", "session_id title"),
        "sessions.pause" => ("set_paused", "session_id paused"),
        "sessions.configure" => ("set_config_option", "session_id config_id value"),
        "sessions.cancel" => ("cancel", "session_id"),
        "sessions.retry" => ("retry_turn", "session_id"),
        "sessions.reset" => ("reset_session", "session_id"),
        "sessions.delete" => ("delete_session", "session_id"),
        "sessions.trash" => ("trash_items", "session_ids folder_ids"),
        "sessions.restore" => ("restore_items", "session_ids folder_ids"),
        "sessions.purge" => ("purge_items", "session_ids folder_ids"),
        "queue.submit" => ("submit", "session_id text content front force delivery"),
        "queue.edit" => ("edit_queued", "session_id id text content"),
        "queue.remove" => ("remove_queued", "session_id id"),
        "queue.clear" => ("clear_queue", "session_id"),
        "queue.send" => ("request_send_queued", "session_id id"),
        "queue.force_send" => ("force_push_queued", "session_id id"),
        "queue.to_draft" => ("queued_to_draft", "session_id id"),
        "queue.reorder" => ("reorder_queue", "session_id order"),
        "drafts.add" => ("add_draft", "session_id text content"),
        "drafts.edit" => ("edit_draft", "session_id id text content"),
        "drafts.remove" => ("remove_draft", "session_id id"),
        "drafts.move" => ("move_draft", "session_id id to_session"),
        "drafts.clear" => ("clear_drafts", "session_id"),
        "drafts.activate" => ("activate_draft", "session_id id"),
        "drafts.activate_all" => ("activate_all_drafts", "session_id"),
        "drafts.reorder" => ("reorder_drafts", "session_id order"),
        "drafts.schedule" => (
            "schedule_draft",
            "session_id id text content fire_at_ms delivery",
        ),
        "drafts.unschedule" => ("unschedule_draft", "session_id id"),
        "sync.apply" => ("sync", "state name args"),
        _ => return None,
    })
}

fn command_value(name: &str, input: &Value, operation: &str) -> Result<Value, &'static str> {
    let (kind, fields) = command(name).ok_or("unknown_operation")?;
    let mut fields_value = input.as_object().ok_or("invalid_argument")?.clone();
    if fields_value
        .keys()
        .any(|key| !fields.split_whitespace().any(|field| field == key))
    {
        return Err("invalid_argument");
    }
    if matches!(name, "queue.edit" | "drafts.edit") && !fields_value.contains_key("content") {
        return Err("invalid_argument");
    }
    if name == "queue.submit" {
        let delivery = fields_value
            .remove("delivery")
            .map(serde_json::from_value::<crate::send_delivery::SendDelivery>)
            .transpose()
            .map_err(|_| "invalid_argument")?;
        let delivery = crate::send_delivery::SendDelivery::resolve(
            delivery,
            input["force"].as_bool().unwrap_or(false),
            input["front"].as_bool().unwrap_or(false),
        )?;
        if matches!(
            delivery,
            crate::send_delivery::SendDelivery::CurrentTurn
                | crate::send_delivery::SendDelivery::NextCheckpoint
        ) {
            return Err("invalid_argument");
        }
        fields_value.insert(
            "force".into(),
            json!(delivery == crate::send_delivery::SendDelivery::Interrupt),
        );
    }
    fields_value.insert("type".into(), json!(kind));
    if name == "sync.apply" {
        fields_value.insert("id".into(), json!(operation));
    } else {
        fields_value.insert("cmid".into(), json!(operation));
    }
    let value = Value::Object(fields_value);
    serde_json::from_value::<crate::core::Inbound>(value.clone())
        .map_err(|_| "invalid_argument")?;
    Ok(value)
}

fn uri(pattern: &str, input: &Value) -> Result<String, &'static str> {
    let object = input.as_object().ok_or("invalid_argument")?;
    let mut path = pattern.to_owned();
    for (key, value) in object {
        if key == "query" || key == "body" {
            continue;
        }
        let placeholder = format!("{{{key}}}");
        if !pattern.contains(&placeholder) {
            return Err("invalid_argument");
        }
        let id = value
            .as_str()
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 512
                    && !matches!(*v, "." | "..")
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_:.@".contains(&b))
            })
            .ok_or("invalid_argument")?;
        path = path.replace(&placeholder, id);
    }
    if path.contains(['{', '}']) {
        return Err("invalid_argument");
    }
    if let Some(query) = object.get("query") {
        let query = query.as_object().ok_or("invalid_argument")?;
        let mut encoded = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in query {
            let value = match value {
                Value::String(s) => s.clone(),
                Value::Number(_) | Value::Bool(_) => value.to_string(),
                _ => return Err("invalid_argument"),
            };
            encoded.append_pair(key, &value);
        }
        path.push('?');
        path.push_str(&encoded.finish());
    }
    Ok(path)
}

async fn dispatch(
    state: &Arc<AppState>,
    parent: &SessionMeta,
    name: &str,
    mut input: Value,
    operation: Option<&str>,
    auth: &web::AuthenticatedProductRequest,
) -> Result<Value, String> {
    let spec = catalog::find(name).ok_or("unknown_operation")?;
    current_identity(state, parent, auth).await?;
    if spec.write && !auth.principal.can_reorder() {
        return Err("permission_denied".into());
    }
    scoped_targets(&state.hub, parent, name, &input)?;
    if input.get("to_session").is_some() {
        target(
            &state.hub,
            parent,
            &json!({"session_id":input["to_session"]}),
        )?;
    }
    if name == "operations.receipt" {
        only_keys(&input, &["operation"])?;
        return Ok(receipt_full(state, parent, arg_str(&input, "operation")?).await?);
    }
    if name == "sync.read" {
        only_keys(&input, &["state"])?;
        let scope = input["state"]
            .as_str()
            .filter(|s| s.len() <= 256)
            .ok_or("invalid_argument")?;
        if let Some(id) = web::session_id_from_sync_state(scope) {
            target(&state.hub, parent, &json!({"session_id":id}))?;
        } else if !matches!(scope, "title" | "order" | "workspace-order" | "folders") {
            return Err("invalid_argument".into());
        }
        let mut principal = auth.principal.clone();
        // Even an owner uses the Agent API's own-session visibility boundary.
        principal.role = crate::admin::AdminRole::Operator;
        let visible = state
            .hub
            .session_list()
            .iter()
            .filter(|s| !s.system && !s.closing && s.owner_user_id == parent.owner_user_id)
            .map(|s| s.id.clone())
            .collect();
        let frame = web::project_outbound(
            &state.hub,
            &principal,
            &visible,
            crate::core::Outbound::SyncPatch {
                state: scope.to_owned(),
                version: 0,
                value: state.hub.sync_value(scope),
                confirmed: vec![],
                resync: true,
            },
        )
        .ok_or("not_found")?;
        return Ok(serde_json::to_value(frame).map_err(|_| "unavailable")?["value"].clone());
    }
    if name.starts_with("configuration.") {
        if name == "configuration.service" {
            only_keys(&input, &[])?;
            return crate::config::editable(
                crate::config::Scope::Service,
                &state.service_config_path,
            )
            .map_err(|_| "configuration_unavailable".into());
        }
        if auth.principal.role != crate::admin::AdminRole::Owner {
            return Err("permission_denied".into());
        }
        only_keys(&input, &["expected_revision", "changes"])?;
        let expected = arg_str(&input, "expected_revision")?.to_owned();
        let path = state.service_config_path.clone();
        let changes = input["changes"].clone();
        let (effect_state, effect_parent, effect_auth) =
            (Arc::clone(state), parent.clone(), auth.clone());
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            crate::config::patch_checked(
                crate::config::Scope::Service,
                &path,
                &expected,
                &changes,
                || {
                    runtime
                        .block_on(current_identity(
                            &effect_state,
                            &effect_parent,
                            &effect_auth,
                        ))
                        .map_err(anyhow::Error::msg)
                },
            )
        })
        .await
        .map_err(|_| "unavailable")?
        .map_err(|error| match error.to_string().as_str() {
            "revision_conflict" => "revision_conflict",
            "permission_denied" => "permission_denied",
            _ => "invalid_configuration",
        })?;
        // The existing watcher is the sole live activation owner. Reloading the
        // returned report here could race a later writer and install stale values.
        return Ok(
            json!({"status":"committed","activation":"Service watcher reloads live fields; restart fields await restart. Read configuration.read('service') to observe activation."}),
        );
    }
    if matches!(
        name,
        "plugins.install" | "plugins.uninstall_plan" | "plugins.uninstall"
    ) {
        only_keys(&input, &["machine", "plugin", "version", "digest", "body"])?;
        let machine = arg_str(&input, "machine")?.to_owned();
        let plugin = arg_str(&input, "plugin")?.to_owned();
        if web::service_managed_refusal(state, &machine).is_some() {
            return Err("service_managed_membership".into());
        }
        let approval = web::operator_approval::OperatorApproval::capture_agent(
            &state.service_id,
            parent,
            auth,
        )
        .map_err(|_| "permission_denied")?;
        let response = match name {
            "plugins.install" => {
                only_keys(&input, &["machine", "plugin", "version", "digest"])?;
                let request = web::PluginInstallRequest {
                    operation_id: operation.ok_or("invalid_operation")?.to_owned(),
                    version: arg_str(&input, "version")?.to_owned(),
                    digest: input["digest"]
                        .as_str()
                        .filter(|digest| {
                            digest.strip_prefix("sha256:").is_some_and(|hash| {
                                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
                            })
                        })
                        .ok_or("invalid_argument")?
                        .to_owned(),
                };
                web::plugin_install::confirmed_install(
                    Arc::clone(state),
                    machine,
                    plugin,
                    request,
                    approval,
                )
                .await
            }
            "plugins.uninstall_plan" => {
                only_keys(&input, &["machine", "plugin"])?;
                web::preview_machine_plugin_uninstall(
                    Arc::clone(state),
                    machine,
                    plugin,
                    approval.actor().clone(),
                )
                .await
            }
            _ => {
                only_keys(&input, &["machine", "plugin", "body"])?;
                let request = serde_json::from_value(input["body"].clone())
                    .map_err(|_| "invalid_argument")?;
                web::plugin_uninstall::confirmed_uninstall(
                    Arc::clone(state),
                    machine,
                    plugin,
                    request,
                    approval,
                )
                .await
            }
        };
        return response_value(response, true).await;
    }
    if name == "queue.submit" && input["delivery"] == "session_default" {
        let session = arg_str(&input, "session_id")?;
        if state.hub.session_info(session).is_none() {
            return Err("session_not_found".into());
        }
        input["delivery"] = json!(match web::send_delivery::policy(&state.hub, session).mode {
            crate::send_delivery::DeliveryMode::Steer => "current_turn",
            crate::send_delivery::DeliveryMode::ToolBoundary => "next_checkpoint",
            crate::send_delivery::DeliveryMode::NextTurn => "next_turn",
        });
    }
    if matches!(name, "sessions.trash" | "sessions.purge")
        && input["session_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == parent.id.as_str()))
    {
        return Err("would_interrupt_caller".into());
    }
    if input["session_id"] == parent.id
        && ((name == "queue.submit"
            && (input["force"] == true
                || matches!(
                    input["delivery"].as_str(),
                    Some("interrupt" | "current_turn" | "next_checkpoint")
                )))
            || matches!(
                name,
                "sessions.delete"
                    | "sessions.reset"
                    | "sessions.cancel"
                    | "sessions.reload"
                    | "sessions.hibernate"
                    | "sessions.retry"
                    | "queue.force_send"
            ))
    {
        return Err("would_interrupt_caller".into());
    }
    if name == "queue.submit"
        && matches!(
            input["delivery"].as_str(),
            Some("current_turn" | "next_checkpoint" | "interrupt")
        )
    {
        only_keys(
            &input,
            &[
                "session_id",
                "text",
                "content",
                "front",
                "force",
                "delivery",
                "target",
            ],
        )?;
        let session = arg_str(&input, "session_id")?;
        let mut body = input.clone();
        body.as_object_mut()
            .ok_or("invalid_argument")?
            .remove("session_id");
        let mut request: web::SessionPromptRequest =
            serde_json::from_value(body).map_err(|_| "invalid_argument")?;
        crate::send_delivery::SendDelivery::resolve(
            request.delivery,
            request.force,
            request.front,
        )?;
        if request.delivery == Some(crate::send_delivery::SendDelivery::Interrupt) {
            if request.target.is_none() {
                return Err("interrupt_requires_target_from_queue_options".into());
            }
            request.operation = Some(operation.ok_or("invalid_operation")?.to_owned());
            return response_value(
                web::send_delivery::submit(Arc::clone(state), session.to_owned(), request).await,
                true,
            )
            .await;
        }
        let effect = web::send_delivery::begin(
            state,
            session,
            &request.text,
            &request.content,
            operation.ok_or("invalid_operation")?.to_owned(),
            request.target.as_ref(),
            request.delivery == Some(crate::send_delivery::SendDelivery::NextCheckpoint),
        )?;
        let mut receipt = web::send_delivery::result(&effect.outcome().await);
        receipt["delivery"] = json!(request.delivery);
        return Ok(receipt);
    }
    if name == "usage.refresh" {
        only_keys(&input, &["account"])?;
        let account = input
            .get("account")
            .map(|_| arg_str(&input, "account"))
            .transpose()?;
        if let Some(account) = account
            && let Ok(id) = account.parse::<crate::provider_account::ProviderAccountId>()
            && !id.is_default()
        {
            let owner = super::actions::account_owner(state, parent).await?;
            super::super::provider_accounts::usage::refresh_one(state, &owner, &id)
                .await
                .map_err(str::to_owned)?;
            return super::super::provider_accounts::usage::read_one(state, &owner, &id)
                .await
                .map_err(str::to_owned);
        }
        // A global refresh covers the caller's named accounts too, through
        // the same coalesced serial refresh as the Web's "refresh all".
        if account.is_none()
            && let Ok(owner) = super::actions::account_owner(state, parent).await
        {
            super::super::provider_accounts::usage::refresh_owner_accounts(state, &owner);
        }
        let snapshot = state
            .usage
            .request_refresh(account)
            .await
            .map_err(|_| "invalid_argument")?;
        return Ok(json!({"status":"accepted","refreshing":snapshot.refreshing}));
    }
    if command(name).is_some() {
        let operation = operation.ok_or("read_only")?;
        let value = command_value(name, &input, operation)?;
        let mut principal = auth.principal.clone();
        principal.role = AdminRole::Operator;
        let accepted = web::handle_command(
            state,
            &principal,
            &value.to_string(),
            &mut HashMap::new(),
            None,
        );
        return Ok(
            json!({"status":if accepted {"accepted"} else {"rejected"},"completion":"observe domain state"}),
        );
    }
    if name == "sessions.create" {
        let body = input
            .get_mut("body")
            .and_then(Value::as_object_mut)
            .ok_or("invalid_argument")?;
        if body.contains_key("system")
            || body.contains_key("owner_user_id")
            || body.contains_key("origin")
        {
            return Err("invalid_argument".into());
        }
        // Agent creation follows the same managed-worktree path as the GUI.
        body.insert("origin".into(), json!("web"));
    }
    if name == "projects.placements" {
        only_keys(&input, &["machine"])?;
        input = json!({"query":{"machine_id":arg_str(&input,"machine")?}});
    }
    let (method, pattern, handler) = route(name).ok_or("unknown_operation")?;
    if !spec.write && input.get("body").is_some() {
        return Err("invalid_argument".into());
    }
    let path = uri(pattern, &input)?;
    route_permission(
        name,
        &auth.principal,
        &method.parse().map_err(|_| "unavailable")?,
        path.split('?').next().unwrap_or(&path),
    )?;
    let body = input.get("body").cloned().unwrap_or(json!({}));
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .extension({
            let mut scoped = auth.clone();
            if name.starts_with("folders.") {
                scoped.principal.role = AdminRole::Operator;
            }
            scoped
        })
        .body(Body::from(body.to_string()))
        .map_err(|_| "invalid_argument")?;
    let response = Router::new()
        .route(pattern, handler)
        .with_state(Arc::clone(state))
        .oneshot(request)
        .await
        .map_err(|_| "unavailable")?;
    response_value(response, spec.write).await
}

async fn response_value(response: axum::response::Response, write: bool) -> Result<Value, String> {
    let status = response.status();
    // Internal handler output is never returned wholesale to the model. Allow
    // large documents/catalogs to be selected, while bounding local allocation.
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|_| "selection_too_large")?;
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)))
    };
    if write {
        Ok(
            json!({"status":if status.is_success(){"accepted"}else{"rejected"},"http_status":status.as_u16(),"result":value}),
        )
    } else if status.is_success() {
        Ok(value)
    } else {
        // The handler's rejection (for example an unknown query field) is the
        // only actionable part; the broker splits it into a bounded message.
        let detail = match &value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.get("error").map_or_else(
                || other.to_string(),
                |error| {
                    error
                        .as_str()
                        .map_or_else(|| error.to_string(), str::to_owned)
                },
            ),
        };
        let detail: String = detail.chars().take(300).collect();
        Err(if detail.trim().is_empty() {
            format!("http_{}", status.as_u16())
        } else {
            format!("http_{}: {}", status.as_u16(), detail.trim())
        })
    }
}

pub(super) async fn read(
    state: &Arc<AppState>,
    parent: &SessionMeta,
    op: &str,
    args: Value,
) -> Result<Value, String> {
    match op {
        "control.list" => {
            only_keys(&args, &["domain"])?;
            let mut value = catalog::list(
                args.get("domain")
                    .map(|_| arg_str(&args, "domain"))
                    .transpose()?,
            );
            let auth = identity(state, parent, false).await.ok();
            let writes = control_supported(state, parent)
                && auth
                    .as_ref()
                    .is_some_and(|auth| auth.principal.can_reorder());
            value["writes_available"] = json!(writes);
            if let Some(operations) = value.get_mut("operations").and_then(Value::as_array_mut) {
                for op in operations {
                    op["available"] = json!(
                        auth.as_ref().is_some_and(|auth| permitted(
                            op["name"].as_str().unwrap(),
                            &auth.principal
                        )) && (op["write"] != true || writes)
                    );
                }
            }
            Ok(value)
        }
        "control.describe" => {
            only_keys(&args, &["name"])?;
            let name = arg_str(&args, "name")?;
            let mut value = catalog::describe(name).ok_or("unknown_operation")?;
            let auth = identity(state, parent, false).await.ok();
            value["available"] = json!(
                auth.as_ref()
                    .is_some_and(|auth| permitted(name, &auth.principal))
                    && (value["write"] != true || control_supported(state, parent))
            );
            Ok(value)
        }
        "control.read" => {
            only_keys(&args, &["name", "input", "selection"])?;
            let name = arg_str(&args, "name")?;
            if catalog::find(name).ok_or("unknown_operation")?.write {
                return Err("read_only".into());
            }
            let auth = identity(state, parent, false).await?;
            let value = dispatch(
                state,
                parent,
                name,
                args.get("input").cloned().unwrap_or(json!({})),
                None,
                &auth,
            )
            .await?;
            current_identity(state, parent, &auth).await?;
            scoped_targets(&state.hub, parent, name, &args["input"])?;
            Ok(
                json!({"revision":crate::managed_calls::canonical_digest(&value),"page":json_reads::select(&value,args.get("selection").unwrap_or(&json!({"describe":true})))?}),
            )
        }
        _ => Err("unknown_operation".into()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Apply {
    name: String,
    input: Value,
}

pub(super) async fn apply(
    state: &Arc<AppState>,
    parent: &SessionMeta,
    args: Value,
    operation: &str,
) -> Result<Value, &'static str> {
    if !control_supported(state, parent) {
        return Err("unsupported_capability");
    }
    let input: Apply = serde_json::from_value(args.clone()).map_err(|_| "invalid_argument")?;
    if !catalog::find(&input.name).ok_or("unknown_operation")?.write {
        return Err("read_only");
    }
    let auth = identity(state, parent, true).await?;
    scoped_targets(&state.hub, parent, &input.name, &input.input)?;
    let store = state.store.as_ref().ok_or("unavailable")?.clone();
    let slot = Arc::clone(&EFFECTS)
        .try_acquire_owned()
        .map_err(|_| "capacity")?;
    let owner = auth.principal.user_id.clone();
    let domain_operation = format!(
        "agent-{}",
        crate::admin::hex_sha256(
            &serde_json::to_vec(&(&state.service_id, &owner, &parent.id, operation))
                .expect("strings serialize")
        )
    );
    let hash = crate::managed_calls::canonical_digest(&args);
    match store
        .admit_agent_operation(&owner, &parent.id, operation, &hash)
        .await
        .map_err(|_| "unavailable")?
    {
        Admission::Existing(result) => {
            current_identity(state, parent, &auth).await?;
            return Ok(receipt_view(result.unwrap_or(
                json!({"status":"pending","outcome":"unknown","domain_operation":domain_operation}),
            )));
        }
        Admission::Conflict => return Err("operation_conflict"),
        Admission::New => {}
    }
    let state = Arc::clone(state);
    let parent = parent.clone();
    let operation = operation.to_owned();
    let pending =
        json!({"status":"pending","outcome":"unknown","domain_operation":domain_operation});
    // A caller disconnect must not cancel an admitted effect or its receipt.
    let task = tokio::spawn(async move {
        let _slot = slot;
        let mut result = match dispatch(
            &state,
            &parent,
            &input.name,
            input.input,
            Some(&domain_operation),
            &auth,
        )
        .await
        {
            Ok(value) => value,
            Err(code) => json!({"status":"unknown","error":code}),
        };
        result["domain_operation"] = json!(domain_operation);
        store
            .finish_agent_operation(&owner, &parent.id, &operation, &result)
            .await
            .map_err(|_| "outcome_unknown")?;
        current_identity(&state, &parent, &auth).await?;
        Ok(receipt_view(result))
    });
    submission_reply(task, pending, Duration::from_millis(500)).await
}

async fn submission_reply(
    task: tokio::task::JoinHandle<Result<Value, &'static str>>,
    pending: Value,
    wait: Duration,
) -> Result<Value, &'static str> {
    // Return before the gateway's transport deadline. Dropping a JoinHandle
    // detaches it; the admitted writer and durable result keep their owner.
    match tokio::time::timeout(wait, task).await {
        Ok(result) => result.map_err(|_| "outcome_unknown")?,
        Err(_) => Ok(pending),
    }
}

pub(super) async fn receipt(
    state: &AppState,
    parent: &SessionMeta,
    operation: &str,
) -> Result<Value, &'static str> {
    Ok(receipt_view(receipt_full(state, parent, operation).await?))
}

fn receipt_view(value: Value) -> Value {
    if serde_json::to_vec(&value).is_ok_and(|bytes| bytes.len() <= 8 * 1024) {
        return value;
    }
    json!({"status":value["status"],"http_status":value["http_status"],"domain_operation":value["domain_operation"],"partial":true,
        "detail":"cowboy.control.read('operations.receipt',{operation},selection)"})
}

async fn receipt_full(
    state: &AppState,
    parent: &SessionMeta,
    operation: &str,
) -> Result<Value, &'static str> {
    let auth = identity(state, parent, false).await?;
    let result = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .agent_operation_receipt(&auth.principal.user_id, &parent.id, operation)
        .await
        .map_err(|_| "unavailable")?;
    current_identity(state, parent, &auth).await?;
    Ok(match result {
        None => json!({"status":"unknown"}),
        Some(None) => json!({"status":"pending","outcome":"unknown"}),
        Some(Some(value)) => value,
    })
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod contract_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_discovered_operation_has_an_adapter_and_reads_cannot_submit() {
        let mut names = std::collections::HashSet::new();
        for op in catalog::OPERATIONS {
            assert!(names.insert(op.name));
            assert!(
                route(op.name).is_some()
                    || command(op.name).is_some()
                    || matches!(
                        op.name,
                        "operations.receipt"
                            | "usage.refresh"
                            | "sync.read"
                            | "plugins.install"
                            | "plugins.uninstall_plan"
                            | "plugins.uninstall"
                    )
                    || op.name.starts_with("configuration."),
                "{}",
                op.name
            );
            if let Some((method, _, _)) = route(op.name) {
                assert_eq!(op.write, method != "GET", "{}", op.name);
            }
            if command(op.name).is_some() {
                assert!(op.write);
            }
        }
    }
    #[test]
    fn ids_cannot_route_around_the_catalog_and_commands_cannot_inject_tags() {
        for id in ["../x", "x/y", "x?y=1", "x%2fy", ""] {
            assert!(uri("/sessions/{session_id}", &json!({"session_id":id})).is_err());
        }
        assert!(
            uri(
                "/sessions/{session_id}",
                &json!({"session_id":"s","headers":{}})
            )
            .is_err()
        );
        assert!(
            command_value(
                "queue.remove",
                &json!({"session_id":"s","id":"m","type":"delete_session"}),
                "op"
            )
            .is_err()
        );
        assert!(
            command_value(
                "queue.edit",
                &json!({"session_id":"s","id":"m","text":"replace"}),
                "op"
            )
            .is_err()
        );
    }
}

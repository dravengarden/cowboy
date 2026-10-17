//! Product-visible infrastructure and effective settings. Reuse their owners'
//! projections; never open files, contact a worker or refresh a collector here.
use super::*;

pub(super) async fn read(
    state: &AppState,
    parent: &SessionMeta,
    op: &str,
    args: &Value,
) -> Result<Value, String> {
    if state.product_auth_enabled && parent.owner_user_id.is_none() {
        return Err("permission_denied".into());
    }
    let value = match op {
        "machines.list" => {
            only_keys(args, &["after", "limit"])?;
            let limit = page_limit(args, 20, 100)?;
            let after = after_id(args)?;
            let mut rows = state
                .machine_snapshots
                .load()
                .await
                .map_err(|_| "unavailable")?;
            rows.retain(|row| after.is_none_or(|after| row.id.as_str() > after));
            rows.sort_by(|a, b| a.id.cmp(&b.id));
            let next = (rows.len() > limit).then(|| rows[limit - 1].id.clone());
            let items = rows
                .iter()
                .take(limit)
                .map(|m| {
                    json!({
                        "id":m.id,"name":m.display_name,"status":m.status,"connected":m.connected,
                        "schedulable":m.schedulable,"health":m.health,"capacity":m.capacity,
                        "active_sessions":m.active_sessions,"resources":m.resources,
                        "plugins":m.plugins.len(),"workspaces":m.workspaces.len(),
                    })
                })
                .collect::<Vec<_>>();
            json!({"machines":items,"next":next})
        }
        "machines.read" => {
            only_keys(
                args,
                &[
                    "id", "section", "path", "offset", "limit", "chars", "describe",
                ],
            )?;
            let id = arg_str(args, "id")?;
            let rows = state
                .machine_snapshots
                .load()
                .await
                .map_err(|_| "unavailable")?;
            let machine = rows.iter().find(|m| m.id == id).ok_or("not_found")?;
            let section = match args.get("section") {
                None => "resources",
                Some(Value::String(section)) => section,
                _ => return Err("invalid_argument".into()),
            };
            let value = match section {
                "resources" => json!(machine.resources),
                "plugins" => json!(machine.plugins),
                "workspaces" => json!(machine.workspaces),
                "capabilities" => {
                    json!({"capabilities":machine.capabilities,"plugin_lifecycle":machine.plugin_lifecycle,"platform":machine.platform,"architecture":machine.architecture})
                }
                _ => return Err("invalid_argument".into()),
            };
            json!({"machine":id,"section":section,"page":json_reads::select(&value,args)?})
        }
        "configuration.schema" => {
            only_keys(
                args,
                &["scope", "path", "offset", "limit", "chars", "describe"],
            )?;
            let scope = match args["scope"].as_str() {
                Some("service") => crate::config::Scope::Service,
                Some("device") => crate::config::Scope::Device,
                _ => return Err("invalid_argument".into()),
            };
            json!({"scope":args["scope"],"storage":"config_file","page":json_reads::select(&crate::config::json_schema(scope),args)?})
        }
        "configuration.read" => {
            only_keys(
                args,
                &[
                    "scope",
                    "session_id",
                    "path",
                    "offset",
                    "limit",
                    "chars",
                    "describe",
                ],
            )?;
            let scope = args["scope"].as_str().ok_or("invalid_argument")?;
            let value = if scope == "service" {
                if args.get("session_id").is_some() {
                    return Err("invalid_argument".into());
                }
                state.service_config.current().to_json()
            } else {
                let id = if args.get("session_id").is_some() {
                    arg_str(args, "session_id")?
                } else {
                    &parent.id
                };
                let info = state
                    .hub
                    .session_info(id)
                    .filter(|info| visible(&info.meta, parent))
                    .ok_or("not_found")?;
                let value = match scope {
                    "session" => {
                        json!({"session_id":id,"provider":info.meta.provider,"version":info.meta.provider_version,
                        "options":state.hub.config_options(id),"preferences":state.hub.config_preferences(id)})
                    }
                    "tools" => super::super::agent_tools::session_view(state, &info.meta),
                    _ => return Err("invalid_argument".into()),
                };
                state
                    .hub
                    .session_info(id)
                    .filter(|info| visible(&info.meta, parent))
                    .ok_or("not_found")?;
                value
            };
            json!({"scope":scope,"revision":crate::managed_calls::canonical_digest(&value),"page":json_reads::select(&value,args)?})
        }
        "usage.diagnostics" => {
            only_keys(args, &["path", "offset", "limit", "chars", "describe"])?;
            let settings = state
                .usage
                .execution_settings()
                .await
                .map_err(|_| "unavailable")?;
            json!({"page":json_reads::select(&json!(settings),args)?})
        }
        _ => return Err("unknown_operation".into()),
    };
    actions::read_parent(&state.hub, state.store.as_ref(), parent).await?;
    Ok(value)
}

fn visible(meta: &SessionMeta, parent: &SessionMeta) -> bool {
    !meta.system && !meta.closing && meta.owner_user_id == parent.owner_user_id
}

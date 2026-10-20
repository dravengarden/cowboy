//! Native subscription quota. The same owner/binding admission serves UI and agents.
use super::*;
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::logins::NativeCompletion;

struct Target {
    operation: String,
    completion: NativeCompletion,
    provider: String,
    source: String,
}

async fn target(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<Target, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let account = registry
        .accounts
        .iter()
        .find(|account| &account.id == id)
        .ok_or("account_unavailable")?;
    if !account.available() {
        return Err("account_disabled");
    }
    if account.provider_id != "claude-code" {
        return Err("account_usage_unsupported");
    }
    let machines = store.list_machines().await.map_err(|_| "unavailable")?;
    let mut machines: Vec<_> = machines.iter().filter(|machine| !machine.revoked).collect();
    machines.sort_by_key(|machine| {
        (
            match state.machine_control.connection_protocol(&machine.id) {
                Some((_, version))
                    if version >= crate::machine_protocol::NATIVE_USAGE_PROTOCOL_VERSION =>
                {
                    0
                }
                Some(_) => 1,
                None => 2,
            },
            machine.id.clone(),
        )
    });
    for machine in machines {
        let Some((operation, completion)) = store
            .native_login_for_session(owner, id, &machine.id)
            .await
            .map_err(|_| "unavailable")?
        else {
            continue;
        };
        let Ok(installed) = active_machine_plugin(
            std::slice::from_ref(machine),
            &machine.id,
            &account.provider_id,
        ) else {
            continue;
        };
        if store
            .with_native_login_binding(owner, &operation, &completion, || {
                resolve_provider_generation_with_auth(
                    &state.provider_catalog,
                    &state.plugin_catalog,
                    &[installed],
                    &account.provider_id,
                    SchedulingAuthentication::Native(&completion.profile),
                )
                .map_err(anyhow::Error::msg)
            })
            .await
            .is_err()
        {
            continue;
        }
        let provider = state
            .provider_catalog
            .account_usage_provider(
                &account.provider_id,
                &completion.profile.provider_version,
                &completion.profile.generation_digest,
            )
            .ok_or("account_usage_unsupported")?;
        let binding = state
            .usage
            .plugin_bindings()
            .into_iter()
            .find(|binding| binding.account == provider)
            .ok_or("account_usage_unsupported")?;
        return Ok(Target {
            operation,
            completion,
            provider,
            source: binding.product_label().into(),
        });
    }
    Err("native_login_required")
}

fn error_message(code: &str) -> &str {
    match code {
        "account_disabled" => "This account is disabled.",
        "account_usage_unsupported" => "This Provider does not support background account usage.",
        "native_login_required" => "Sign in on a Machine to query this account's usage.",
        "machine_usage_update_required" => {
            "The signed-in Machine needs an update to query this account's usage."
        }
        "machine_unavailable" => {
            "The signed-in Machine is offline. Usage will refresh when it reconnects."
        }
        _ => "Account usage could not be refreshed. Cowboy will retry automatically.",
    }
}

pub(in crate::server) async fn read_one(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let account = registry
        .accounts
        .iter()
        .find(|account| &account.id == id)
        .ok_or("account_unavailable")?;
    let mut value = json!({"account_id":id,"provider_id":account.provider_id,"refreshing":false});
    let target = match target(state, owner, id).await {
        Ok(target) => target,
        Err(code) => {
            value["reason"] = json!(code);
            value["message"] = json!(error_message(code));
            return Ok(value);
        }
    };
    value["machine_id"] = json!(target.completion.machine_id);
    value["available"] = json!(true);
    let cache = state.usage.native.entry(owner, id.as_str());
    value["refreshing"] = json!(cache.collection.try_lock().is_err());
    // Bind the cache only inside current admission: a delayed read of an old
    // login must not erase the newer login's successful observation.
    store
        .with_native_login_binding(owner, &target.operation, &target.completion, || {
            let entry = {
                let mut entry = cache.data.lock();
                entry.bind(&target.completion);
                entry.clone()
            };
            value["usage"] = json!(entry.usage);
            if value["usage"].is_object() {
                value["usage"]["refresh"] = json!({
                    "last_attempt_at_ms":entry.last_attempt_at_ms,
                    "manual_refresh_after_ms":entry.last_attempt_at_ms.saturating_add(300_000),
                    "next_auto_refresh_at_ms":entry.last_attempt_at_ms.saturating_add(300_000),
                    "stale":entry.error.is_some(),
                });
            }
            value["last_attempt_at_ms"] = json!(entry.last_attempt_at_ms);
            value["next_refresh_at_ms"] = json!(entry.last_attempt_at_ms.saturating_add(300_000));
            if let Some(code) = entry.error {
                value["reason"] = json!(code);
                value["message"] = json!(error_message(code));
            }
            Ok(())
        })
        .await
        .map_err(|_| "account_binding_changed")?;
    Ok(value)
}

pub(in crate::server) async fn list_value(
    state: &AppState,
    owner: &str,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let mut accounts = Vec::new();
    state.usage.native.retain(
        owner,
        &registry
            .accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect::<Vec<_>>(),
    );
    for account in registry.accounts {
        accounts.push(read_one(state, owner, &account.id).await?);
    }
    Ok(json!({"accounts":accounts}))
}

pub(in crate::server) async fn refresh_one(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(), &'static str> {
    // Resolve ownership before allocating a cache slot for an untrusted ID.
    target(state, owner, id).await?;
    let cache = state.usage.native.entry(owner, id.as_str());
    let collection = Arc::clone(&cache.collection).lock_owned().await;
    let target = target(state, owner, id).await?;
    let due = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .with_native_login_binding(owner, &target.operation, &target.completion, || {
            let mut entry = cache.data.lock();
            entry.bind(&target.completion);
            let due = entry.due(now_ms());
            if due {
                entry.last_attempt_at_ms = now_ms();
            }
            Ok(due)
        })
        .await
        .map_err(|_| "account_binding_changed")?;
    if !due {
        return Ok(());
    }
    // A browser closing or a short MCP deadline must not discard an admitted
    // query's result while still charging its shared refresh budget.
    let state = Arc::clone(state);
    let owner = owner.to_owned();
    tokio::spawn(async move {
        let _collection = collection;
        let result = collect(&state, &owner, &target).await;
        cache.data.lock().finish(&target.completion, result);
    })
    .await
    .map_err(|_| "native_usage_unavailable")
}

async fn collect(state: &AppState, owner: &str, target: &Target) -> Result<Value, &'static str> {
    let completion = &target.completion;
    let store = state.store.as_ref().ok_or("unavailable")?;
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
        .ok_or("machine_unavailable")?;
    if protocol < crate::machine_protocol::NATIVE_USAGE_PROTOCOL_VERSION {
        return Err("machine_usage_update_required");
    }
    let pending = store
        .with_native_login_binding(owner, &target.operation, completion, || {
            anyhow::ensure!(
                state
                    .supervisor
                    .runtime_allowed(&completion.profile.provider_id, &completion.machine_id),
                "runtime_not_allowed"
            );
            state
                .machine_control
                .begin_native_usage(&connection, &completion.profile)
                .map_err(anyhow::Error::msg)
        })
        .await
        .map_err(|_| "account_binding_changed")?;
    let raw = pending.await.map_err(|_| "native_usage_unavailable")?;
    let machines = store.list_machines().await.map_err(|_| "unavailable")?;
    let installed = active_machine_plugin(
        &machines,
        &completion.machine_id,
        &completion.profile.provider_id,
    )
    .map_err(|_| "native_usage_unavailable")?;
    store.with_native_login_binding(owner, &target.operation, completion, || {
        anyhow::ensure!(state.machine_control.is_current(&connection), "machine_connection_changed");
        anyhow::ensure!(state.supervisor.runtime_allowed(&completion.profile.provider_id, &completion.machine_id), "runtime_not_allowed");
        resolve_provider_generation_with_auth(
            &state.provider_catalog, &state.plugin_catalog, &[installed],
            &completion.profile.provider_id, SchedulingAuthentication::Native(&completion.profile),
        ).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(raw["status"] == "available" && raw["rate_limits"].is_object(), "native_usage_unavailable");
        Ok(json!({
            "provider":target.provider,"status":"available","source":target.source,
            "observed_at_ms":now_ms(),"account":raw.get("account"),"rate_limits":raw["rate_limits"],
        }))
    }).await.map_err(|_| "native_usage_unavailable")
}

pub(in crate::server) async fn list(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
) -> Response {
    response(list_value(&state, &auth.principal.user_id).await)
}

pub(in crate::server) async fn refresh(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
) -> Response {
    match refresh_one(&state, &auth.principal.user_id, &id).await {
        Ok(()) => response(read_one(&state, &auth.principal.user_id, &id).await),
        Err(code) => response(Err(code)),
    }
}

pub(in crate::server) async fn read(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
) -> Response {
    response(read_one(&state, &auth.principal.user_id, &id).await)
}

fn response(result: Result<Value, &'static str>) -> Response {
    match result {
        Ok(value) => ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response(),
        Err(code) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

/// Bounded serial collection avoids simultaneously launching one CLI per account.
pub(in crate::server) async fn run(state: Arc<AppState>) {
    loop {
        if let Some(store) = &state.store
            && let Ok(users) = store.list_users().await
        {
            for user in users.iter().filter(|user| user.disabled_at_ms.is_none()) {
                if let Ok(registry) = store.provider_accounts(&user.id).await {
                    for account in registry
                        .accounts
                        .iter()
                        .filter(|account| account.available())
                    {
                        let _ = refresh_one(&state, &user.id, &account.id).await;
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

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

/// Advisory projection only. The dispatch/commit paths recheck this scope in
/// their admission transaction; ordinary reads never acquire its write locks.
async fn observes_binding(
    store: &crate::store::Store,
    owner: &str,
    operation: &str,
    completion: &NativeCompletion,
    account: &crate::store::provider_accounts::Account,
    revision: i64,
) -> Result<bool, &'static str> {
    let receipt = store
        .provider_login_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?;
    let key = store
        .machine_encryption_public_key(&completion.machine_id)
        .await
        .map_err(|_| "unavailable")?;
    Ok(receipt.as_ref().is_some_and(|receipt| {
        !receipt.cancel_requested
            && receipt.dispatch_rejection.is_none()
            && receipt.native_logout.is_none()
            && receipt.native_completion.as_ref() == Some(completion)
            && receipt
                .native_binding_revision
                .is_some_and(|bound| account.admits_binding(bound, revision))
    }) && key.as_ref().is_some_and(|key| {
        crate::admin::hex_sha256(key.as_bytes()) == completion.recipient_fingerprint
    }) && completion
        .profile
        .identity
        .fingerprint(&account.provider_id)
        .is_ok_and(|fingerprint| account.identity_fingerprint.as_ref() == Some(&fingerprint)))
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
    let (mut outdated, mut pending) = (false, false);
    for machine in machines {
        let Some((operation, completion)) = store
            .native_login_for_session(owner, id, &machine.id)
            .await
            .map_err(|_| "unavailable")?
        else {
            // An unfinished newer sign-in blocks this Machine; say so instead
            // of implying the account must sign in from scratch.
            pending |= store
                .native_login_pending(owner, id, &machine.id)
                .await
                .unwrap_or(false);
            continue;
        };
        let Ok(installed) = active_machine_plugin(
            std::slice::from_ref(machine),
            &machine.id,
            &account.provider_id,
        ) else {
            continue;
        };
        if !observes_binding(
            store,
            owner,
            &operation,
            &completion,
            account,
            registry.revision,
        )
        .await?
        {
            continue;
        }
        if let Err(error) = resolve_provider_generation_with_auth(
            &state.provider_catalog,
            &state.plugin_catalog,
            &[installed],
            &account.provider_id,
            SchedulingAuthentication::Native(&completion.profile),
        ) {
            // A Provider upgrade invalidates native logins by design; say so
            // instead of implying the account never signed in.
            outdated |= error == "native_provider_generation_mismatch";
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
    Err(if pending {
        "native_login_pending"
    } else if outdated {
        "native_login_outdated"
    } else {
        "native_login_required"
    })
}

fn error_message(code: &str) -> &str {
    match code {
        "account_disabled" => "This account is disabled.",
        "account_usage_unsupported" => "This Provider does not support background account usage.",
        "native_login_required" => "Sign in on a Machine to query this account's usage.",
        "native_login_pending" => {
            "A newer sign-in has not finished. Finish or cancel it in Settings → Subscriptions."
        }
        "native_login_outdated" => {
            "The Provider was updated after this account signed in. Sign in again to use it and query its usage."
        }
        "machine_usage_update_required" => {
            "The signed-in Machine needs an update to query this account's usage."
        }
        "machine_unavailable" => {
            "The signed-in Machine is offline. Usage will refresh when it reconnects."
        }
        "native_usage_authentication_required" => {
            "This Machine's native login needs renewal. Sign in again to refresh usage."
        }
        "native_usage_busy" | "machine_usage_busy" => {
            "This Machine is busy. Cowboy will retry the usage query automatically."
        }
        "runtime_not_allowed" => "This Provider cannot currently run on the signed-in Machine.",
        "native_usage_collector_unavailable" | "native_usage_home_unsupported" => {
            "The installed Provider cannot query this native login. Check its Machine installation."
        }
        "native_usage_profile_changed" | "account_binding_changed" => {
            "This account's login changed. Cowboy will query the current login."
        }
        "native_usage_collector_failed" => {
            "The native quota query failed. Cowboy will retry automatically."
        }
        "usage_admission_unavailable" => {
            "Usage collection is paused. The Service admission ledger needs repair and a restart."
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
    // A read neither takes the database admission write lock nor binds a cache
    // generation. Delayed reads cannot erase a newer login's observation.
    let entry = cache.data.lock().snapshot(&target.completion);
    let budget = state.usage.limiter.view(
        &target.provider,
        &crate::usage::limiter::Limiter::account_key(owner, id.as_str()),
        now_ms(),
    );
    let retry_at_ms = budget.retry_at_ms(now_ms());
    let stopped = budget.next_allowed_ms == i64::MAX;
    value["usage"] = json!(entry.usage);
    if value["usage"].is_object() {
        value["usage"]["refresh"] = json!({
            "last_attempt_at_ms":entry.last_attempt_at_ms,
            "manual_refresh_after_ms":retry_at_ms,
            "next_auto_refresh_at_ms":retry_at_ms,
            "stale":stopped || entry.error.is_some(),
        });
    }
    value["last_attempt_at_ms"] = json!(budget.last_attempt_ms);
    value["next_refresh_at_ms"] = json!(retry_at_ms);
    if let Some(code) = if stopped {
        Some("usage_admission_unavailable")
    } else {
        entry.error
    } {
        value["reason"] = json!(code);
        value["message"] = json!(error_message(code));
    }
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
        accounts.push(
            read_one(state, owner, &account.id)
                .await
                .unwrap_or_else(|code| {
                    json!({"account_id":account.id,"provider_id":account.provider_id,
                "reason":code,"message":error_message(code),"refreshing":false})
                }),
        );
    }
    Ok(json!({"accounts":accounts}))
}

/// Owners whose named accounts are being refreshed by a global refresh.
static OWNER_REFRESHES: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

/// Refresh every available named account of `owner` once, serially, as part
/// of a global usage refresh. A global refresh arriving while one is running
/// joins it instead of starting another, and each account keeps its own
/// budget, so an account that is not due sends nothing. Results publish
/// through the shared cache and its change broadcast.
pub(in crate::server) fn refresh_owner_accounts(state: &Arc<AppState>, owner: &str) {
    struct Running(String);
    impl Drop for Running {
        fn drop(&mut self) {
            OWNER_REFRESHES.lock().remove(&self.0);
        }
    }
    if !OWNER_REFRESHES.lock().insert(owner.to_owned()) {
        return;
    }
    let running = Running(owner.to_owned());
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let owner = running.0.clone();
        let Some(store) = state.store.as_ref() else {
            return;
        };
        let Ok(registry) = store.provider_accounts(&owner).await else {
            return;
        };
        for account in registry
            .accounts
            .iter()
            .filter(|account| account.available())
        {
            let _ = refresh_one(&state, &owner, &account.id).await;
        }
        drop(running);
    });
}

pub(in crate::server) async fn refresh_one(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(), &'static str> {
    let state = Arc::clone(state);
    let owner = owner.to_owned();
    let id = id.clone();
    // Own the entire admission transaction, not just the later network query.
    // An HTTP/MCP observer may detach while the transaction is committing.
    tokio::spawn(async move { refresh_owned(&state, &owner, &id).await })
        .await
        .map_err(|_| "native_usage_unavailable")?
}

async fn refresh_owned(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<(), &'static str> {
    // Resolve ownership before allocating a cache slot for an untrusted ID.
    target(state, owner, id).await?;
    let cache = state.usage.native.entry(owner, id.as_str());
    let collection = Arc::clone(&cache.collection).lock_owned().await;
    let target = target(state, owner, id).await?;
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    let mut permit = None;
    let due = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .with_native_login_binding(owner, &target.operation, &target.completion, || {
            let mut entry = cache.data.lock();
            entry.bind(&target.completion);
            let due = entry.due(now_ms())
                && match state.usage.limiter.start(&target.provider, &key, now_ms()) {
                    Ok(value) => {
                        permit = Some(value);
                        true
                    }
                    Err(_) => false,
                };
            if due {
                entry.last_attempt_at_ms = now_ms();
            }
            Ok(due)
        })
        .await;
    let due = match due {
        Ok(due) => due,
        Err(_) => {
            if let Some(permit) = permit {
                permit.finish(crate::usage::limiter::Outcome::LocalFailure, now_ms());
            }
            return Err("account_binding_changed");
        }
    };
    if !due {
        return Ok(());
    }
    state.hub.broadcast_account_usage_changed(owner);
    let result = collect(state, owner, &target).await;
    let outcome = collection_outcome(result.as_ref().err().copied());
    let succeeded = result.is_ok();
    cache.data.lock().finish(&target.completion, result);
    if succeeded {
        state.usage.native.persist();
    }
    if let Some(permit) = permit {
        permit.finish(outcome, now_ms());
    }
    drop(collection);
    state.hub.broadcast_account_usage_changed(owner);
    tracing::debug!(
        account_id = id.as_str(),
        "native account usage refresh settled"
    );
    Ok(())
}

fn collection_outcome(error: Option<&str>) -> crate::usage::limiter::Outcome {
    use crate::usage::limiter::Outcome;
    match error {
        None => Outcome::Success,
        Some("native_usage_authentication_required") => Outcome::Authentication,
        Some(
            "machine_unavailable"
            | "machine_usage_update_required"
            | "machine_connection_changed"
            | "native_usage_busy"
            | "machine_usage_busy"
            | "account_binding_changed"
            | "native_usage_profile_changed"
            | "native_usage_collector_unavailable"
            | "native_usage_home_unsupported"
            | "runtime_not_allowed"
            | "native_usage_dispatch_failed",
        ) => Outcome::LocalFailure,
        Some(_) => Outcome::Failure,
    }
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
            // Keep transport/admission reasons separate from a database binding
            // rejection, without exporting arbitrary internal error text.
            Ok(
                if state
                    .supervisor
                    .runtime_allowed(&completion.profile.provider_id, &completion.machine_id)
                {
                    state
                        .machine_control
                        .begin_native_usage(&connection, &completion.profile)
                        .map_err(|code| dispatch_error(&code))
                } else {
                    Err("runtime_not_allowed")
                },
            )
        })
        .await
        .map_err(|_| "account_binding_changed")??;
    let raw = pending
        .await
        .map_err(|code| crate::machine_protocol::native_usage_error_code(&code))?;
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

fn dispatch_error(code: &str) -> &'static str {
    match code {
        "Machine pending request budget exceeded" => "machine_usage_busy",
        "Machine is not connected" | "Machine disconnected" => "machine_unavailable",
        "Machine operation connection is no longer current" => "machine_connection_changed",
        _ => "native_usage_dispatch_failed",
    }
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
        let mut candidates = Vec::new();
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
                        candidates.push((user.id.clone(), account.id.clone()));
                    }
                }
            }
        }
        // Oldest first prevents a low Provider budget from always favouring
        // the first accounts in registry order.
        candidates.sort_by_key(|(owner, id)| {
            state
                .usage
                .limiter
                .view(
                    "",
                    &crate::usage::limiter::Limiter::account_key(owner, id.as_str()),
                    now_ms(),
                )
                .last_attempt_ms
        });
        for (owner, id) in candidates {
            let _ = refresh_one(&state, &owner, &id).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outdated_login_explains_the_provider_update() {
        let outdated = error_message("native_login_outdated");
        assert_ne!(outdated, error_message("native_login_required"));
        assert!(outdated.contains("updated") && outdated.contains("Sign in again"));
        let pending = error_message("native_login_pending");
        assert!(pending.contains("Finish or cancel"), "{pending}");
    }

    #[test]
    fn local_native_failures_leave_other_provider_accounts_available() {
        use crate::usage::limiter::{Limiter, Outcome};
        for reason in [
            "machine_unavailable",
            "machine_usage_update_required",
            "machine_connection_changed",
            "native_usage_busy",
            "machine_usage_busy",
            "account_binding_changed",
            "native_usage_profile_changed",
            "native_usage_collector_unavailable",
            "native_usage_home_unsupported",
            "runtime_not_allowed",
            "native_usage_dispatch_failed",
        ] {
            let limiter = Limiter::new(None);
            limiter
                .start("anthropic", "offline", 1000)
                .unwrap_or_else(|_| panic!("start"))
                .finish(collection_outcome(Some(reason)), 1000);
            limiter
                .start("anthropic", "online", 61_000)
                .unwrap_or_else(|_| panic!("{reason}"))
                .finish(Outcome::Success, 61_000);
        }
        assert_eq!(
            collection_outcome(Some("native_usage_collector_failed")),
            Outcome::Failure
        );
        assert_eq!(
            collection_outcome(Some("native_usage_authentication_required")),
            Outcome::Authentication
        );
    }
}

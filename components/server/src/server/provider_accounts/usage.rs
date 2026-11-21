//! Named account quota: native subscriptions (a login on one Machine) and
//! portable accounts (a Service vault credential on every Machine, see
//! [`portable`]). The same owner/binding admission serves UI and agents.
use super::*;
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::logins::NativeCompletion;
use crate::usage::native::Scope;

mod portable;

pub(in crate::server) mod reset;

#[derive(Clone)]
struct Target {
    operation: String,
    completion: NativeCompletion,
    provider: String,
    source: String,
    /// The exact installed release verified for this login; a reset is
    /// decided from it, never from a later second lookup.
    installed: crate::machine_protocol::PluginInventory,
}

/// The read model's account target: which credential an observation binds.
enum Resolved {
    Native(Box<Target>),
    Portable(Box<portable::Target>),
}

impl Resolved {
    fn scope(&self) -> Scope {
        match self {
            Self::Native(target) => Scope::from(target.completion.clone()),
            Self::Portable(target) => Scope::Portable(target.scope.clone()),
        }
    }

    fn provider(&self) -> &str {
        match self {
            Self::Native(target) => &target.provider,
            Self::Portable(target) => &target.binding.account,
        }
    }
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

/// Internal: a verified native login exists, but the generation that would
/// query it declares no account usage (an Antigravity Machine before 0.2.10).
/// Public as `account_usage_unsupported`; the read model still reports the
/// sign-in (`signed_in`).
const SIGNED_IN_USAGE_UNSUPPORTED: &str = "account_usage_unsupported_signed_in";

fn public_code(code: &'static str) -> &'static str {
    if code == SIGNED_IN_USAGE_UNSUPPORTED {
        "account_usage_unsupported"
    } else {
        code
    }
}

/// What a native login is resolved for: any usage-capable login, or only
/// a login whose Machine can spend a banked reset under host policy.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Usage,
    Reset,
}

async fn target(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<Resolved, &'static str> {
    // One binding serves the read and its reset support: a login that can
    // also spend a reset wins over a read-only one on another Machine.
    match target_for(state, owner, id, Purpose::Reset).await {
        Ok(resolved) => Ok(resolved),
        Err(_) => target_for(state, owner, id, Purpose::Usage).await,
    }
}

async fn target_for(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    purpose: Purpose,
) -> Result<Resolved, &'static str> {
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
    // A vault account's sessions run from its Service replica on any
    // Machine, whatever native login it may also have.
    if super::portable::provider(state, &account.provider_id) {
        let sign_in = store
            .latest_vault_sign_in(owner, id)
            .await
            .map_err(|_| "unavailable")?;
        let portable =
            portable::target(state, account, sign_in).unwrap_or(Err("account_usage_unsupported"));
        return portable.map(|target| Resolved::Portable(Box::new(target)));
    }
    if !state
        .provider_catalog
        .native_account_providers()
        .contains(&account.provider_id)
    {
        return Err("account_usage_unsupported");
    }
    let machines = store.list_machines().await.map_err(|_| "unavailable")?;
    let mut machines: Vec<_> = machines
        .iter()
        .filter(|machine| !machine.revoked)
        // A reset considers only logins whose Machine can spend one under
        // host policy, before ranking: an old or disallowed login never
        // masks an eligible one.
        .filter(|machine| {
            purpose == Purpose::Usage || resets_on(state, &account.provider_id, &machine.id)
        })
        .collect();
    // Prefer the login whose Machine can serve every named-usage operation
    // (reads and resets) under host policy, so an old or disallowed login
    // never masks an eligible one; the order is deterministic.
    machines.sort_by_key(|machine| {
        (
            machine_rank(state, &account.provider_id, &machine.id),
            machine.id.clone(),
        )
    });
    let (mut outdated, mut pending, mut unsupported, mut read_only) = (false, false, false, false);
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
        let provider = match usage_provider(state, &installed, &completion.profile) {
            Ok(provider) => provider,
            // A Provider upgrade invalidates a native Claude login by design;
            // say so instead of implying the account never signed in.
            Err(UsageGeneration::Outdated) => {
                outdated = true;
                continue;
            }
            Err(UsageGeneration::Unavailable) => continue,
            // Another login may still declare usage: keep looking.
            Err(UsageGeneration::Unsupported) => {
                unsupported = true;
                continue;
            }
        };
        let Some(binding) = state
            .usage
            .plugin_bindings()
            .into_iter()
            .find(|binding| binding.account == provider)
        else {
            unsupported = true;
            continue;
        };
        // A reset considers only a login whose exact installed release
        // declares resets for this account, before any later login.
        if purpose == Purpose::Reset
            && reset::exact_reset_binding(state, &installed, &provider).is_none()
        {
            read_only = true;
            continue;
        }
        return Ok(Resolved::Native(Box::new(Target {
            operation,
            completion,
            provider,
            source: binding.product_label().into(),
            installed,
        })));
    }
    Err(if purpose == Purpose::Reset && read_only {
        // Verified logins exist, but none can spend a reset.
        "account_reset_unsupported"
    } else if pending {
        "native_login_pending"
    } else if outdated {
        "native_login_outdated"
    } else if unsupported {
        SIGNED_IN_USAGE_UNSUPPORTED
    } else {
        "native_login_required"
    })
}

/// Whether a login's Machine can spend a named reset under host policy.
fn resets_on(state: &AppState, provider_id: &str, machine_id: &str) -> bool {
    state
        .machine_control
        .connection_protocol(machine_id)
        .is_some_and(|(_, version)| {
            version >= crate::machine_protocol::NATIVE_RESET_PROTOCOL_VERSION
        })
        && state
            .supervisor
            .vendor_request_allowed(provider_id, machine_id)
}

fn machine_rank(state: &AppState, provider_id: &str, machine_id: &str) -> (bool, u8) {
    login_rank(
        state
            .machine_control
            .connection_protocol(machine_id)
            .map(|(_, version)| version),
        state
            .supervisor
            .vendor_request_allowed(provider_id, machine_id),
    )
}

/// Placement rank of a native login's Machine: allowed by host policy and
/// reset-capable first, then usage-capable, then connected, then offline.
fn login_rank(protocol: Option<u16>, allowed: bool) -> (bool, u8) {
    use crate::machine_protocol::{NATIVE_RESET_PROTOCOL_VERSION, NATIVE_USAGE_PROTOCOL_VERSION};
    let capability = match protocol {
        Some(version) if version >= NATIVE_RESET_PROTOCOL_VERSION => 0,
        Some(version) if version >= NATIVE_USAGE_PROTOCOL_VERSION => 1,
        Some(_) => 2,
        None => 3,
    };
    (!allowed || protocol.is_none(), capability)
}

/// Why a Machine's installed generation cannot serve an account's usage.
#[derive(Debug, PartialEq, Eq)]
enum UsageGeneration {
    /// The native login predates an incompatible Provider update.
    Outdated,
    /// This Machine's installation cannot run the query now.
    Unavailable,
    /// The generation that would run it declares no account usage.
    Unsupported,
}

/// The account-usage provider of the generation a usage query runs on.
///
/// A native Claude login binds its exact signed release: the query runs only
/// where that login still resolves, and that release's declaration decides.
/// An account-sandbox Provider (Antigravity) keeps its credential in the
/// daemon's account home, independent of the Provider release: the query
/// runs on the Machine's active trusted generation, whose declaration
/// decides, and the daemon admits only the login's current signed-in
/// credential generation of an unfenced account.
fn usage_provider(
    state: &AppState,
    installed: &crate::machine_protocol::PluginInventory,
    profile: &crate::provider_account::NativeProfileReceipt,
) -> Result<String, UsageGeneration> {
    let package = state.provider_catalog.package(
        &installed.plugin_id,
        &installed.plugin_version,
        &installed.generation_digest,
    );
    let resolve = |authentication| {
        resolve_provider_generation_with_auth(
            &state.provider_catalog,
            &state.plugin_catalog,
            std::slice::from_ref(installed),
            &profile.provider_id,
            authentication,
        )
        .map(drop)
    };
    choose_usage_provider(
        package.as_ref(),
        installed,
        profile,
        || resolve(SchedulingAuthentication::Native(profile)),
        || resolve(SchedulingAuthentication::Discovery),
        || {
            state.provider_catalog.account_usage_provider(
                &profile.provider_id,
                &profile.provider_version,
                &profile.generation_digest,
            )
        },
    )
}

/// [`usage_provider`]'s decision, given the installed generation's trusted
/// package and the resolutions it may need.
fn choose_usage_provider(
    installed_package: Option<&cowboy_provider_sdk::ProviderPackage>,
    installed: &crate::machine_protocol::PluginInventory,
    profile: &crate::provider_account::NativeProfileReceipt,
    resolve_login: impl FnOnce() -> Result<(), String>,
    resolve_installed: impl FnOnce() -> Result<(), String>,
    signed_in_usage: impl FnOnce() -> Option<String>,
) -> Result<String, UsageGeneration> {
    let sandboxed = installed_package.is_some_and(|package| {
        package.manifest.runtime.isolation
            == Some(cowboy_provider_sdk::RuntimeIsolationPolicy::AccountSandboxV1)
    });
    if !sandboxed {
        resolve_login().map_err(|error| {
            if error == "native_provider_generation_mismatch" {
                UsageGeneration::Outdated
            } else {
                UsageGeneration::Unavailable
            }
        })?;
        return signed_in_usage().ok_or(UsageGeneration::Unsupported);
    }
    if profile.account_id.is_default()
        || profile.generation == 0
        || profile.provider_id != installed.plugin_id
    {
        return Err(UsageGeneration::Unavailable);
    }
    resolve_installed().map_err(|_| UsageGeneration::Unavailable)?;
    installed_package
        .and_then(|package| package.manifest.host.account_usage.as_ref())
        .map(|usage| usage.provider.clone())
        .ok_or(UsageGeneration::Unsupported)
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
            "The Machine that would query this account's usage needs an update."
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
        "runtime_not_allowed" => {
            "This Provider cannot currently run on the Machine that would query its usage."
        }
        "native_usage_collector_unavailable" | "native_usage_home_unsupported" => {
            "The installed Provider cannot query this native login. Check its Machine installation."
        }
        "native_usage_profile_changed" | "account_binding_changed" => {
            "This account's login changed. Cowboy will query the current login."
        }
        "native_usage_collector_failed" => {
            "The native quota query failed. Cowboy will retry automatically."
        }
        "portable_credentials_required" => {
            "Sign in or enter this account's API key in Settings → Subscriptions to query its usage."
        }
        "portable_usage_machine_unavailable" => {
            "No connected Machine can query this account's usage. Usage will refresh when one reconnects."
        }
        "portable_usage_replica_pending" => {
            "The Machine has not received this account's current credentials yet. Cowboy will retry automatically."
        }
        "portable_usage_authentication_required" => {
            "This account's sign-in needs renewal. Sign in again in Settings → Subscriptions to refresh usage."
        }
        "portable_usage_busy" => {
            "This account's credentials are being updated. Cowboy will retry the usage query automatically."
        }
        "portable_usage_throttled" => {
            "The Provider asked Cowboy to slow down. Usage will refresh later."
        }
        "portable_usage_refresh_unsupported" => {
            "This Provider's installed usage collector could renew the account's sign-in, so it is not run for this account. A Provider update that never renews during usage queries enables it."
        }
        "portable_usage_collector_unavailable" | "portable_usage_account_required" => {
            "The installed Provider cannot query this account's usage. Check its Machine installation."
        }
        "portable_usage_collector_failed" | "portable_usage_temporarily_unavailable" => {
            "The account quota query failed. Cowboy will retry automatically."
        }
        "usage_admission_unavailable" => {
            "Usage collection is paused. The Service admission ledger needs repair and a restart."
        }
        "account_reset_unsupported" | "native_reset_unsupported" => {
            "This account's Provider does not offer usage resets."
        }
        "usage_reset_paced" => {
            "Usage requests to this Provider are paced; no reset was sent. Try again in a minute."
        }
        "native_reset_outcome_unknown" | "reset_outcome_unknown" => {
            "The reset outcome is unknown. Check credits before trying again."
        }
        "native_reset_expired" => "The reset was not started in time; no reset was sent.",
        "reset_confirmation_required" => "Type confirm to use a reset.",
        "reset_credit_invalid" => {
            "The confirmed reset is invalid. Refresh usage and confirm again."
        }
        "reset_authentication_required" => {
            "This account's sign-in needs renewal before a reset can be used."
        }
        "reset_credit_changed" => "The available reset changed. Refresh usage and confirm again.",
        "reset_unavailable" => "No reset is currently usable for this account.",
        "reset_refused" => "The Provider refused this reset; no reset was sent.",
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
            if code == SIGNED_IN_USAGE_UNSUPPORTED {
                value["signed_in"] = json!(true);
            }
            let code = public_code(code);
            value["reason"] = json!(code);
            value["message"] = json!(error_message(code));
            return Ok(value);
        }
    };
    value["machine_id"] = json!(match &target {
        Resolved::Native(native) => Some(native.completion.machine_id.clone()),
        Resolved::Portable(portable) => portable::selected_machine(state, owner, id, portable),
    });
    value["available"] = json!(true);
    value["signed_in"] = json!(true);
    // Whether a reset of exactly this resolved binding can be spent now.
    value["resets_supported"] = json!(reset::supported_for(state, &target));
    let cache = state.usage.native.entry(owner, id.as_str());
    value["refreshing"] = json!(cache.collection.try_lock().is_err());
    // A read neither takes the database admission write lock nor binds a cache
    // generation. Delayed reads cannot erase a newer login's observation.
    let entry = cache.data.lock().snapshot(&target.scope());
    let budget = state.usage.limiter.view(
        target.provider(),
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
    target(state, owner, id).await.map_err(public_code)?;
    let cache = state.usage.native.entry(owner, id.as_str());
    let collection = Arc::clone(&cache.collection).lock_owned().await;
    let target = match target(state, owner, id).await.map_err(public_code)? {
        Resolved::Native(target) => *target,
        Resolved::Portable(target) => {
            return portable::refresh(state, owner, id, &target, &cache, collection).await;
        }
    };
    let scope = Scope::from(target.completion.clone());
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    let mut permit = None;
    let due = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .with_native_login_binding(owner, &target.operation, &target.completion, || {
            let mut entry = cache.data.lock();
            entry.bind(&scope);
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
    cache.data.lock().finish(&scope, result);
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
        Some("native_usage_authentication_required" | "portable_usage_authentication_required") => {
            Outcome::Authentication
        }
        Some("portable_usage_throttled") => Outcome::Throttled,
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
            | "native_usage_dispatch_failed"
            | "portable_usage_machine_unavailable"
            | "portable_usage_replica_pending"
            | "portable_usage_busy"
            | "portable_usage_account_required"
            | "portable_usage_collector_unavailable"
            | "portable_usage_refresh_unsupported"
            | "portable_usage_dispatch_failed",
        ) => Outcome::LocalFailure,
        Some(_) => Outcome::Failure,
    }
}

async fn collect(state: &AppState, owner: &str, target: &Target) -> Result<Value, &'static str> {
    let completion = &target.completion;
    let store = state.store.as_ref().ok_or("unavailable")?;
    let (connection, _) = state
        .machine_control
        .connection_protocol(&completion.machine_id)
        .ok_or("machine_unavailable")?;
    let pending = store
        .with_native_login_binding(owner, &target.operation, completion, || {
            // Keep transport/admission reasons separate from a database binding
            // rejection, without exporting arbitrary internal error text.
            Ok(
                if state
                    .supervisor
                    .vendor_request_allowed(&completion.profile.provider_id, &completion.machine_id)
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
        anyhow::ensure!(state.supervisor.vendor_request_allowed(&completion.profile.provider_id, &completion.machine_id), "runtime_not_allowed");
        anyhow::ensure!(
            usage_provider(state, &installed, &completion.profile).as_deref() == Ok(target.provider.as_str()),
            "native_usage_unavailable"
        );
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

    fn package(source: &str) -> cowboy_provider_sdk::ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(source).unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    fn installed(
        package: &cowboy_provider_sdk::ProviderPackage,
    ) -> crate::machine_protocol::PluginInventory {
        serde_json::from_value(json!({
            "plugin_id": package.manifest.id, "plugin_version": package.manifest.version,
            "generation_digest": "sha256:installed", "contract_fingerprint": "contract",
            "state": "active",
        }))
        .unwrap()
    }

    fn profile(
        provider: &str,
        version: &str,
        generation: u64,
    ) -> crate::provider_account::NativeProfileReceipt {
        serde_json::from_value(json!({
            "account_id": "11111111-1111-4111-8111-111111111111",
            "provider_id": provider, "request_id": "login", "generation": generation,
            "method": "google-subscription", "provider_version": version,
            "generation_digest": "sha256:signed-in", "auth_contract_fingerprint": "contract",
            "identity": {"authority": "google", "subject": "owner", "tenant": null},
        }))
        .unwrap()
    }

    fn mismatch() -> Result<(), String> {
        Err("native_provider_generation_mismatch".into())
    }

    #[test]
    fn a_sandboxed_login_reads_quota_on_the_installed_generation_across_upgrades() {
        let antigravity = package(include_str!(
            "../../../../../plugins/antigravity/provider.json"
        ));
        let installed = installed(&antigravity);
        // Signed in on an earlier release that declared no usage: the
        // installed generation's declaration decides, never the login's.
        let old = profile("antigravity", "0.2.5", 1);
        assert_eq!(
            choose_usage_provider(
                Some(&antigravity),
                &installed,
                &old,
                mismatch,
                || Ok(()),
                || None
            ),
            Ok("antigravity".to_owned())
        );
        // The installed generation must still be trusted on this Machine.
        assert_eq!(
            choose_usage_provider(
                Some(&antigravity),
                &installed,
                &old,
                || Ok(()),
                || Err("untrusted".into()),
                || None
            ),
            Err(UsageGeneration::Unavailable)
        );
        for invalid in [
            profile("antigravity", "0.2.5", 0),
            profile("claude-code", "0.2.5", 1),
        ] {
            assert_eq!(
                choose_usage_provider(
                    Some(&antigravity),
                    &installed,
                    &invalid,
                    || Ok(()),
                    || Ok(()),
                    || None
                ),
                Err(UsageGeneration::Unavailable)
            );
        }
        let mut silent = antigravity.clone();
        silent.manifest.host.account_usage = None;
        assert_eq!(
            choose_usage_provider(
                Some(&silent),
                &installed,
                &old,
                || Ok(()),
                || Ok(()),
                || None
            ),
            Err(UsageGeneration::Unsupported)
        );
    }

    #[test]
    fn a_native_claude_login_still_binds_its_signed_release() {
        let claude = package(include_str!(
            "../../../../../plugins/claude-code/provider.json"
        ));
        let installed = installed(&claude);
        let login = profile("claude-code", "3.0.0", 1);
        assert_eq!(
            choose_usage_provider(
                Some(&claude),
                &installed,
                &login,
                mismatch,
                || Ok(()),
                || Some("x".into())
            ),
            Err(UsageGeneration::Outdated)
        );
        assert_eq!(
            choose_usage_provider(
                Some(&claude),
                &installed,
                &login,
                || Err("other".into()),
                || Ok(()),
                || None
            ),
            Err(UsageGeneration::Unavailable)
        );
        assert_eq!(
            choose_usage_provider(
                Some(&claude),
                &installed,
                &login,
                || Ok(()),
                || Ok(()),
                || None
            ),
            Err(UsageGeneration::Unsupported)
        );
        assert_eq!(
            choose_usage_provider(
                Some(&claude),
                &installed,
                &login,
                || Ok(()),
                || Ok(()),
                || Some("anthropic".into())
            ),
            Ok("anthropic".to_owned())
        );
    }

    #[test]
    fn an_old_or_disallowed_login_never_masks_an_eligible_one() {
        use crate::machine_protocol::{
            NATIVE_RESET_PROTOCOL_VERSION, NATIVE_USAGE_PROTOCOL_VERSION,
        };
        let eligible = login_rank(Some(NATIVE_RESET_PROTOCOL_VERSION), true);
        for worse in [
            login_rank(Some(NATIVE_USAGE_PROTOCOL_VERSION), true),
            login_rank(Some(NATIVE_RESET_PROTOCOL_VERSION), false),
            login_rank(Some(1), true),
            login_rank(None, true),
        ] {
            assert!(eligible < worse, "{worse:?}");
        }
        assert!(
            login_rank(Some(NATIVE_USAGE_PROTOCOL_VERSION), true)
                < login_rank(Some(NATIVE_RESET_PROTOCOL_VERSION), false)
        );
    }

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

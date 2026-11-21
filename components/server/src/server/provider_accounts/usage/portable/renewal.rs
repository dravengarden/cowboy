//! Renewal of a refresh-custody managed account (protocol 51): after the
//! refresh-free collector reports authentication required inside one
//! budgeted collection, or when a Machine running the account's sessions
//! reports its access token is about to expire. At most one renewal per
//! account is recorded at a time (`usage::custody`); a pending one only
//! ever resumes on its own Machine and never runs twice.
use super::*;
use crate::machine_protocol::PluginInventory;
use crate::machine_protocol::refresh_custody::{PortableRenewalReply, PortableRenewalRequest};
use crate::usage::custody::{Admission, RenewalState};
use crate::usage::portable::RenewalDispatch;

use super::super::super::portable::custody as rules;

const REQUIRED: &str = "portable_usage_authentication_required";
const NOT_RETAINED: &str = "portable_renewal_not_retained";
const UNREADABLE: &str = "portable_renewal_unreadable";

/// Whether a failed refresh-free collection may renew on `machine`.
pub(super) fn eligible(
    state: &AppState,
    id: &ProviderAccountId,
    target: &Target,
    machine: &str,
) -> bool {
    rules::managed(state, id, &target.plugin_id)
        && rules::machine_admits(state, id, machine, &target.plugin_id).is_ok()
}

/// Run (or resume) one recorded renewal on `machine` and return the
/// collector output it produced. Every outcome is recorded before return.
pub(super) async fn renew(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    machine: &str,
    plugin: &PluginInventory,
    usage: PortableUsageRequest,
) -> Result<Value, &'static str> {
    if !eligible(state, id, target, machine) {
        return Err(REQUIRED);
    }
    let custody = &state.usage.custody;
    let connected = |candidate: &str| {
        state.machine_control.machine_supports(
            candidate,
            crate::machine_protocol::REFRESH_CUSTODY_PROTOCOL_VERSION,
        )
    };
    let (renewal, resume) = match custody.admit(
        id.as_str(),
        target.auth_generation,
        machine,
        connected,
        now_ms(),
    ) {
        Admission::Fresh(renewal) => (renewal, false),
        Admission::Resume(renewal) if renewal.machine_id == machine => (renewal, true),
        Admission::Resume(_) => return Err(REQUIRED),
        Admission::Refused(code) => return Err(code),
    };
    let Some(_flight) = custody.fly(&renewal.renewal_id) else {
        return Err("portable_usage_busy");
    };
    let envelope = if resume {
        None
    } else {
        match full_envelope(state, id, &target.plugin_id, machine).await {
            Ok(envelope) => Some(envelope),
            Err(code) => {
                // Nothing left the Controller: the renewal never ran.
                custody.settle(id.as_str(), &renewal.renewal_id, RenewalState::Settled);
                return Err(code);
            }
        }
    };
    let request = PortableRenewalRequest {
        renewal_id: renewal.renewal_id.clone(),
        usage,
        envelope,
        resume_only: resume,
    };
    let (pending, code) = under_account_lock(state, owner, id, target, || {
        crate::usage::portable::begin_renewal(&state.machine_control, machine, plugin, request)
    })
    .await;
    let Some(pending) = pending else {
        if !resume {
            custody.settle(id.as_str(), &renewal.renewal_id, RenewalState::Settled);
        }
        return Err(code);
    };
    match pending.await {
        // Unknown outcome: the record stays pending and only resumes.
        RenewalDispatch::Lost => Err("portable_usage_temporarily_unavailable"),
        RenewalDispatch::Refused(code) if code == "portable_usage_busy" => {
            Err("portable_usage_busy")
        }
        RenewalDispatch::Refused(code) if code == NOT_RETAINED => {
            tracing::warn!(account_id = id.as_str(), %machine,
                "a lost renewal reply cannot be recovered; renewal stays closed until a new sign-in");
            custody.settle(id.as_str(), &renewal.renewal_id, RenewalState::Unknown);
            Err(REQUIRED)
        }
        // Every other Machine refusal happens before the collector starts.
        RenewalDispatch::Refused(code) => {
            custody.settle(id.as_str(), &renewal.renewal_id, RenewalState::Settled);
            Err(crate::machine_protocol::portable_usage::portable_usage_error_code(&code))
        }
        RenewalDispatch::Replied(value) => settle_reply(state, id, target, &renewal, value).await,
    }
}

async fn settle_reply(
    state: &Arc<AppState>,
    id: &ProviderAccountId,
    target: &Target,
    renewal: &crate::usage::custody::Renewal,
    value: Value,
) -> Result<Value, &'static str> {
    let custody = &state.usage.custody;
    let reply = serde_json::from_value::<PortableRenewalReply>(value)
        .ok()
        .filter(|reply| reply.renewal_id == renewal.renewal_id);
    let Some(reply) =
        reply.filter(|reply| reply.usage.get("error").and_then(Value::as_str) != Some(UNREADABLE))
    else {
        custody.settle(id.as_str(), &renewal.renewal_id, RenewalState::Unknown);
        return Err(REQUIRED);
    };
    let applied = match &reply.bundle {
        None => Ok(false),
        Some(bundle) => {
            rules::apply_renewal(
                state,
                id,
                &target.plugin_id,
                bundle,
                renewal.vault_generation,
            )
            .await
        }
    };
    let state_after = match &applied {
        Ok(true) => RenewalState::Rotated,
        Ok(false) => RenewalState::Settled,
        Err(error) => {
            tracing::warn!(account_id = id.as_str(), %error, "a renewed credential was not applied");
            RenewalState::Unknown
        }
    };
    custody.settle(id.as_str(), &renewal.renewal_id, state_after);
    applied.map_err(|_| REQUIRED)?;
    match reply.usage.get("error").and_then(Value::as_str) {
        Some(code) => Err(crate::machine_protocol::portable_usage::portable_usage_error_code(code)),
        None => Ok(reply.usage),
    }
}

/// The vault's full current credential, sealed to `machine` only.
async fn full_envelope(
    state: &AppState,
    id: &ProviderAccountId,
    provider: &str,
    machine: &str,
) -> Result<crate::machine_protocol::SealedProviderAuth, &'static str> {
    let key = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .machine_encryption_public_key(machine)
        .await
        .ok()
        .flatten()
        .ok_or("portable_usage_machine_unavailable")?;
    super::super::super::portable::scope(state, id, provider)
        .ok_or("portable_credentials_required")?
        .seal_for_machine(provider, &key)
        .map_err(|_| REQUIRED)
}

/// A Machine running the account's sessions reports its access-only copy
/// at `generation` expires soon. Renew there once, unless the vault moved
/// on, a renewal is recorded, or one settled at this generation recently.
pub(in crate::server) async fn needed(
    state: &Arc<AppState>,
    machine: &str,
    id: &ProviderAccountId,
    provider: &str,
    generation: u64,
) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let Ok(Some(owner)) = store.provider_account_owner(id).await else {
        return;
    };
    let Ok(Resolved::Portable(target)) = super::super::target(state, &owner, id).await else {
        return;
    };
    if target.plugin_id != provider
        || target.auth_generation != generation
        || state
            .usage
            .custody
            .recently_settled(id.as_str(), generation, now_ms())
    {
        return;
    }
    let Some(plugin) = state
        .machine_control
        .connected_plugin_inventory()
        .into_iter()
        .find(|installed| {
            installed.machine_id == machine
                && installed.plugin.plugin_id == target.plugin_id
                && installed.plugin.state
                    == crate::machine_protocol::PluginInstallationState::Active
        })
        .map(|installed| installed.plugin)
    else {
        return;
    };
    let Some(sidecars) = state
        .usage
        .exact_sidecars(machine, &plugin, &target.members)
    else {
        return;
    };
    let usage = request(id, &target, &plugin, sidecars);
    let outcome = renew(state, &owner, id, &target, machine, &plugin, usage).await;
    tracing::info!(account_id = id.as_str(), %machine, renewed = outcome.is_ok(),
        reason = outcome.err().unwrap_or("ok"), "session-requested credential renewal settled");
}

/// The collection request a renewal runs (the Machine removes
/// `token_refresh` so the collector takes its refresh-capable path).
pub(super) fn request(
    id: &ProviderAccountId,
    target: &Target,
    plugin: &PluginInventory,
    sidecars: Vec<crate::machine_protocol::portable_usage::PortableSidecar>,
) -> PortableUsageRequest {
    PortableUsageRequest {
        account_id: id.clone(),
        plugin_id: plugin.plugin_id.clone(),
        plugin_version: plugin.plugin_version.clone(),
        generation_digest: plugin.generation_digest.clone(),
        auth_generation: target.auth_generation,
        payload: json!({
            "operation": "collect",
            "provider": target.binding.account,
            "product": target.binding.product_label(),
            "activity": null,
            "token_refresh": false,
        }),
        sidecars,
    }
}

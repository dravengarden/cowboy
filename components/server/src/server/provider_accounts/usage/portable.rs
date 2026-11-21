//! Usage of named portable accounts (Codex, Grok, a DeepSeek API key): the
//! credential lives in the account's Service vault and is replicated to every
//! Machine, so usage runs the Provider's signed collector on one Machine that
//! holds the vault's current generation, in the account's own replica scope.
//!
//! Admission: the owner's available account must resolve to a Ready vault
//! generation before the shared budget is charged. The command is enqueued
//! under the owner's account lock after ownership, availability and the exact
//! vault generations are rechecked, so a disable, revocation or key change
//! either precedes the check or waits for the enqueue; an enqueued command's
//! reply alone decides its outcome. An observation is stored only if the
//! account still resolves to the same scope. The Machine refuses a replica
//! that is not exactly that generation and holds the account's credential
//! guard so no apply or wipe overlaps the collector.
//!
//! Placement is deterministic: among connected Machines with an exact
//! released active generation of the collector Plugin that host policy
//! allows (for the collector and every sidecar Plugin it starts), prefer the
//! Machine running most of the owner's live sessions on the account (whose credential lineage a refresh would follow anyway), then the
//! lowest Machine ID. Like every additional account it never inherits the
//! Service account's configured usage query Machine.
use super::*;
use crate::machine_protocol::portable_usage::PortableUsageRequest;
use crate::store::provider_accounts::portable::Use;
use crate::usage::native::{PortableScope, Scope, Slot};

#[derive(Clone)]
pub(super) struct Target {
    pub(super) scope: PortableScope,
    pub(super) binding: crate::plugin_host::PluginUsageSpec,
    /// Scope member whose signed host owns the collector.
    pub(super) plugin_id: String,
    members: Vec<String>,
    /// The vault's current Ready generation of `plugin_id`.
    pub(super) auth_generation: u64,
}

/// `None` when the account's Provider keeps no named credential in a vault.
pub(super) fn target(
    state: &AppState,
    account: &crate::store::provider_accounts::Account,
    sign_in_generation: Option<u64>,
) -> Option<Result<Target, &'static str>> {
    if !super::super::portable::provider(state, &account.provider_id) {
        return None;
    }
    let members = state
        .provider_catalog
        .portable_account_scope(&account.provider_id);
    let host = match state.usage.portable_usage_host(&members) {
        Some(host) => host,
        None => return Some(Err("account_usage_unsupported")),
    };
    let status = |plugin: &str| {
        super::super::portable::scope(state, &account.id, plugin)
            .and_then(|vault| vault.status(plugin))
    };
    let others = members
        .iter()
        .filter(|plugin| **plugin != host.plugin_id)
        .map(|plugin| status(plugin))
        .collect::<Vec<_>>();
    let collector = status(&host.plugin_id);
    Some(
        ready_target(
            account,
            host.binding,
            host.plugin_id,
            members,
            collector.as_ref(),
        )
        .map(|mut target| {
            target.scope.sign_in_generation = sign_in_generation;
            target
        })
        .and_then(|target| {
            // One committed shared generation across every scope member that
            // holds a key, or no observation: a half-distributed key
            // replacement never mixes two credentials in one balance (the
            // Machine fences each participating replica on it too).
            for status in others.iter().flatten() {
                if ready(Some(status)).is_ok_and(|generation| generation != target.auth_generation)
                {
                    return Err("portable_usage_replica_pending");
                }
            }
            Ok(target)
        }),
    )
}

fn ready(
    status: Option<&crate::provider_service::ProviderAuthenticationStatus>,
) -> Result<u64, &'static str> {
    status
        .filter(|status| {
            status.authentication_state
                == crate::provider_service::ServiceAuthenticationState::Ready
                && status.auth_generation > 0
        })
        .map(|status| status.auth_generation)
        .ok_or("portable_credentials_required")
}

/// Only a Ready vault generation is collected. A sign-in observation binds
/// the account's identity and admission revision, so a refresh rotation keeps
/// it; an entered key has no identity, so its observation binds the key's
/// vault generation and a replaced key never serves its predecessor's data.
fn ready_target(
    account: &crate::store::provider_accounts::Account,
    binding: crate::plugin_host::PluginUsageSpec,
    plugin_id: String,
    members: Vec<String>,
    status: Option<&crate::provider_service::ProviderAuthenticationStatus>,
) -> Result<Target, &'static str> {
    let generation = ready(status)?;
    Ok(Target {
        scope: PortableScope {
            portable_provider: account.provider_id.clone(),
            usage_provider: binding.account.clone(),
            identity_fingerprint: account.identity_fingerprint.clone(),
            admission_revision: account.admission_revision,
            credential_generation: account.identity_fingerprint.is_none().then_some(generation),
            sign_in_generation: None,
        },
        binding,
        plugin_id,
        members,
        auth_generation: generation,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    machine_id: String,
    /// Speaks [`crate::machine_protocol::PORTABLE_USAGE_PROTOCOL_VERSION`]
    /// and holds account-scope subscription replicas.
    current: bool,
    /// Host runtime policy admits the collector Plugin and every sidecar
    /// Plugin it starts there.
    allowed: bool,
    /// The owner's live (worker-running, untrashed) sessions on this account
    /// there.
    sessions: usize,
    /// Every sidecar Plugin has an exact active generation there.
    complete: bool,
    /// The collector there can never redeem a refresh token (its release
    /// declares it, or the Provider CLI does not rotate credentials).
    refresh_safe: bool,
}

/// One placement check a candidate must pass.
type Check = fn(&Candidate) -> bool;

/// Pick the Machine to collect on, or name why none can.
///
/// The checks run in order — host policy, a collector release that can never
/// redeem a refresh token, the Machine protocol, the exact sidecars — and a
/// refusal is the first unmet check of the candidate that met the most of them.
/// A Machine already running a refresh-free release therefore reports why it
/// cannot run yet (for example a Machine update) instead of being hidden behind
/// an older release elsewhere that could never run.
fn choose(candidates: &[Candidate]) -> Result<&str, &'static str> {
    if candidates.is_empty() {
        return Err("portable_usage_machine_unavailable");
    }
    let checks: [(Check, &'static str); 4] = [
        (|c| c.allowed, "runtime_not_allowed"),
        (|c| c.refresh_safe, "portable_usage_refresh_unsupported"),
        (|c| c.current, "machine_usage_update_required"),
        (|c| c.complete, "portable_usage_collector_unavailable"),
    ];
    let mut remaining: Vec<_> = candidates.iter().collect();
    for (check, refusal) in checks {
        remaining.retain(|candidate| check(candidate));
        if remaining.is_empty() {
            return Err(refusal);
        }
    }
    remaining
        .into_iter()
        .min_by(|left, right| {
            right
                .sessions
                .cmp(&left.sessions)
                .then(left.machine_id.cmp(&right.machine_id))
        })
        .map(|candidate| candidate.machine_id.as_str())
        .ok_or("portable_usage_machine_unavailable")
}

/// Whether `code` is a placement refusal: it describes the Machines that could
/// run the collector, not an observation, so it is stale once placement
/// succeeds again.
pub(super) fn placement_refusal(code: &str) -> bool {
    matches!(
        code,
        "portable_usage_machine_unavailable"
            | "runtime_not_allowed"
            | "portable_usage_refresh_unsupported"
            | "machine_usage_update_required"
            | "portable_usage_collector_unavailable"
    )
}

/// One placed collection: the Machine, the exact collector release and the
/// exact sidecar Plugin generations checked for that Machine.
pub(super) type Placement = (
    String,
    crate::machine_protocol::PluginInventory,
    Vec<crate::machine_protocol::portable_usage::PortableSidecar>,
);

pub(super) fn placement(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
) -> Result<Placement, &'static str> {
    let host = state
        .usage
        .portable_usage_host(&target.members)
        .filter(|host| host.plugin_id == target.plugin_id)
        .ok_or("portable_usage_machine_unavailable")?;
    let sessions = state
        .hub
        .session_list_filtered(|session_owner| session_owner == Some(owner));
    // A Provider CLI that rotates credentials may only run a collector that
    // never redeems a refresh token: a poll must not race a session's refresh.
    let rotates = state
        .provider_catalog
        .latest_package(&target.plugin_id)
        .is_none_or(|(_, _, package)| {
            package.manifest.authentication.refresh
                == cowboy_provider_sdk::RefreshOwnership::CompareAndSwap
        });
    // The sidecars the exact collector release on each Machine starts.
    let sidecars: Vec<_> = host
        .candidates
        .iter()
        .map(|(machine, plugin, _)| state.usage.exact_sidecars(machine, plugin, &target.members))
        .collect();
    let candidates: Vec<_> = host
        .candidates
        .iter()
        .zip(&sidecars)
        .map(|((machine, _, refresh_free), sidecars)| Candidate {
            refresh_safe: *refresh_free || !rotates,
            complete: sidecars.is_some(),
            machine_id: machine.clone(),
            current: state.machine_control.machine_supports(
                machine,
                crate::machine_protocol::PORTABLE_USAGE_PROTOCOL_VERSION,
            ) && super::super::portable::machine_ready(state, machine, &target.plugin_id),
            allowed: std::iter::once(target.plugin_id.as_str())
                .chain(
                    sidecars
                        .iter()
                        .flatten()
                        .map(|sidecar| sidecar.plugin_id.as_str()),
                )
                .all(|plugin| state.supervisor.vendor_request_allowed(plugin, machine)),
            sessions: sessions
                .iter()
                .filter(|meta| {
                    meta.provider_account_id == *id
                        && &meta.machine_id == machine
                        && !meta.closing
                        && meta.trashed_at_ms.is_none()
                        && state.supervisor.has_live_worker(&meta.id)
                })
                .count(),
        })
        .collect();
    let machine = choose(&candidates)?;
    let ((machine, plugin, _), sidecars) = host
        .candidates
        .iter()
        .zip(sidecars)
        .find(|((candidate, _, _), _)| candidate == machine)
        .ok_or("portable_usage_machine_unavailable")?;
    let sidecars = sidecars.ok_or("portable_usage_collector_unavailable")?;
    Ok((machine.clone(), plugin.clone(), sidecars))
}

/// One budgeted, coalesced collection; the caller holds `collection`.
pub(super) async fn refresh(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    cache: &Slot,
    collection: tokio::sync::OwnedMutexGuard<()>,
) -> Result<(), &'static str> {
    let scope = Scope::Portable(target.scope.clone());
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    // Permanent ineligibility (no Machine, policy, an unsafe collector such
    // as Grok's) is resolved before the shared Provider window is charged:
    // it never displaces the Service account's queries, and the card keeps
    // its reason.
    let placed = match placement(state, owner, id, target) {
        Ok(placed) => placed,
        Err(code) => {
            let changed = {
                let mut entry = cache.data.lock();
                entry.bind(&scope);
                let changed = entry.error != Some(code);
                entry.finish(&scope, Err(code));
                changed
            };
            // Owner surfaces learn a new reason at once.
            if changed {
                state.hub.broadcast_account_usage_changed(owner);
            }
            return Ok(());
        }
    };
    let permit = {
        let mut entry = cache.data.lock();
        entry.bind(&scope);
        if !entry.due(now_ms()) {
            return Ok(());
        }
        let Ok(permit) = state.usage.limiter.start_bounded(
            &target.binding.account,
            &key,
            now_ms(),
            now_ms().saturating_add(crate::usage::limiter::REMOTE_POLL_START_BOUND_MS),
        ) else {
            return Ok(());
        };
        entry.last_attempt_at_ms = now_ms();
        permit
    };
    state.hub.broadcast_account_usage_changed(owner);
    let result = collect(state, owner, id, target, placed).await;
    let outcome = collection_outcome(result.as_ref().err().copied());
    let succeeded = result.is_ok();
    cache.data.lock().finish(&scope, result);
    if succeeded {
        state.usage.native.persist();
    }
    permit.finish(outcome, now_ms());
    drop(collection);
    state.hub.broadcast_account_usage_changed(owner);
    tracing::debug!(
        account_id = id.as_str(),
        "portable account usage refresh settled"
    );
    Ok(())
}

/// Run `enqueue` under the owner's account lock after rechecking that the
/// account is available, still serves this Provider and its vault holds
/// exactly the target's generations. What `enqueue` returned is kept even if
/// the lock transaction then fails: an enqueued command's reply decides.
pub(super) async fn under_account_lock<T>(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    enqueue: impl FnOnce() -> Result<T, &'static str>,
) -> (Option<T>, &'static str) {
    let Some(store) = state.store.as_ref() else {
        return (None, "unavailable");
    };
    let (mut sent, mut refused) = (None, None);
    let current = || {
        let generation = |plugin: &str| {
            super::super::portable::scope(state, id, plugin)
                .and_then(|vault| vault.status(plugin))
                .and_then(|status| ready(Some(&status)).ok())
        };
        generation(&target.plugin_id) == Some(target.auth_generation)
    };
    let locked = store
        .with_portable_account(
            owner,
            id,
            Use::Observe,
            // The reserved authority, not just the Provider: a new sign-in
            // or key between selection and enqueue refuses the dispatch.
            |account| {
                account.provider_id == target.scope.portable_provider
                    && account.admission_revision == target.scope.admission_revision
                    && account.identity_fingerprint == target.scope.identity_fingerprint
            },
            || {
                if current() {
                    match enqueue() {
                        Ok(pending) => sent = Some(pending),
                        Err(code) => refused = Some(code),
                    }
                } else {
                    refused = Some("account_binding_changed");
                }
                std::future::ready(Ok(()))
            },
        )
        .await;
    let code = refused.unwrap_or(if locked.is_ok() {
        "portable_usage_dispatch_failed"
    } else {
        "account_binding_changed"
    });
    (sent, code)
}

async fn collect(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    placed: Placement,
) -> Result<Value, &'static str> {
    let (machine, plugin, sidecars) = placed;
    // Activity attachments are Provider-wide request telemetry with no
    // account dimension; attaching them would show other accounts' spend.
    let request = PortableUsageRequest {
        account_id: id.clone(),
        plugin_id: plugin.plugin_id.clone(),
        plugin_version: plugin.plugin_version.clone(),
        generation_digest: plugin.generation_digest.clone(),
        auth_generation: target.auth_generation,
        // Usage never redeems a refresh token (the Machine enforces it too).
        payload: json!({
            "operation": "collect",
            "provider": target.binding.account,
            "product": target.binding.product_label(),
            "activity": null,
            "token_refresh": false,
        }),
        sidecars,
    };
    let (pending, code) = under_account_lock(state, owner, id, target, || {
        crate::usage::portable::begin_collect(&state.machine_control, &machine, &plugin, request)
    })
    .await;
    let raw = pending.ok_or(code)?.await.inspect_err(|code| {
        tracing::info!(account_id = id.as_str(), %machine, reason = code, "portable account usage refused");
    })?;
    // A disabled, revoked or re-signed account never receives the result.
    match super::target(state, owner, id).await {
        Ok(Resolved::Portable(current))
            if current.scope == target.scope && current.plugin_id == target.plugin_id => {}
        _ => return Err("account_binding_changed"),
    }
    observation(target, &raw)
}

/// The stored observation: the collector's account, windows and Provider
/// activity, under the Service's own clock and labels.
fn observation(target: &Target, raw: &Value) -> Result<Value, &'static str> {
    let status = raw["status"]
        .as_str()
        .filter(|status| matches!(*status, "available" | "exhausted"))
        .ok_or("portable_usage_collector_failed")?;
    if !raw["rate_limits"].is_object() && !raw["account"].is_object() {
        return Err("portable_usage_collector_failed");
    }
    let mut value = json!({
        "provider": target.binding.account, "status": status,
        "source": target.binding.product_label(), "observed_at_ms": now_ms(),
        "account": raw.get("account"), "rate_limits": raw.get("rate_limits"),
    });
    if let Some(activity) = raw.get("activity").filter(|activity| !activity.is_null()) {
        value["activity"] = activity.clone();
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(machine: &str, current: bool, allowed: bool, sessions: usize) -> Candidate {
        Candidate {
            machine_id: machine.into(),
            current,
            allowed,
            sessions,
            refresh_safe: true,
            complete: true,
        }
    }

    #[test]
    fn placement_prefers_the_owner_sessions_then_the_lowest_id() {
        let mut machines = vec![
            candidate("ovh", true, true, 0),
            candidate("hawk", true, true, 0),
            candidate("falcon", true, true, 0),
        ];
        assert_eq!(choose(&machines), Ok("falcon"));
        machines[0].sessions = 1;
        assert_eq!(choose(&machines), Ok("ovh"));
        machines[1].sessions = 2;
        assert_eq!(choose(&machines), Ok("hawk"));
        // Order of discovery never changes the choice.
        machines.reverse();
        assert_eq!(choose(&machines), Ok("hawk"));
    }

    #[test]
    fn placement_refusals_name_their_cause() {
        assert_eq!(choose(&[]), Err("portable_usage_machine_unavailable"));
        // A collector that could redeem a refresh token never runs.
        let mut rotating = candidate("ovh", true, true, 1);
        rotating.refresh_safe = false;
        assert_eq!(
            choose(&[rotating.clone()]),
            Err("portable_usage_refresh_unsupported")
        );
        assert_eq!(
            choose(&[rotating, candidate("hawk", true, true, 0)]),
            Ok("hawk")
        );
        // A Machine missing a checked sidecar Plugin never runs it.
        let mut partial = candidate("ovh", true, true, 3);
        partial.complete = false;
        assert_eq!(
            choose(&[partial.clone()]),
            Err("portable_usage_collector_unavailable")
        );
        assert_eq!(
            choose(&[partial, candidate("hawk", true, true, 0)]),
            Ok("hawk")
        );
        // Policy excludes a Machine even when the owner's sessions run there.
        assert_eq!(
            choose(&[candidate("ovh", true, false, 5)]),
            Err("runtime_not_allowed")
        );
        assert_eq!(
            choose(&[
                candidate("ovh", true, false, 5),
                candidate("hawk", false, true, 0),
            ]),
            Err("machine_usage_update_required")
        );
        assert_eq!(
            choose(&[
                candidate("ovh", false, true, 5),
                candidate("hawk", true, true, 0),
            ]),
            Ok("hawk")
        );
    }

    #[test]
    fn a_refusal_names_the_most_capable_candidate_not_an_older_release() {
        // Hawk runs the refresh-free release but has not been updated; Falcon
        // is current with an older collector that may renew the sign-in.
        let mut older = candidate("falcon", true, true, 0);
        older.refresh_safe = false;
        let outdated = candidate("hawk", false, true, 0);
        assert_eq!(
            choose(&[older.clone(), outdated.clone()]),
            Err("machine_usage_update_required")
        );
        // Its missing sidecar is named, not the older release elsewhere.
        let mut partial = candidate("hawk", true, true, 0);
        partial.complete = false;
        assert_eq!(
            choose(&[older.clone(), partial]),
            Err("portable_usage_collector_unavailable")
        );
        // Only when no allowed Machine has a refresh-free release is that the
        // cause, whatever else those Machines lack.
        let mut older_outdated = older.clone();
        older_outdated.current = false;
        assert_eq!(
            choose(&[older, older_outdated]),
            Err("portable_usage_refresh_unsupported")
        );
        // Policy still comes first: a refresh-free release where the runtime
        // may not run is no candidate.
        let mut forbidden = candidate("hawk", true, false, 0);
        forbidden.refresh_safe = true;
        assert_eq!(choose(&[forbidden]), Err("runtime_not_allowed"));
        for code in [
            "portable_usage_machine_unavailable",
            "runtime_not_allowed",
            "portable_usage_refresh_unsupported",
            "machine_usage_update_required",
            "portable_usage_collector_unavailable",
        ] {
            assert!(placement_refusal(code), "{code}");
        }
        assert!(!placement_refusal("portable_usage_authentication_required"));
    }

    fn target() -> Target {
        Target {
            scope: PortableScope {
                portable_provider: "codex".into(),
                usage_provider: "openai".into(),
                identity_fingerprint: Some("sha256:pika".into()),
                admission_revision: Some(3),
                credential_generation: None,
                sign_in_generation: None,
            },
            binding: serde_json::from_value(json!({"account":"openai","product":"OpenAI"}))
                .unwrap(),
            plugin_id: "codex".into(),
            members: vec!["codex".into()],
            auth_generation: 9,
        }
    }

    #[test]
    fn observations_keep_collector_data_under_service_labels() {
        let raw = json!({
            "provider": "spoofed", "status": "available", "source": "Codex App Server",
            "observed_at_ms": 1, "account": {"account": {"planType": "pro"}},
            "rate_limits": {"rateLimits": {"primary": {"usedPercent": 12}}},
            "activity": {"days": []},
        });
        let value = observation(&target(), &raw).unwrap();
        assert_eq!(value["provider"], "openai");
        assert_eq!(value["source"], "OpenAI");
        assert!(value["observed_at_ms"].as_i64().unwrap() > 1);
        assert_eq!(value["account"], raw["account"]);
        assert_eq!(value["rate_limits"], raw["rate_limits"]);
        assert_eq!(value["activity"], raw["activity"]);
        // A balance-only report (DeepSeek) is an account without windows.
        let balance = json!({"status":"exhausted","account":{"balanceInfos":[]}});
        let value = observation(&target(), &balance).unwrap();
        assert_eq!(value["status"], "exhausted");
        assert!(value.get("activity").is_none());
        for invalid in [
            json!({"status":"error","rate_limits":{}}),
            json!({"status":"available"}),
            json!(null),
        ] {
            assert_eq!(
                observation(&target(), &invalid),
                Err("portable_usage_collector_failed")
            );
        }
    }

    #[test]
    fn portable_failures_follow_the_shared_budget_rules() {
        use crate::usage::limiter::Outcome;
        assert_eq!(
            collection_outcome(Some("portable_usage_authentication_required")),
            Outcome::Authentication
        );
        assert_eq!(
            collection_outcome(Some("portable_usage_throttled")),
            Outcome::Throttled
        );
        for local in [
            "portable_usage_machine_unavailable",
            "portable_usage_replica_pending",
            "portable_usage_busy",
            "machine_usage_update_required",
            "runtime_not_allowed",
        ] {
            assert_eq!(collection_outcome(Some(local)), Outcome::LocalFailure);
        }
        assert_eq!(
            collection_outcome(Some("portable_usage_collector_failed")),
            Outcome::Failure
        );
        for code in [
            "portable_credentials_required",
            "portable_usage_machine_unavailable",
            "portable_usage_replica_pending",
            "portable_usage_authentication_required",
            "portable_usage_throttled",
            "portable_usage_collector_failed",
        ] {
            assert_ne!(error_message(code), error_message("unknown"), "{code}");
        }
    }

    #[test]
    fn only_a_ready_generation_is_collected_and_only_a_sign_in_survives_rotation() {
        let account: crate::store::provider_accounts::Account = serde_json::from_value(json!({
            "id": "11111111-1111-4111-8111-111111111111", "provider_id": "codex-deepseek",
            "label": "Work", "enabled": true, "admission_revision": 4,
        }))
        .unwrap();
        let binding: crate::plugin_host::PluginUsageSpec =
            serde_json::from_value(json!({"account":"deepseek","product":"DeepSeek"})).unwrap();
        let mut status = crate::provider_service::ProviderAuthenticationStatus {
            provider_id: "claude-deepseek".into(),
            account_id: account.id.clone(),
            recipient_key: None,
            auth_generation: 7,
            authentication_state: crate::provider_service::ServiceAuthenticationState::Ready,
            distribution_state: crate::provider_service::ServiceDistributionState::Current,
            auth_contract_fingerprint: "sha256:x".into(),
            authentication_scope: "deepseek-api-key-v1".into(),
            portable_schema: "deepseek-api-key-v1".into(),
            projection_schema: "claude-deepseek-gateway-v1".into(),
            account_label: None,
            updated_at_ms: 0,
        };
        let build = |status: Option<&crate::provider_service::ProviderAuthenticationStatus>| {
            ready_target(
                &account,
                binding.clone(),
                "claude-deepseek".into(),
                vec!["claude-deepseek".into()],
                status,
            )
        };
        // One API-key account of a shared scope runs the scope's collector.
        let first = build(Some(&status)).unwrap();
        assert_eq!(first.auth_generation, 7);
        assert_eq!(first.plugin_id, "claude-deepseek");
        assert_eq!(first.scope.portable_provider, "codex-deepseek");
        assert_eq!(first.scope.usage_provider, "deepseek");
        // A replaced entered key is a new authority: its predecessor's
        // cached balance or in-flight result is never shown for it.
        status.auth_generation = 8;
        assert_ne!(build(Some(&status)).unwrap().scope, first.scope);
        // A signed-in identity keeps its observation across a refresh.
        let mut signed_in = account.clone();
        signed_in.identity_fingerprint = Some("sha256:pika".into());
        let scope = |status: &crate::provider_service::ProviderAuthenticationStatus| {
            ready_target(
                &signed_in,
                binding.clone(),
                "codex".into(),
                vec![],
                Some(status),
            )
            .unwrap()
            .scope
        };
        let rotated = scope(&status);
        status.auth_generation = 9;
        assert_eq!(scope(&status), rotated);
        assert_eq!(build(None).err(), Some("portable_credentials_required"));
        for state in [
            crate::provider_service::ServiceAuthenticationState::SignedOut,
            crate::provider_service::ServiceAuthenticationState::Expired,
        ] {
            status.authentication_state = state;
            assert_eq!(
                build(Some(&status)).err(),
                Some("portable_credentials_required")
            );
        }
    }
}

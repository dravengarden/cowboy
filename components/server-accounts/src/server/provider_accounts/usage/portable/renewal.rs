//! Idle renewal of a named portable account's sign-in (Grok).
//!
//! Usage never redeems a refresh token, so an account whose sessions all
//! ended keeps an access token that expires a few hours later, and every
//! usage read then reports authentication required although the sign-in is
//! intact. When a read reports that, the account is quiet (below) and the
//! placed Machine's exact collector release declares an idle renewal, the
//! Controller schedules one renewal: the Machine starts the Provider CLI in the
//! account's own session home, where it refreshes exactly as a session's
//! startup would. The new credential reaches the vault only through the
//! Machine's ordinary refresh observation and the Service compare-and-swap,
//! like any session's refresh, and is then distributed; a follow-up usage read
//! uses it.
//!
//! Safety rules, each against one way a refresh token could be presented
//! twice:
//! - **Quiet account.** Every untrashed session of the account is stopped
//!   (exited, crashed or interrupted, not closing, no live worker) on a
//!   connected Machine. A starting, running or closing worker, or any session
//!   on a disconnected Machine (whose detached worker may still run, or whose
//!   rotated credential has not reached the vault yet), defers renewal. The
//!   check repeats under the account lock that worker sends also take.
//! - **Launch hold.** While a renewal is in flight, and after it renewed
//!   until the vault generation advances (bounded by [`SETTLE_MS`]), the
//!   Controller's launch gate holds new workers of the account, so no worker
//!   starts from another generation's home with the token being rotated.
//! - **Affinity.** A renewal that renewed or may have started is recorded
//!   durably with its Machine and vault generation; until the vault advances
//!   past that generation only the same Machine may renew again (its home
//!   holds the rotated token), so another Machine never presents the spent
//!   one.
//! - **Own budget.** Each renewal takes its own paced start from the
//!   Provider's usage budget and rechecks everything at dispatch.
//!
//! A proven pre-start refusal releases the attempt; a renewal that changed
//! nothing or failed after starting waits [`AFTER_FAILED_MS`], leaving "sign
//! in again" for the owner.
use super::*;
use crate::machine_protocol::portable_usage::{
    PORTABLE_RENEWAL_PROTOCOL_VERSION, PortableRenewalRequest,
};

/// Minimum spacing after a renewal that renewed: one sign-in lasts hours.
const AFTER_RENEWED_MS: i64 = 30 * 60_000;
/// Spacing after a renewal that renewed nothing or failed after starting.
const AFTER_FAILED_MS: i64 = 6 * 3_600_000;
/// The Machine never starts a renewal later than this after dispatch.
const DISPATCH_WINDOW_MS: i64 = 30_000;
/// The longest a renewed account's launches wait for the vault to advance.
const SETTLE_MS: i64 = 60_000;
/// Budget waits before a scheduled renewal gives up until the next read.
const ATTEMPTS: usize = 3;
/// The longest one budget wait.
const MAX_WAIT_MS: i64 = 3 * 3_600_000;
/// After the budget allows it, the follow-up usage read waits this much more
/// for the renewed credential's compare-and-swap and distribution.
const FOLLOW_UP_MARGIN_MS: i64 = 15_000;
/// Follow-up reads before giving up until an owner surface asks again.
const FOLLOW_UPS: usize = 4;

/// The durable record of one account's renewals.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Entry {
    /// Earliest next renewal.
    next_allowed_ms: i64,
    /// The Machine and vault generation of a renewal that renewed or may
    /// have started; cleared once the vault advances past the generation.
    affinity: Option<(String, u64)>,
    /// Launches of the account wait until then (in flight or settling).
    hold_until_ms: i64,
}

#[derive(Default)]
struct Ledger {
    path: Option<std::path::PathBuf>,
    entries: std::collections::BTreeMap<String, Entry>,
    loaded: bool,
    /// Accounts with a scheduled renewal task.
    scheduled: std::collections::BTreeSet<String>,
}

static LEDGER: std::sync::LazyLock<parking_lot::Mutex<Ledger>> =
    std::sync::LazyLock::new(Default::default);

fn key(owner: &str, id: &ProviderAccountId) -> String {
    crate::usage::limiter::Limiter::account_key(owner, id.as_str())
}

impl Ledger {
    fn load(&mut self, state: &AppState) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.path = state.usage.state_file("usage-renewals.json");
        self.entries = self
            .path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
    }

    /// Persist before an effect relies on it; a failed write refuses the
    /// renewal (an affinity that cannot be kept must not be created).
    fn save(&self) -> bool {
        let Some(path) = self.path.as_ref() else {
            return true;
        };
        let write = || -> std::io::Result<()> {
            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            let tmp = path.with_extension("json.tmp");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(&serde_json::to_vec(&self.entries)?)?;
            file.sync_all()?;
            std::fs::rename(&tmp, path)
        };
        write()
            .inspect_err(|error| tracing::warn!(%error, "portable renewal ledger not saved"))
            .is_ok()
    }

    fn update(&mut self, key: &str, change: impl FnOnce(&mut Entry)) -> bool {
        change(self.entries.entry(key.to_owned()).or_default());
        self.save()
    }
}

/// The vault's current Ready generation of `plugin` for the account.
fn vault_generation(state: &AppState, id: &ProviderAccountId, plugin: &str) -> Option<u64> {
    super::super::super::portable::scope(state, id, plugin)
        .and_then(|vault| vault.status(plugin))
        .map(|status| status.auth_generation)
}

/// Every untrashed session of the account is stopped on a connected Machine.
fn quiet(state: &AppState, owner: &str, id: &ProviderAccountId) -> bool {
    let connected: std::collections::BTreeSet<_> = state
        .machine_control
        .connected_machine_ids()
        .into_iter()
        .collect();
    state
        .hub
        .session_list_filtered(|session_owner| session_owner == Some(owner))
        .iter()
        .filter(|meta| meta.provider_account_id == *id && meta.trashed_at_ms.is_none())
        .all(|meta| {
            !meta.closing
                && matches!(
                    meta.status,
                    crate::core::Status::Exited
                        | crate::core::Status::Crashed
                        | crate::core::Status::Interrupted
                )
                && connected.contains(&meta.machine_id)
                && !state.supervisor.has_live_worker(&meta.id)
        })
}

/// Why the account cannot be renewed on `machine` now, if it cannot.
fn refusal(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    (machine, installed, _): &Placement,
) -> Option<&'static str> {
    if !state
        .machine_control
        .machine_supports(machine, PORTABLE_RENEWAL_PROTOCOL_VERSION)
        || !state.usage.idle_renewal(installed)
    {
        return Some("portable_renewal_unsupported");
    }
    if !quiet(state, owner, id) {
        return Some("portable_renewal_account_active");
    }
    let mut ledger = LEDGER.lock();
    ledger.load(state);
    match ledger.entries.get(&key(owner, id)) {
        Some(entry) if entry.next_allowed_ms > now_ms() => Some("portable_renewal_spaced"),
        Some(Entry {
            affinity: Some((owner_machine, generation)),
            ..
        }) if *generation >= target.auth_generation && owner_machine != machine => {
            Some("portable_renewal_affinity")
        }
        _ => None,
    }
}

/// Hold a launch of this session's account while its renewal is in flight
/// or settling. Called by the Controller's launch gate.
#[must_use]
pub fn launch_hold(state: &AppState, meta: &crate::core::SessionMeta) -> Option<String> {
    if meta.provider_account_id.is_default() {
        return None;
    }
    let owner = meta.owner_user_id.as_deref()?;
    let mut ledger = LEDGER.lock();
    ledger.load(state);
    let entry = ledger.entries.get(&key(owner, &meta.provider_account_id))?;
    let settled = entry.affinity.as_ref().is_some_and(|(_, generation)| {
        vault_generation(state, &meta.provider_account_id, &meta.provider)
            .is_some_and(|current| current > *generation)
    });
    (entry.hold_until_ms > now_ms() && !settled)
        .then(|| "This account's sign-in is being renewed. Try again in a moment.".to_owned())
}

/// Schedule one renewal of the account after a read reported authentication
/// required. Nothing is scheduled while one is.
pub fn schedule(state: &Arc<AppState>, owner: &str, id: &ProviderAccountId) {
    {
        let mut ledger = LEDGER.lock();
        ledger.load(state);
        if !ledger.scheduled.insert(key(owner, id)) {
            return;
        }
    }
    let (state, owner, id) = (Arc::clone(state), owner.to_owned(), id.clone());
    tokio::spawn(async move {
        let renewed = run(&state, &owner, &id).await;
        LEDGER.lock().scheduled.remove(&key(&owner, &id));
        if renewed {
            follow_up(&state, &owner, &id).await;
        }
    });
}

/// Wait for a budget start, then renew once if the account still qualifies.
async fn run(state: &Arc<AppState>, owner: &str, id: &ProviderAccountId) -> bool {
    let budget_key = format!("{}#renewal", key(owner, id));
    for _ in 0..ATTEMPTS {
        let Ok(Resolved::Portable(target)) = super::super::target(state, owner, id).await else {
            return false;
        };
        let provider = target.binding.account.clone();
        let now = now_ms();
        let wait = state
            .usage
            .limiter
            .view(&provider, &budget_key, now)
            .retry_at_ms(now)
            .saturating_sub(now)
            .clamp(0, MAX_WAIT_MS);
        tokio::time::sleep(std::time::Duration::from_millis(
            u64::try_from(wait).unwrap_or(0),
        ))
        .await;
        // One collection of the account at a time, reads included.
        let cache = state.usage.native.entry(owner, id.as_str());
        let _collection = Arc::clone(&cache.collection).lock_owned().await;
        let Ok(Resolved::Portable(target)) = super::super::target(state, owner, id).await else {
            return false;
        };
        let Ok(placed) = placement(state, owner, id, &target) else {
            return false;
        };
        if let Some(code) = refusal(state, owner, id, &target, &placed) {
            tracing::info!(
                account_id = id.as_str(),
                reason = code,
                "portable account renewal deferred"
            );
            return false;
        }
        let now = now_ms();
        let Ok(permit) = state.usage.limiter.start_bounded(
            &provider,
            &budget_key,
            now,
            now.saturating_add(crate::usage::limiter::REMOTE_POLL_START_BOUND_MS),
        ) else {
            continue;
        };
        let (outcome, renewed) = renew(state, owner, id, &target, &placed).await;
        permit.finish(outcome, now_ms());
        return renewed;
    }
    false
}

/// Dispatch one admitted renewal and record its outcome.
async fn renew(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    (machine, installed, _): &Placement,
) -> (crate::usage::limiter::Outcome, bool) {
    use crate::usage::limiter::Outcome;
    let account = key(owner, id);
    // Durable before dispatch: hold launches and bind the affinity.
    let recorded = LEDGER.lock().update(&account, |entry| {
        entry.affinity = Some((machine.clone(), target.auth_generation));
        entry.hold_until_ms = now_ms().saturating_add(DISPATCH_WINDOW_MS + SETTLE_MS);
        entry.next_allowed_ms = now_ms().saturating_add(AFTER_FAILED_MS);
    });
    if !recorded {
        return (Outcome::Unused, false);
    }
    let request = PortableRenewalRequest {
        account_id: id.clone(),
        plugin_id: installed.plugin_id.clone(),
        plugin_version: installed.plugin_version.clone(),
        generation_digest: installed.generation_digest.clone(),
        auth_generation: target.auth_generation,
        expires_at_ms: now_ms().saturating_add(DISPATCH_WINDOW_MS),
    };
    let (pending, code) = under_account_lock(state, owner, id, target, || {
        // A worker that started since the check renews by itself.
        if !quiet(state, owner, id) {
            return Err("portable_renewal_account_active");
        }
        crate::usage::portable::begin_renewal(&state.machine_control, machine, installed, request)
    })
    .await;
    let result = match pending {
        Some(pending) => pending.await,
        None => Err((false, code)),
    };
    tracing::info!(account_id = id.as_str(), %machine, outcome = ?result, "portable account renewal settled");
    let mut ledger = LEDGER.lock();
    match result {
        Ok("renewed") => {
            ledger.update(&account, |entry| {
                entry.next_allowed_ms = now_ms().saturating_add(AFTER_RENEWED_MS);
                entry.hold_until_ms = now_ms().saturating_add(SETTLE_MS);
            });
            (Outcome::Success, true)
        }
        Ok(_) => {
            // The home did not change: nothing was rotated there.
            ledger.update(&account, |entry| {
                entry.affinity = None;
                entry.hold_until_ms = 0;
            });
            (Outcome::Authentication, false)
        }
        // Nothing started: no affinity, no cooldown; the next read retries.
        Err((false, _)) => {
            ledger.update(&account, |entry| *entry = Entry::default());
            (Outcome::Unused, false)
        }
        // May have rotated the token: keep the affinity and the spacing.
        Err((true, _)) => {
            ledger.update(&account, |entry| entry.hold_until_ms = 0);
            (Outcome::Failure, false)
        }
    }
}

/// Read the renewed account's usage once its budget allows, retrying while
/// another collection moves the deadline, without waiting for an owner
/// surface to ask.
async fn follow_up(state: &Arc<AppState>, owner: &str, id: &ProviderAccountId) {
    for _ in 0..FOLLOW_UPS {
        let Ok(Resolved::Portable(target)) = super::super::target(state, owner, id).await else {
            return;
        };
        let now = now_ms();
        let wait = state
            .usage
            .limiter
            .view(&target.binding.account, &key(owner, id), now)
            .retry_at_ms(now)
            .saturating_sub(now)
            .clamp(0, MAX_WAIT_MS)
            .saturating_add(FOLLOW_UP_MARGIN_MS);
        tokio::time::sleep(std::time::Duration::from_millis(
            u64::try_from(wait).unwrap_or(0),
        ))
        .await;
        let cache = state.usage.native.entry(owner, id.as_str());
        let before = cache.data.lock().last_attempt_at_ms;
        let _ = super::super::refresh_one(state, owner, id).await;
        let entry = cache.data.lock().clone();
        // Done once a read ran with anything but a pending distribution.
        if entry.last_attempt_at_ms != before
            && entry.error != Some("portable_usage_replica_pending")
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ledger_round_trips_its_affinity() {
        let entry = Entry {
            next_allowed_ms: 5,
            affinity: Some(("hawk".into(), 7)),
            hold_until_ms: 9,
        };
        let text = serde_json::to_string(&entry).unwrap();
        assert_eq!(serde_json::from_str::<Entry>(&text).unwrap(), entry);
        assert_eq!(
            serde_json::from_str::<Entry>("{}").unwrap(),
            Entry::default()
        );
    }
}

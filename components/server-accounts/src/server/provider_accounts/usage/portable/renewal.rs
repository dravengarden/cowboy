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
//! - **Quiet account.** Every session of the account (trashed ones included)
//!   is stopped (exited, crashed or interrupted, not closing, no live worker)
//!   on a connected Machine. A starting, running or closing worker, or any
//!   session on a disconnected Machine, defers renewal. The check repeats
//!   under the account lock that worker sends also take.
//! - **Launch hold.** From the durable record until the vault advances
//!   (bounded by the dispatch deadline plus [`SETTLE_MS`], kept when the
//!   outcome is uncertain), the Controller's launch gate holds new workers of
//!   the account as a resource wait.
//! - **Affinity.** A renewal that renewed or may have started is recorded
//!   durably (directory-synced, failing closed when unreadable) with its
//!   Machine and vault generation; until the vault advances past that
//!   generation only the same Machine may renew again, and a later attempt
//!   proven never to have started restores the earlier record.
//! - **Own budget.** Each renewal takes its own paced start from the
//!   Provider's usage budget, with one absolute deadline shared with the
//!   Machine, and rechecks everything at dispatch.
//!
//! Known residual, no larger than a session start's (which performs the same
//! startup refresh without any of these checks): a worker launch already past
//! the gate whose status has not changed yet, a reconnecting Machine whose
//! worker snapshots have not arrived, a session refresh whose observation has
//! not reached the vault, and a Machine that has not yet applied a renewed
//! generation, are not visible to the Controller.
use super::*;
use crate::machine_protocol::portable_usage::{
    PORTABLE_RENEWAL_PROTOCOL_VERSION, PortableRenewalRequest,
};

/// Minimum spacing after a renewal that renewed: one sign-in lasts hours.
const AFTER_RENEWED_MS: i64 = 30 * 60_000;
/// Spacing after a renewal that renewed nothing or failed after starting.
const AFTER_FAILED_MS: i64 = 6 * 3_600_000;
/// The Machine never starts a renewal later than this after its budget start.
const DISPATCH_WINDOW_MS: i64 = 30_000;
/// How long launches wait after the dispatch deadline for the vault to advance.
const SETTLE_MS: i64 = 60_000;
/// Budget waits before a scheduled renewal gives up until the next read.
const ATTEMPTS: usize = 3;
/// The longest one budget wait.
const MAX_WAIT_MS: i64 = 3 * 3_600_000;
/// After the budget allows it, the follow-up usage read waits this much more.
const FOLLOW_UP_MARGIN_MS: i64 = 15_000;
/// Budget waits of the follow-up before an owner surface must ask again.
const FOLLOW_UPS: usize = 4;

/// The durable record of one account's renewals.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Entry {
    /// Earliest next renewal.
    next_allowed_ms: i64,
    /// The Machine and vault generation of a renewal that renewed or may
    /// have started; void once the vault advances past the generation.
    affinity: Option<(String, u64)>,
    /// Launches of the account wait until then (in flight or settling).
    hold_until_ms: i64,
}

#[derive(Default)]
struct Ledger {
    path: Option<std::path::PathBuf>,
    entries: std::collections::BTreeMap<String, Entry>,
    loaded: bool,
    /// The journal exists but cannot be read: no renewal until it can, and
    /// every named launch waits out a renewal it may have hidden.
    unreadable: bool,
    /// When the journal was first read by this Controller.
    loaded_at_ms: i64,
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
        self.loaded_at_ms = now_ms();
        self.path = state.usage.state_file("usage-renewals.json");
        let Some(path) = self.path.as_ref() else {
            return;
        };
        // Fail closed: a journal that exists but cannot be decoded may hold
        // an affinity, so it never reads as empty.
        match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(entries) => self.entries = entries,
                Err(_) => self.unreadable = true,
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => self.unreadable = true,
        }
        if self.unreadable {
            tracing::error!(path = %path.display(), "portable renewal journal unreadable; renewals stay off");
        }
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
            std::fs::rename(&tmp, path)?;
            // The rename is durable only once its directory is.
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            std::fs::File::open(parent)?.sync_all()
        };
        write()
            .inspect_err(|error| tracing::warn!(%error, "portable renewal journal not saved"))
            .is_ok()
    }

    /// Stage the change and publish it only once it is durable. A rename
    /// whose directory sync failed may still surface after a crash; that
    /// errs toward an affinity and hold that were never needed.
    fn update(&mut self, key: &str, change: impl FnOnce(&mut Entry)) -> bool {
        if self.unreadable {
            return false;
        }
        let previous = self.entries.get(key).cloned();
        change(self.entries.entry(key.to_owned()).or_default());
        if self.save() {
            return true;
        }
        match previous {
            Some(previous) => self.entries.insert(key.to_owned(), previous),
            None => self.entries.remove(key),
        };
        false
    }
}

/// The vault's current Ready generation of `plugin` for the account.
fn vault_generation(state: &AppState, id: &ProviderAccountId, plugin: &str) -> Option<u64> {
    super::super::super::portable::scope(state, id, plugin)
        .and_then(|vault| vault.status(plugin))
        .map(|status| status.auth_generation)
}

/// Every session of the account, trashed ones included (trashing does not
/// prove the worker stopped), is stopped on a connected Machine.
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
        .filter(|meta| meta.provider_account_id == *id)
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
    if ledger.unreadable {
        return Some("portable_renewal_journal_unreadable");
    }
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
/// or settling, as a resource wait (a queued prompt resumes through the
/// queue's retry once it clears). Called by the Controller's launch gate.
#[must_use]
pub fn launch_hold(state: &AppState, meta: &crate::core::SessionMeta) -> Option<String> {
    if meta.provider_account_id.is_default() {
        return None;
    }
    let owner = meta.owner_user_id.as_deref()?;
    let mut ledger = LEDGER.lock();
    ledger.load(state);
    let wait = || {
        format!(
            "{}this account's sign-in is being renewed",
            crate::server::WAIT_PREFIX
        )
    };
    // An unreadable journal may hide a renewal dispatched before a restart:
    // hold named launches for one dispatch window and settle period.
    if ledger.unreadable && now_ms() < ledger.loaded_at_ms + DISPATCH_WINDOW_MS + SETTLE_MS {
        return Some(wait());
    }
    let entry = ledger.entries.get(&key(owner, &meta.provider_account_id))?;
    let settled = entry.affinity.as_ref().is_some_and(|(_, generation)| {
        vault_generation(state, &meta.provider_account_id, &meta.provider)
            .is_some_and(|current| current > *generation)
    });
    (entry.hold_until_ms > now_ms() && !settled).then(wait)
}

/// Schedule one renewal of the account after a read of `failed` reported
/// authentication required. Nothing is scheduled while one is, and the
/// renewal only ever acts on that same binding and vault generation.
pub fn schedule(state: &Arc<AppState>, owner: &str, id: &ProviderAccountId, failed: &Target) {
    {
        let mut ledger = LEDGER.lock();
        ledger.load(state);
        if !ledger.scheduled.insert(key(owner, id)) {
            return;
        }
    }
    let (state, owner, id) = (Arc::clone(state), owner.to_owned(), id.clone());
    let failed = failed.clone();
    tokio::spawn(async move {
        let renewed = run(&state, &owner, &id, &failed).await;
        LEDGER.lock().scheduled.remove(&key(&owner, &id));
        if let Some(generation) = renewed {
            follow_up(&state, &owner, &id, generation).await;
        }
    });
}

/// Wait for a budget start, then renew once if the account still qualifies.
/// Returns the renewal's input vault generation when it renewed.
async fn run(
    state: &Arc<AppState>,
    owner: &str,
    id: &ProviderAccountId,
    failed: &Target,
) -> Option<u64> {
    let budget_key = format!("{}#renewal", key(owner, id));
    let provider = failed.binding.account.clone();
    for _ in 0..ATTEMPTS {
        // Its own Provider pacing, and never sooner than the account's
        // collection interval after the read that failed.
        let now = now_ms();
        let limiter = &state.usage.limiter;
        let interval_end = limiter
            .view(&provider, &key(owner, id), now)
            .last_attempt_ms
            .saturating_add(limiter.account_interval_ms());
        let wait = limiter
            .view(&provider, &budget_key, now)
            .retry_at_ms(now)
            .max(interval_end)
            .saturating_sub(now)
            .clamp(0, MAX_WAIT_MS);
        tokio::time::sleep(std::time::Duration::from_millis(
            u64::try_from(wait).unwrap_or(0),
        ))
        .await;
        // One collection of the account at a time, reads included.
        let cache = state.usage.native.entry(owner, id.as_str());
        let _collection = Arc::clone(&cache.collection).lock_owned().await;
        // Only the binding and generation whose read failed: a new sign-in
        // or a session's refresh since then needs no renewal.
        let Ok(Resolved::Portable(target)) = super::super::target(state, owner, id).await else {
            return None;
        };
        if target.scope != failed.scope
            || target.plugin_id != failed.plugin_id
            || target.auth_generation != failed.auth_generation
        {
            return None;
        }
        let Ok(placed) = placement(state, owner, id, &target) else {
            return None;
        };
        if let Some(code) = refusal(state, owner, id, &target, &placed) {
            tracing::info!(
                account_id = id.as_str(),
                reason = code,
                "portable account renewal deferred"
            );
            return None;
        }
        let now = now_ms();
        // One absolute deadline for the budget reservation and the Machine.
        let latest_start = now.saturating_add(
            DISPATCH_WINDOW_MS.min(crate::usage::limiter::REMOTE_POLL_START_BOUND_MS),
        );
        let Ok(permit) =
            state
                .usage
                .limiter
                .start_bounded(&provider, &budget_key, now, latest_start)
        else {
            continue;
        };
        let (outcome, renewed) = renew(state, owner, id, &target, &placed, latest_start).await;
        permit.finish(outcome, now_ms());
        return renewed.then_some(target.auth_generation);
    }
    None
}

/// Dispatch one admitted renewal and record its outcome.
async fn renew(
    state: &AppState,
    owner: &str,
    id: &ProviderAccountId,
    target: &Target,
    (machine, installed, _): &Placement,
    expires_at_ms: i64,
) -> (crate::usage::limiter::Outcome, bool) {
    use crate::usage::limiter::Outcome;
    let account = key(owner, id);
    // Durable before dispatch: hold launches and bind the affinity. The
    // prior record returns if this attempt is proven never to have started.
    let prior = LEDGER.lock().entries.get(&account).cloned();
    let recorded = LEDGER.lock().update(&account, |entry| {
        entry.affinity = Some((machine.clone(), target.auth_generation));
        entry.hold_until_ms = expires_at_ms.saturating_add(SETTLE_MS);
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
        expires_at_ms,
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
        Ok(("renewed", billing)) => {
            ledger.update(&account, |entry| {
                entry.next_allowed_ms = now_ms().saturating_add(AFTER_RENEWED_MS);
                entry.hold_until_ms = now_ms().saturating_add(SETTLE_MS);
            });
            // A throttled billing read after the rotation still paces.
            let outcome = billing.map_or(Outcome::Success, |code| {
                super::super::collection_outcome(Some(code))
            });
            (outcome, true)
        }
        Ok(_) => {
            // No home changed: nothing was rotated by this run.
            ledger.update(&account, |entry| {
                entry.affinity = prior.and_then(|prior| prior.affinity);
                entry.hold_until_ms = 0;
            });
            (Outcome::Authentication, false)
        }
        // Nothing started: restore the prior record (an earlier attempt's
        // affinity stays) and add no cooldown; the next read retries.
        Err((false, _)) => {
            ledger.update(&account, |entry| *entry = prior.unwrap_or_default());
            (Outcome::Unused, false)
        }
        // May have rotated the token and may still run: keep the affinity,
        // the spacing and the bounded launch hold; pace by the closed code.
        Err((true, code)) => (super::super::collection_outcome(Some(code)), false),
    }
}

/// Read the renewed account's usage at a vault generation newer than the
/// renewal's input: wait (bounded) for the compare-and-swap, release the
/// read's authentication backoff (new credentials), then read once the budget
/// allows, counting only reads that actually ran.
async fn follow_up(state: &Arc<AppState>, owner: &str, id: &ProviderAccountId, generation: u64) {
    let deadline = now_ms().saturating_add(SETTLE_MS * 2);
    let target = loop {
        match super::super::target(state, owner, id).await {
            Ok(Resolved::Portable(target)) if target.auth_generation > generation => break target,
            Ok(Resolved::Portable(_)) if now_ms() < deadline => {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            _ => return,
        }
    };
    let read_key = key(owner, id);
    state.usage.limiter.credentials_changed(&read_key);
    let cache = state.usage.native.entry(owner, id.as_str());
    for _ in 0..FOLLOW_UPS {
        let now = now_ms();
        let wait = state
            .usage
            .limiter
            .view(&target.binding.account, &read_key, now)
            .retry_at_ms(now)
            .saturating_sub(now)
            .clamp(0, MAX_WAIT_MS)
            .saturating_add(FOLLOW_UP_MARGIN_MS);
        tokio::time::sleep(std::time::Duration::from_millis(
            u64::try_from(wait).unwrap_or(0),
        ))
        .await;
        let before = cache.data.lock().last_attempt_at_ms;
        let _ = super::super::refresh_one(state, owner, id).await;
        let after = cache.data.lock().clone();
        // Done once a collector ran; a refusal before dispatch retries.
        if after.last_attempt_at_ms != before && !after.error.is_some_and(not_started) {
            return;
        }
    }
}

/// Codes of a read that never started its collector.
fn not_started(code: &str) -> bool {
    placement_refusal(code)
        || matches!(
            code,
            "machine_usage_busy"
                | "machine_connection_changed"
                | "portable_usage_dispatch_failed"
                | "portable_usage_replica_pending"
                | "portable_usage_busy"
                | "account_binding_changed"
        )
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

    #[test]
    fn a_journal_write_that_fails_publishes_nothing() {
        let mut ledger = Ledger {
            path: Some("/nonexistent-cowboy-renewal-dir/usage-renewals.json".into()),
            ..Ledger::default()
        };
        ledger.entries.insert("kept".into(), Entry::default());
        assert!(!ledger.update("kept", |entry| entry.hold_until_ms = 9));
        assert!(!ledger.update("new", |entry| entry.hold_until_ms = 9));
        assert_eq!(ledger.entries.get("kept"), Some(&Entry::default()));
        assert!(!ledger.entries.contains_key("new"));
        let dir = tempfile::tempdir().unwrap();
        ledger.path = Some(dir.path().join("usage-renewals.json"));
        assert!(ledger.update("new", |entry| entry.hold_until_ms = 9));
        let saved: std::collections::BTreeMap<String, Entry> =
            serde_json::from_slice(&std::fs::read(ledger.path.unwrap()).unwrap()).unwrap();
        assert_eq!(saved["new"].hold_until_ms, 9);
    }

    #[test]
    fn only_refusals_before_dispatch_retry_the_follow_up() {
        assert!(not_started("machine_usage_busy"));
        assert!(not_started("portable_usage_machine_unavailable"));
        assert!(!not_started("portable_usage_authentication_required"));
        assert!(!not_started("portable_usage_throttled"));
    }

    #[test]
    fn an_unreadable_journal_refuses_every_update() {
        let mut ledger = Ledger {
            unreadable: true,
            ..Ledger::default()
        };
        assert!(!ledger.update("account", |entry| entry.next_allowed_ms = 1));
        assert!(ledger.entries.is_empty());
    }
}

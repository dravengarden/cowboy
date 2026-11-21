//! Durable refresh-custody state of named portable accounts.
//!
//! A custody-managed account's refresh token rests only in the Service
//! vault; one renewal operation at a time restores it on one Machine (see
//! `docs/portable-refresh-lease.md`, "Grok implementation design"). This
//! module owns the per-account decision state and persists every change
//! before it takes effect. It never holds credentials.
//!
//! Fail closed: a missing file is an empty state, but an unreadable or
//! corrupt one poisons the store. A poisoned store treats every declaring
//! account as managed (no Machine receives a refresh token) and admits no
//! renewal, because a lost record could hide a pending one.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::machine_protocol::refresh_custody::RENEWAL_REUSE_WINDOW_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalState {
    /// Dispatched; its outcome has not been observed.
    Pending,
    /// It provably did not redeem (never ran, or its credential came back
    /// unchanged): the vault's token at its generation is still live.
    Settled,
    /// It redeemed the token of its generation: no renewal may run at that
    /// generation again, whatever this process later believes the vault holds.
    Rotated,
    /// Its reply was lost and could not be recovered: the vault's refresh
    /// token may have been spent. No renewal runs again at this generation.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renewal {
    pub renewal_id: String,
    pub machine_id: String,
    pub vault_generation: u64,
    pub started_at_ms: i64,
    pub state: RenewalState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCustody {
    /// Every Machine receives only access-only copies of this account.
    pub managed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renewal: Option<Renewal>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Persisted {
    schema: u16,
    accounts: BTreeMap<String, AccountCustody>,
}

/// What a renewal request may do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Start a new renewal (already recorded as pending).
    Fresh(Renewal),
    /// Ask the Machine for the retained result of this pending renewal; it
    /// never runs again.
    Resume(Renewal),
    /// No renewal may run; the closed reason.
    Refused(&'static str),
}

struct Inner {
    accounts: BTreeMap<String, AccountCustody>,
    poisoned: bool,
}

#[derive(Clone)]
pub struct Custody {
    inner: Arc<parking_lot::Mutex<Inner>>,
    path: Option<Arc<PathBuf>>,
    /// Renewals this Controller process is awaiting now.
    in_flight: Arc<parking_lot::Mutex<std::collections::BTreeSet<String>>>,
}

/// Held while this process awaits a renewal's reply.
pub struct Flight {
    set: Arc<parking_lot::Mutex<std::collections::BTreeSet<String>>>,
    renewal_id: String,
}

impl Drop for Flight {
    fn drop(&mut self) {
        self.set.lock().remove(&self.renewal_id);
    }
}

/// A renewal of the account is being awaited by this process.
pub const CUSTODY_BUSY: &str = "portable_usage_busy";

impl Default for Custody {
    fn default() -> Self {
        Self {
            inner: Arc::new(parking_lot::Mutex::new(Inner {
                accounts: BTreeMap::new(),
                poisoned: false,
            })),
            path: None,
            in_flight: Arc::default(),
        }
    }
}

/// Reason when no renewal may run.
pub const CUSTODY_UNAVAILABLE: &str = "portable_usage_authentication_required";

impl Custody {
    /// The state restored from `path`. A missing file is empty; any other
    /// read or decode failure poisons the store (see the module docs).
    #[must_use]
    pub fn persistent(path: PathBuf) -> Self {
        let (accounts, poisoned) = match std::fs::read(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), false),
            Err(_) => (BTreeMap::new(), true),
            Ok(bytes) => match serde_json::from_slice::<Persisted>(&bytes) {
                Ok(persisted) if persisted.schema == 1 => (persisted.accounts, false),
                _ => (BTreeMap::new(), true),
            },
        };
        if poisoned {
            tracing::error!(path = %path.display(), "refresh custody state is unreadable; renewals stay closed");
        }
        Self {
            inner: Arc::new(parking_lot::Mutex::new(Inner { accounts, poisoned })),
            path: Some(Arc::new(path)),
            in_flight: Arc::default(),
        }
    }

    fn persist(&self, accounts: &BTreeMap<String, AccountCustody>) -> bool {
        let Some(path) = self.path.as_deref() else {
            return true;
        };
        match save(
            path,
            &Persisted {
                schema: 1,
                accounts: accounts.clone(),
            },
        ) {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(%error, "persisting refresh custody state failed");
                false
            }
        }
    }

    /// Whether `account` must receive only access-only copies. A poisoned
    /// store answers yes for every account of a declaring Provider.
    #[must_use]
    pub fn managed(&self, account: &str) -> bool {
        let inner = self.inner.lock();
        inner.poisoned
            || inner
                .accounts
                .get(account)
                .is_some_and(|custody| custody.managed)
    }

    /// Whether `account` was durably recorded as managed (never inferred
    /// from a poisoned store): such an account must never be sealed with
    /// its refresh token again, even if its Provider stops declaring custody.
    #[must_use]
    pub fn recorded_managed(&self, account: &str) -> bool {
        self.inner
            .lock()
            .accounts
            .get(account)
            .is_some_and(|custody| custody.managed)
    }

    /// Every account recorded as managed.
    #[must_use]
    pub fn managed_accounts(&self) -> Vec<String> {
        self.inner
            .lock()
            .accounts
            .iter()
            .filter(|(_, custody)| custody.managed)
            .map(|(account, _)| account.clone())
            .collect()
    }

    /// Whether a renewal of `account` at `generation` settled within the
    /// reuse window: a session's repeated request does not renew again.
    #[must_use]
    pub fn recently_settled(&self, account: &str, generation: u64, now_ms: i64) -> bool {
        self.inner
            .lock()
            .accounts
            .get(account)
            .and_then(|custody| custody.renewal.as_ref())
            .is_some_and(|renewal| {
                matches!(renewal.state, RenewalState::Settled | RenewalState::Rotated)
                    && renewal.vault_generation == generation
                    && now_ms.saturating_sub(renewal.started_at_ms) < RENEWAL_REUSE_WINDOW_MS
            })
    }

    /// Mark `renewal_id` awaited by this process; `None` if it already is.
    #[must_use]
    pub fn fly(&self, renewal_id: &str) -> Option<Flight> {
        self.in_flight
            .lock()
            .insert(renewal_id.to_owned())
            .then(|| Flight {
                set: Arc::clone(&self.in_flight),
                renewal_id: renewal_id.to_owned(),
            })
    }

    /// Mark `account` managed. Returns whether it was newly enabled and
    /// durably recorded (a failed write leaves it unmanaged).
    pub fn enable(&self, account: &str) -> bool {
        let mut inner = self.inner.lock();
        if inner.poisoned
            || inner
                .accounts
                .get(account)
                .is_some_and(|custody| custody.managed)
        {
            return false;
        }
        let mut next = inner.accounts.clone();
        next.entry(account.to_owned()).or_default().managed = true;
        if !self.persist(&next) {
            return false;
        }
        inner.accounts = next;
        true
    }

    /// Decide and durably record a renewal of `account` at the vault's
    /// `generation` on `machine_id`.
    ///
    /// A pending renewal is never followed by a fresh one at the same
    /// generation: only its retained result can settle it, on the Machine
    /// that ran it. One whose Machine is gone for longer than the reuse
    /// window becomes `Unknown`, which blocks renewals until the vault moves
    /// on (a new sign-in).
    pub fn admit(
        &self,
        account: &str,
        generation: u64,
        machine_id: &str,
        connected: impl Fn(&str) -> bool,
        now_ms: i64,
    ) -> Admission {
        let mut inner = self.inner.lock();
        if inner.poisoned {
            return Admission::Refused(CUSTODY_UNAVAILABLE);
        }
        let mut next = inner.accounts.clone();
        let custody = next.entry(account.to_owned()).or_default();
        if !custody.managed {
            return Admission::Refused(CUSTODY_UNAVAILABLE);
        }
        if let Some(previous) = custody.renewal.as_mut() {
            if previous.state == RenewalState::Pending
                && self.in_flight.lock().contains(&previous.renewal_id)
            {
                return Admission::Refused(CUSTODY_BUSY);
            }
            if previous.vault_generation >= generation {
                match previous.state {
                    RenewalState::Pending if connected(&previous.machine_id) => {
                        return Admission::Resume(previous.clone());
                    }
                    RenewalState::Pending
                        if now_ms.saturating_sub(previous.started_at_ms)
                            > RENEWAL_REUSE_WINDOW_MS =>
                    {
                        previous.state = RenewalState::Unknown;
                        let _ = self.persist(&next);
                        inner.accounts = next;
                        return Admission::Refused(CUSTODY_UNAVAILABLE);
                    }
                    RenewalState::Pending | RenewalState::Unknown => {
                        return Admission::Refused(CUSTODY_UNAVAILABLE);
                    }
                    RenewalState::Rotated => return Admission::Refused(CUSTODY_UNAVAILABLE),
                    RenewalState::Settled if previous.vault_generation > generation => {
                        return Admission::Refused(CUSTODY_UNAVAILABLE);
                    }
                    RenewalState::Settled => {}
                }
            }
        }
        let renewal = Renewal {
            renewal_id: format!("renewal-{}", uuid::Uuid::new_v4()),
            machine_id: machine_id.to_owned(),
            vault_generation: generation,
            started_at_ms: now_ms,
            state: RenewalState::Pending,
        };
        custody.renewal = Some(renewal.clone());
        if !self.persist(&next) {
            return Admission::Refused(CUSTODY_UNAVAILABLE);
        }
        inner.accounts = next;
        Admission::Fresh(renewal)
    }

    /// The recorded renewal of `account`, if `renewal_id` is it and pending.
    #[must_use]
    pub fn pending(&self, account: &str, renewal_id: &str) -> Option<Renewal> {
        self.inner
            .lock()
            .accounts
            .get(account)
            .and_then(|custody| custody.renewal.clone())
            .filter(|renewal| {
                renewal.renewal_id == renewal_id && renewal.state == RenewalState::Pending
            })
    }

    /// Record the outcome of the recorded renewal `renewal_id`. Another ID
    /// (a stale or foreign reply) changes nothing and returns false.
    pub fn settle(&self, account: &str, renewal_id: &str, state: RenewalState) -> bool {
        let mut inner = self.inner.lock();
        let mut next = inner.accounts.clone();
        let Some(renewal) = next
            .get_mut(account)
            .and_then(|custody| custody.renewal.as_mut())
            .filter(|renewal| {
                renewal.renewal_id == renewal_id && renewal.state == RenewalState::Pending
            })
        else {
            return false;
        };
        renewal.state = state;
        if !self.persist(&next) {
            return false;
        }
        inner.accounts = next;
        true
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self, account: &str) -> Option<AccountCustody> {
        self.inner.lock().accounts.get(account).cloned()
    }
}

fn save(path: &Path, persisted: &Persisted) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec(persisted)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = "11111111-1111-4111-8111-111111111111";

    fn managed(dir: &Path) -> Custody {
        let custody = Custody::persistent(dir.join("custody.json"));
        assert!(custody.enable(ACCOUNT));
        custody
    }

    fn fresh(admission: Admission) -> Renewal {
        match admission {
            Admission::Fresh(renewal) => renewal,
            other => panic!("expected a fresh renewal, got {other:?}"),
        }
    }

    #[test]
    fn an_unmanaged_account_never_renews_and_enabling_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let custody = Custody::persistent(dir.path().join("custody.json"));
        assert!(!custody.managed(ACCOUNT));
        assert_eq!(
            custody.admit(ACCOUNT, 3, "hawk", |_| true, 0),
            Admission::Refused(CUSTODY_UNAVAILABLE)
        );
        assert!(custody.enable(ACCOUNT));
        assert!(!custody.enable(ACCOUNT), "already enabled");
        let restored = Custody::persistent(dir.path().join("custody.json"));
        assert!(restored.managed(ACCOUNT));
        assert!(!restored.managed("22222222-2222-4222-8222-222222222222"));
    }

    #[test]
    fn a_pending_renewal_is_never_followed_by_another_at_its_generation() {
        let dir = tempfile::tempdir().unwrap();
        let custody = managed(dir.path());
        let first = fresh(custody.admit(ACCOUNT, 3, "hawk", |_| true, 1_000));
        // Concurrent requests from any Machine only resume the first.
        assert_eq!(
            custody.admit(ACCOUNT, 3, "ovh", |_| true, 1_001),
            Admission::Resume(first.clone())
        );
        // Its Machine disconnected: refused, never re-run, until the window
        // passes and it becomes unknown.
        assert_eq!(
            custody.admit(ACCOUNT, 3, "ovh", |_| false, 2_000),
            Admission::Refused(CUSTODY_UNAVAILABLE)
        );
        assert_eq!(
            custody.admit(
                ACCOUNT,
                3,
                "ovh",
                |_| false,
                1_000 + RENEWAL_REUSE_WINDOW_MS + 1
            ),
            Admission::Refused(CUSTODY_UNAVAILABLE)
        );
        assert_eq!(
            custody.snapshot(ACCOUNT).unwrap().renewal.unwrap().state,
            RenewalState::Unknown
        );
        // Unknown blocks even a connected Machine at the same generation.
        assert_eq!(
            custody.admit(ACCOUNT, 3, "hawk", |_| true, i64::MAX),
            Admission::Refused(CUSTODY_UNAVAILABLE)
        );
        // A new sign-in moved the vault: renewal is possible again.
        let next = fresh(custody.admit(ACCOUNT, 4, "hawk", |_| true, i64::MAX));
        assert_ne!(next.renewal_id, first.renewal_id);
    }

    #[test]
    fn an_awaited_renewal_is_busy_not_resumed() {
        let dir = tempfile::tempdir().unwrap();
        let custody = managed(dir.path());
        let renewal = fresh(custody.admit(ACCOUNT, 3, "hawk", |_| true, 0));
        let flight = custody.fly(&renewal.renewal_id).unwrap();
        assert!(custody.fly(&renewal.renewal_id).is_none());
        assert_eq!(
            custody.admit(ACCOUNT, 3, "hawk", |_| true, 1),
            Admission::Refused(CUSTODY_BUSY)
        );
        drop(flight);
        assert_eq!(
            custody.admit(ACCOUNT, 3, "hawk", |_| true, 1),
            Admission::Resume(renewal)
        );
        assert!(custody.recorded_managed(ACCOUNT));
    }

    #[test]
    fn only_the_recorded_renewal_settles_and_a_restart_keeps_it_pending() {
        let dir = tempfile::tempdir().unwrap();
        let custody = managed(dir.path());
        let renewal = fresh(custody.admit(ACCOUNT, 3, "hawk", |_| true, 0));
        assert!(!custody.settle(ACCOUNT, "renewal-foreign", RenewalState::Settled));
        assert!(custody.pending(ACCOUNT, "renewal-foreign").is_none());
        // A Controller restart restores the pending record exactly.
        let restored = Custody::persistent(dir.path().join("custody.json"));
        assert_eq!(
            restored.pending(ACCOUNT, &renewal.renewal_id),
            Some(renewal.clone())
        );
        assert_eq!(
            restored.admit(ACCOUNT, 3, "hawk", |_| true, 1),
            Admission::Resume(renewal.clone())
        );
        assert!(restored.settle(ACCOUNT, &renewal.renewal_id, RenewalState::Settled));
        assert!(!restored.settle(ACCOUNT, &renewal.renewal_id, RenewalState::Settled));
        // A settled renewal at the same generation permits the next one; an
        // older generation never runs.
        assert_eq!(
            restored.admit(ACCOUNT, 2, "hawk", |_| true, 2),
            Admission::Refused(CUSTODY_UNAVAILABLE)
        );
        assert!(matches!(
            restored.admit(ACCOUNT, 3, "hawk", |_| true, 2),
            Admission::Fresh(_)
        ));
    }

    #[test]
    fn a_corrupt_state_is_managed_everywhere_and_renews_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("custody.json");
        std::fs::write(&path, b"{").unwrap();
        let custody = Custody::persistent(path);
        assert!(custody.managed(ACCOUNT));
        assert!(custody.managed("22222222-2222-4222-8222-222222222222"));
        assert!(!custody.recorded_managed(ACCOUNT));
        assert!(!custody.enable(ACCOUNT));
        assert_eq!(
            custody.admit(ACCOUNT, 3, "hawk", |_| true, 0),
            Admission::Refused(CUSTODY_UNAVAILABLE)
        );
    }

    #[test]
    fn an_unwritable_state_admits_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let custody = managed(dir.path());
        // Replace the file's directory entry with a directory: writes fail.
        let path = dir.path().join("custody.json.tmp");
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            custody.admit(ACCOUNT, 3, "hawk", |_| true, 0),
            Admission::Refused(CUSTODY_UNAVAILABLE)
        );
        assert_eq!(custody.snapshot(ACCOUNT).unwrap().renewal, None);
    }

    /// A fake vendor whose refresh tokens are single-use: presenting a spent
    /// one revokes the family, which the test treats as failure.
    #[derive(Default)]
    struct Vendor {
        current: u64,
        reused: bool,
    }

    impl Vendor {
        fn redeem(&mut self, token: u64) -> Option<u64> {
            if token != self.current {
                self.reused = true;
                return None;
            }
            self.current += 1;
            Some(self.current)
        }
    }

    /// The vault: generation and the refresh token it holds.
    #[derive(Clone, Copy)]
    struct Vault {
        generation: u64,
        token: u64,
    }

    /// The Controller side of `usage::portable::renewal::renew`, against a
    /// Machine that retains replies until it restarts.
    #[allow(clippy::too_many_arguments)]
    fn renew(
        custody: &Custody,
        vault: &parking_lot::Mutex<Vault>,
        vendor: &parking_lot::Mutex<Vendor>,
        retained: &parking_lot::Mutex<BTreeMap<String, u64>>,
        machine: &str,
        connected: bool,
        lose_reply: bool,
        now_ms: i64,
    ) {
        let current = *vault.lock();
        let (renewal, resume) =
            match custody.admit(ACCOUNT, current.generation, machine, |_| connected, now_ms) {
                Admission::Fresh(renewal) => (renewal, false),
                Admission::Resume(renewal) if renewal.machine_id == machine => (renewal, true),
                _ => return,
            };
        let Some(_flight) = custody.fly(&renewal.renewal_id) else {
            return;
        };
        if !connected {
            if !resume {
                custody.settle(ACCOUNT, &renewal.renewal_id, RenewalState::Settled);
            }
            return;
        }
        // The Machine: a known ID returns its retained result, never runs.
        let known = retained.lock().get(&renewal.renewal_id).copied();
        let rotated = match (known, resume) {
            (Some(token), _) => Some(token),
            (None, true) => {
                custody.settle(ACCOUNT, &renewal.renewal_id, RenewalState::Unknown);
                return;
            }
            (None, false) => {
                let token = vendor.lock().redeem(current.token);
                if let Some(token) = token {
                    retained.lock().insert(renewal.renewal_id.clone(), token);
                }
                token
            }
        };
        if lose_reply {
            return;
        }
        if let Some(token) = rotated {
            let mut vault = vault.lock();
            // The generation compare-and-swap.
            if vault.generation == renewal.vault_generation {
                *vault = Vault {
                    generation: vault.generation + 1,
                    token,
                };
            }
        }
        let outcome = if rotated.is_some() {
            RenewalState::Rotated
        } else {
            RenewalState::Settled
        };
        custody.settle(ACCOUNT, &renewal.renewal_id, outcome);
    }

    /// Racing renewals from several Machines, lost replies, Machine and
    /// Controller restarts and disconnects never present a spent token.
    #[test]
    fn racing_renewals_never_reuse_a_refresh_token() {
        for seed in 0..200_u64 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("custody.json");
            let mut custody = Custody::persistent(path.clone());
            assert!(custody.enable(ACCOUNT));
            let vault = parking_lot::Mutex::new(Vault {
                generation: 1,
                token: 0,
            });
            let vendor = parking_lot::Mutex::new(Vendor::default());
            let retained = parking_lot::Mutex::new(BTreeMap::new());
            let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let mut next = |bound: u64| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) % bound
            };
            let mut now = 0_i64;
            for _ in 0..60 {
                now += i64::try_from(next(120_000)).unwrap();
                match next(10) {
                    0 => custody = Custody::persistent(path.clone()),
                    1 => retained.lock().clear(),
                    _ => {
                        // Three Machines race the same attempt.
                        std::thread::scope(|scope| {
                            for machine in ["falcon", "hawk", "ovh"] {
                                let connected = next(5) != 0;
                                let lose = next(4) == 0;
                                let (custody, vault, vendor, retained) =
                                    (&custody, &vault, &vendor, &retained);
                                scope.spawn(move || {
                                    renew(
                                        custody, vault, vendor, retained, machine, connected, lose,
                                        now,
                                    );
                                });
                            }
                        });
                    }
                }
                assert!(
                    !vendor.lock().reused,
                    "seed {seed}: a refresh token was presented twice"
                );
            }
            // The vault holds the vendor's live token unless a lost reply
            // left the account blocked (it then needs a new sign-in).
            let blocked = custody
                .snapshot(ACCOUNT)
                .and_then(|custody| custody.renewal)
                .is_some_and(|renewal| {
                    matches!(renewal.state, RenewalState::Pending | RenewalState::Unknown)
                });
            assert!(
                blocked || vault.lock().token == vendor.lock().current,
                "seed {seed}"
            );
        }
    }

    /// Sessions on three Machines run across many access-token lifetimes.
    /// The proactive scheduler (every 30 s, renewing within the lead before
    /// expiry) races usage-read renewals from other Machines. With replies
    /// delivered, no refresh token is ever presented twice and no session
    /// ever sends an expired access token: each new token reaches every
    /// Machine's access-only copy before the old one expires, which a running
    /// Grok CLI adopts at expiry.
    #[test]
    fn sessions_on_several_machines_never_outlive_their_access_token() {
        use crate::machine_protocol::refresh_custody::RENEWAL_LEAD_MS;
        const LIFETIME_MS: i64 = 3_600_000;
        for seed in 0..50_u64 {
            let dir = tempfile::tempdir().unwrap();
            let custody = Custody::persistent(dir.path().join("custody.json"));
            assert!(custody.enable(ACCOUNT));
            let vault = parking_lot::Mutex::new(Vault {
                generation: 1,
                token: 0,
            });
            let vendor = parking_lot::Mutex::new(Vendor::default());
            let retained = parking_lot::Mutex::new(BTreeMap::new());
            // Expiry of each generation's access token, by generation.
            let expiry = parking_lot::Mutex::new(BTreeMap::from([(1_u64, LIFETIME_MS)]));
            let mut state = seed.wrapping_add(17);
            let mut next = |bound: u64| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) % bound
            };
            let machines = ["hawk", "ovh", "falcon"];
            let mut now = 0_i64;
            while now < 10 * LIFETIME_MS {
                now += 30_000;
                let current = *vault.lock();
                let expires = expiry.lock()[&current.generation];
                // Every session's copy is the vault's current generation (it
                // is distributed with the reply); a session sending a token
                // after its expiry is the regression this test forbids.
                assert!(
                    now < expires,
                    "seed {seed}: a session outlived its access token at {now}"
                );
                let due = expires - now <= RENEWAL_LEAD_MS;
                let usage_read = next(6) == 0;
                std::thread::scope(|scope| {
                    for (index, machine) in machines.iter().enumerate() {
                        let scheduler = due && index == 0;
                        let usage = usage_read && expires - now <= 60_000 && index == 1;
                        if !(scheduler || usage || next(20) == 0) {
                            continue;
                        }
                        let (custody, vault, vendor, retained, expiry) =
                            (&custody, &vault, &vendor, &retained, &expiry);
                        scope.spawn(move || {
                            let before = vault.lock().generation;
                            renew(custody, vault, vendor, retained, machine, true, false, now);
                            let after = vault.lock().generation;
                            if after != before {
                                expiry.lock().insert(after, now + LIFETIME_MS);
                            }
                        });
                    }
                });
                assert!(
                    !vendor.lock().reused,
                    "seed {seed}: a refresh token was presented twice"
                );
            }
        }
    }
}

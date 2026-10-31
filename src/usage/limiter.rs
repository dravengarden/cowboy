//! One durable admission budget for every Service-initiated quota collection.
//! A collector invocation is the unit: native CLIs own their internal HTTP.
//! Passive usage events and reads of cached observations never spend this budget.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::{Config, Handle, Scope, schema};

#[derive(Clone)]
pub struct Limiter {
    inner: Arc<parking_lot::Mutex<State>>,
    config: Handle,
    path: Option<PathBuf>,
}

struct State {
    durable: Durable,
    active: BTreeSet<String>,
    failed_closed: bool,
    /// Accounts whose latest attempt failed authentication. In memory only:
    /// after a restart the durable backoff simply runs its course.
    authentication_failed: BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Durable {
    schema: u32,
    /// A missing journal may be an upgrade from a collector without this ledger.
    /// Wait out one window rather than assuming the previous process was idle.
    startup_after_ms: i64,
    providers: BTreeMap<String, Vec<i64>>,
    accounts: BTreeMap<String, Account>,
    #[serde(default)]
    provider_backoff: BTreeMap<String, Account>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Account {
    last_attempt_ms: i64,
    next_allowed_ms: i64,
    failures: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub last_attempt_ms: i64,
    pub next_allowed_ms: i64,
}

impl Budget {
    /// Cache scheduling is only a hint. Never persist the fail-closed sentinel
    /// there: a repaired ledger must be reachable again without editing caches.
    pub fn retry_at_ms(self, now: i64) -> i64 {
        if self.next_allowed_ms == i64::MAX {
            now.saturating_add(60_000)
        } else {
            self.next_allowed_ms
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// A reserved start that sent nothing; keep its charge, change no backoff.
    Unused,
    Failure,
    Authentication,
    LocalFailure,
}

pub struct Permit {
    limiter: Limiter,
    provider: String,
    account: String,
    finished: bool,
}

impl Limiter {
    pub fn new(path: Option<PathBuf>) -> Self {
        let now = super::now_ms();
        let mut failed_closed = false;
        let durable = match path.as_ref().map(std::fs::read) {
            Some(Ok(bytes)) => serde_json::from_slice::<Durable>(&bytes)
                .ok()
                .filter(|value| {
                    value.schema == 1
                        && value.startup_after_ms >= 0
                        && value.providers.values().all(|times| {
                            times.iter().all(|time| *time >= 0)
                                && times.windows(2).all(|pair| pair[0] <= pair[1])
                        })
                        && value
                            .accounts
                            .values()
                            .chain(value.provider_backoff.values())
                            .all(|entry| {
                                entry.last_attempt_ms >= 0
                                    && entry.next_allowed_ms >= entry.last_attempt_ms
                            })
                })
                .unwrap_or_else(|| {
                    failed_closed = true;
                    tracing::error!(
                        "invalid usage admission ledger; new quota queries are stopped"
                    );
                    empty(now)
                }),
            Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => {
                failed_closed = true;
                tracing::error!(%error, "unreadable usage admission ledger; new quota queries are stopped");
                empty(now)
            }
            _ => empty(if path.is_some() { now + 300_000 } else { 0 }),
        };
        let this = Self {
            inner: Arc::new(parking_lot::Mutex::new(State {
                durable,
                active: BTreeSet::new(),
                failed_closed,
                authentication_failed: BTreeSet::new(),
            })),
            config: Handle::new(Config::defaults(Scope::Service)),
            path,
        };
        // Persist the initial quiet window, so repeated restarts cannot skip it.
        let mut state = this.inner.lock();
        if !state.failed_closed && !this.save(&state.durable) {
            state.failed_closed = true;
        }
        drop(state);
        this
    }

    #[must_use]
    pub fn with_config(mut self, config: Handle) -> Self {
        self.config = config;
        self
    }

    pub fn account_key(owner: &str, account: &str) -> String {
        serde_json::to_string(&[owner, account]).expect("string tuple")
    }

    pub fn view(&self, provider: &str, account: &str, now: i64) -> Budget {
        self.budget(&self.inner.lock(), provider, account, now, true)
    }

    fn budget(
        &self,
        state: &State,
        provider: &str,
        account: &str,
        now: i64,
        polling: bool,
    ) -> Budget {
        let config = self.config.current();
        let window = super::duration_ms(config.get(&schema::USAGE_PROVIDER_WINDOW));
        let count = config.get(&schema::USAGE_REQUESTS_PER_WINDOW);
        let spacing = (window + count - 1) / count;
        let slot = state.durable.accounts.get(account);
        let mut next = slot
            .map_or(0, |slot| {
                if polling {
                    slot.next_allowed_ms
                        .max(slot.last_attempt_ms.saturating_add(super::duration_ms(
                            config.get(&schema::USAGE_ACCOUNT_INTERVAL),
                        )))
                } else if slot.failures > 0 {
                    slot.next_allowed_ms
                } else {
                    0
                }
            })
            .max(state.durable.startup_after_ms);
        if let Some(backoff) = state.durable.provider_backoff.get(provider) {
            next = next.max(backoff.next_allowed_ms);
        }
        if let Some(attempts) = state.durable.providers.get(provider) {
            if let Some(last) = attempts.last() {
                next = next.max(last.saturating_add(spacing));
            }
            let recent: Vec<_> = attempts
                .iter()
                .filter(|time| time.saturating_add(window) > now)
                .collect();
            if recent.len() >= usize::try_from(count).unwrap_or(1) {
                let index = recent.len() - usize::try_from(count).unwrap_or(1);
                next = next.max(recent[index].saturating_add(window));
            }
        }
        if state.failed_closed {
            next = i64::MAX;
        } else if state.active.contains(provider) {
            next = next.max(now.saturating_add(1_000));
        }
        Budget {
            last_attempt_ms: slot.map_or(0, |slot| slot.last_attempt_ms),
            next_allowed_ms: next,
        }
    }

    /// Charges failed and cancelled attempts too. Persist before dispatch.
    pub fn start(&self, provider: &str, account: &str, now: i64) -> Result<Permit, Budget> {
        self.start_with(provider, account, now, true)
    }

    /// A confirmed usage action (a reset) is a user write, not a poll. It
    /// shares the Provider window, spacing, failure backoff and single active
    /// collector, but not the account polling interval a preceding refresh
    /// just started. It still delays that account's next poll.
    pub fn start_action(&self, provider: &str, account: &str, now: i64) -> Result<Permit, Budget> {
        self.start_with(provider, account, now, false)
    }

    fn start_with(
        &self,
        provider: &str,
        account: &str,
        now: i64,
        polling: bool,
    ) -> Result<Permit, Budget> {
        let mut state = self.inner.lock();
        let budget = self.budget(&state, provider, account, now, polling);
        if now < budget.next_allowed_ms {
            return Err(budget);
        }
        let mut next = state.durable.clone();
        let starts = next.providers.entry(provider.into()).or_default();
        // Retain the maximum configurable window, including across live edits.
        starts.retain(|time| time.saturating_add(3_600_000) > now);
        starts.push(now);
        let slot = next.accounts.entry(account.into()).or_default();
        slot.last_attempt_ms = now;
        slot.next_allowed_ms = slot
            .next_allowed_ms
            .max(now.saturating_add(super::duration_ms(
                self.config.get(&schema::USAGE_ACCOUNT_INTERVAL),
            )));
        if !self.save(&next) {
            state.failed_closed = true;
            return Err(Budget {
                next_allowed_ms: i64::MAX,
                ..budget
            });
        }
        state.durable = next;
        state.active.insert(provider.into());
        Ok(Permit {
            limiter: self.clone(),
            provider: provider.into(),
            account: account.into(),
            finished: false,
        })
    }

    fn finish(&self, provider: &str, account: &str, outcome: Outcome, now: i64) {
        let mut state = self.inner.lock();
        let interval = super::duration_ms(self.config.get(&schema::USAGE_ACCOUNT_INTERVAL));
        let max_backoff =
            super::duration_ms(self.config.get(&schema::USAGE_FAILURE_BACKOFF_MAX)).max(interval);
        if outcome == Outcome::Authentication {
            state.authentication_failed.insert(account.into());
        } else if outcome != Outcome::Unused {
            state.authentication_failed.remove(account);
        }
        let slot = state.durable.accounts.entry(account.into()).or_default();
        match outcome {
            Outcome::Success => slot.failures = 0,
            Outcome::Unused => {}
            Outcome::Failure | Outcome::Authentication | Outcome::LocalFailure => {
                slot.failures = slot.failures.saturating_add(1);
                let backoff = if matches!(outcome, Outcome::Authentication) {
                    max_backoff
                } else {
                    interval
                        .saturating_mul(1_i64 << slot.failures.min(10))
                        .min(max_backoff)
                };
                slot.next_allowed_ms = slot.next_allowed_ms.max(now.saturating_add(backoff));
            }
        }
        // A Provider outage/rate-limit must not immediately switch accounts.
        // Authentication failures are account-specific.
        let backoff = state
            .durable
            .provider_backoff
            .entry(provider.into())
            .or_default();
        match outcome {
            Outcome::Success => backoff.failures = 0,
            Outcome::Failure => {
                backoff.failures = backoff.failures.saturating_add(1);
                let window = super::duration_ms(self.config.get(&schema::USAGE_PROVIDER_WINDOW));
                backoff.next_allowed_ms = now.saturating_add(
                    window
                        .saturating_mul(1_i64 << backoff.failures.min(10))
                        .min(max_backoff),
                );
            }
            Outcome::Authentication | Outcome::LocalFailure | Outcome::Unused => {}
        }
        if !self.save(&state.durable) {
            state.failed_closed = true;
        }
        state.active.remove(provider);
    }

    /// New credentials supersede an authentication failure: drop that
    /// account's failure backoff so its next poll may run once the ordinary
    /// account interval, Provider window and spacing allow. Each release needs
    /// a fresh failure, so repeated credential changes cannot bypass the budget.
    pub fn credentials_changed(&self, account: &str) -> bool {
        let mut state = self.inner.lock();
        if state.failed_closed || !state.authentication_failed.remove(account) {
            return false;
        }
        let interval = super::duration_ms(self.config.get(&schema::USAGE_ACCOUNT_INTERVAL));
        let mut next = state.durable.clone();
        let Some(slot) = next.accounts.get_mut(account) else {
            return false;
        };
        slot.failures = 0;
        slot.next_allowed_ms = slot.last_attempt_ms.saturating_add(interval);
        if !self.save(&next) {
            state.failed_closed = true;
            return false;
        }
        state.durable = next;
        true
    }

    fn save(&self, value: &Durable) -> bool {
        let Some(path) = &self.path else { return true };
        let result = (|| -> std::io::Result<()> {
            use std::os::unix::fs::OpenOptionsExt as _;
            let tmp = path.with_extension("json.tmp");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(&serde_json::to_vec(value)?)?;
            file.sync_all()?;
            std::fs::rename(tmp, path)?;
            std::fs::File::open(path.parent().unwrap_or(std::path::Path::new(".")))?.sync_all()
        })();
        if let Err(error) = result {
            tracing::error!(%error, "usage admission ledger failed; new quota queries are stopped");
            false
        } else {
            true
        }
    }
}

fn empty(startup_after_ms: i64) -> Durable {
    Durable {
        schema: 1,
        startup_after_ms,
        providers: BTreeMap::new(),
        accounts: BTreeMap::new(),
        provider_backoff: BTreeMap::new(),
    }
}

impl Permit {
    pub fn finish(mut self, outcome: Outcome, now: i64) {
        self.limiter
            .finish(&self.provider, &self.account, outcome, now);
        self.finished = true;
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if !self.finished {
            self.limiter.finish(
                &self.provider,
                &self.account,
                Outcome::Failure,
                super::now_ms(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_plugin_configuration_requires_exact_unique_identifiers() {
        for text in [
            "['gemini', 'gemini']",
            "['*']",
            "['Gemini']",
            "['a/b']",
            "['']",
        ] {
            let report = crate::config::check_text(
                Scope::Service,
                &format!("schema = 1\n[plugins]\ndisabled = {text}\n"),
                None,
            );
            assert!(!report.is_valid(), "{text}");
        }
        // The retired Gemini identity remains a valid disabled entry, so an
        // existing Service configuration keeps loading after its removal.
        let report = crate::config::check_text(
            Scope::Service,
            "schema = 1\n[plugins]\ndisabled = ['gemini']\n",
            None,
        );
        assert!(report.is_valid());
    }

    #[test]
    fn rolling_budget_staggers_accounts_and_provider_budgets_are_independent() {
        let limiter = Limiter::new(None);
        let first = limiter
            .start("anthropic", "lynx", 1000)
            .unwrap_or_else(|_| panic!("first"));
        assert!(limiter.start("anthropic", "otter", 61_000).is_err()); // still active
        limiter
            .start("openai", "pika", 1000)
            .unwrap_or_else(|_| panic!("independent"))
            .finish(Outcome::Success, 1000);
        first.finish(Outcome::Success, 1000);
        assert!(limiter.start("anthropic", "otter", 60_999).is_err());
        limiter
            .start("anthropic", "otter", 61_000)
            .unwrap_or_else(|_| panic!("stagger"))
            .finish(Outcome::Success, 61_000);
        assert!(limiter.start("anthropic", "third", 120_999).is_err());
        limiter
            .start("anthropic", "third", 121_000)
            .unwrap_or_else(|_| panic!("window"))
            .finish(Outcome::Success, 121_000);
        assert!(limiter.start("anthropic", "lynx", 300_999).is_err());
        limiter
            .start("anthropic", "lynx", 301_000)
            .unwrap_or_else(|_| panic!("account"))
            .finish(Outcome::Success, 301_000);
    }

    #[test]
    fn failure_backoff_applies_across_accounts_and_does_not_exceed_configured_max() {
        let limiter = Limiter::new(None);
        limiter
            .start("anthropic", "lynx", 1000)
            .unwrap_or_else(|_| panic!("first"))
            .finish(Outcome::Failure, 2000);
        assert_eq!(
            limiter.view("anthropic", "lynx", 2000).next_allowed_ms,
            602_000
        );
        assert_eq!(
            limiter.view("anthropic", "otter", 2000).next_allowed_ms,
            242_000
        );
        for index in 0..12 {
            let now = limiter.view("anthropic", "lynx", 1000).next_allowed_ms;
            limiter
                .start("anthropic", "lynx", now)
                .unwrap_or_else(|_| panic!("{index}"))
                .finish(Outcome::Failure, now);
            assert!(limiter.view("anthropic", "lynx", now).next_allowed_ms <= now + 3_600_000);
        }
    }

    #[test]
    fn new_credentials_release_only_an_authentication_backoff_once() {
        let limiter = Limiter::new(None);
        assert!(!limiter.credentials_changed("lynx"));
        limiter
            .start("anthropic", "lynx", 1000)
            .unwrap_or_else(|_| panic!("start"))
            .finish(Outcome::Authentication, 2000);
        assert_eq!(
            limiter.view("anthropic", "lynx", 2000).next_allowed_ms,
            3_602_000
        );
        assert!(limiter.credentials_changed("lynx"));
        // The account interval and Provider spacing still apply.
        assert_eq!(
            limiter.view("anthropic", "lynx", 2000).next_allowed_ms,
            301_000
        );
        assert!(!limiter.credentials_changed("lynx"));
        limiter
            .start("anthropic", "lynx", 301_000)
            .unwrap_or_else(|_| panic!("restart"))
            .finish(Outcome::Failure, 302_000);
        assert!(!limiter.credentials_changed("lynx"));
    }

    #[test]
    fn local_failure_backs_off_only_its_account() {
        let limiter = Limiter::new(None);
        limiter
            .start("anthropic", "offline", 1000)
            .unwrap_or_else(|_| panic!("start"))
            .finish(Outcome::LocalFailure, 2000);
        assert_eq!(
            limiter.view("anthropic", "offline", 2000).next_allowed_ms,
            602_000
        );
        limiter
            .start("anthropic", "online", 61_000)
            .unwrap_or_else(|_| panic!("other account"))
            .finish(Outcome::Success, 61_000);
    }

    #[test]
    fn restart_and_lost_dispatch_keep_reservations_and_corrupt_journal_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("limits.json");
        let limiter = Limiter::new(Some(path.clone()));
        let now = limiter.view("anthropic", "otter", 0).next_allowed_ms;
        let permit = limiter
            .start("anthropic", "otter", now)
            .unwrap_or_else(|_| panic!("start"));
        let restarted = Limiter::new(Some(path.clone()));
        assert!(restarted.start("anthropic", "otter", now + 1).is_err());
        assert!(restarted.start("anthropic", "lynx", now + 59_999).is_err());
        permit.finish(Outcome::Authentication, now);
        assert_eq!(
            Limiter::new(Some(path.clone()))
                .view("anthropic", "otter", now)
                .next_allowed_ms,
            now + 3_600_000
        );
        std::fs::write(&path, b"broken").unwrap();
        let broken = Limiter::new(Some(path.clone()));
        assert_eq!(
            broken
                .start("openai", "pika", now)
                .err()
                .unwrap()
                .next_allowed_ms,
            i64::MAX
        );
        assert_eq!(std::fs::read(path).unwrap(), b"broken");
    }

    #[test]
    fn write_failure_never_dispatches_and_cancellation_keeps_budget() {
        let limiter = Limiter::new(None);
        let now = super::super::now_ms();
        drop(
            limiter
                .start("anthropic", "otter", now)
                .unwrap_or_else(|_| panic!("start")),
        );
        assert!(limiter.view("anthropic", "otter", now).next_allowed_ms >= now + 600_000);
        let root = tempfile::tempdir().unwrap();
        let broken = Limiter::new(Some(root.path().join("missing").join("limits.json")));
        assert!(broken.start("anthropic", "otter", i64::MAX - 1).is_err());
    }

    #[test]
    fn concurrent_requests_charge_once_and_live_config_cannot_reset_existing_history() {
        let limiter = Limiter::new(None);
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let results = std::thread::scope(|scope| {
            (0..16)
                .map(|index| {
                    let limiter = limiter.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        limiter.start("anthropic", &format!("account-{index}"), 1000)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        for permit in results.into_iter().flatten() {
            permit.finish(Outcome::Success, 1000);
        }
        let report = crate::config::check_text(
            Scope::Service,
            "schema = 1\n[usage]\nrequests_per_window = 1\nprovider_window = '4m'\n",
            None,
        );
        assert!(report.is_valid(), "{}", report.render_text());
        limiter.config.reload(&report).unwrap();
        assert_eq!(
            limiter.view("anthropic", "new", 1000).next_allowed_ms,
            241_000
        );
    }

    #[test]
    fn confirmed_actions_share_provider_pacing_but_not_the_polling_interval() {
        let limiter = Limiter::new(None);
        limiter
            .start("anthropic", "lynx", 1000)
            .unwrap()
            .finish(Outcome::Success, 2000);
        // One minute spacing applies to every Provider start, including a reset.
        assert!(limiter.start_action("anthropic", "lynx", 30_000).is_err());
        let permit = limiter.start_action("anthropic", "lynx", 61_000).unwrap();
        // Still a single active request per Provider.
        assert!(limiter.start_action("anthropic", "mink", 122_000).is_err());
        permit.finish(Outcome::Success, 62_000);
        // The action delays the account's next poll; it does not reset it.
        assert!(limiter.view("anthropic", "lynx", 62_000).next_allowed_ms >= 361_000);
        // Two starts per window still hold across polls and actions.
        assert!(limiter.start_action("anthropic", "mink", 100_000).is_err());
        assert!(limiter.start_action("anthropic", "mink", 121_000).is_ok());
    }
}

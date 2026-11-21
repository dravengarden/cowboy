//! Admission for contained read programs.
//!
//! Every program is a Bubblewrap + Bun process (512 MiB data, 4 s CPU), so the
//! Controller bounds how many run at once. Bursts are common (several sessions
//! paging history together), so a request waits briefly for a slot instead of
//! failing on first contention, and one session cannot hold every slot. A
//! refusal names its scope and a retry delay, and is logged with the current
//! holders so a saturated Controller can be diagnosed afterwards.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// Concurrent programs across the Controller.
pub(super) const GLOBAL_SLOTS: usize = 6;
/// Concurrent programs one session may hold.
pub(super) const SESSION_SLOTS: usize = 2;
/// Longest wait for a slot; never more than half the program's own deadline.
const MAX_WAIT: Duration = Duration::from_secs(5);
/// Suggested delay after a refusal.
const RETRY_AFTER_MS: u64 = 2000;

pub(super) static ADMISSION: LazyLock<Admission> =
    LazyLock::new(|| Admission::new(GLOBAL_SLOTS, SESSION_SLOTS));

pub(super) struct Admission {
    global: Arc<Semaphore>,
    per_session: usize,
    holders: Mutex<HashMap<String, usize>>,
    released: Notify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Controller,
    Session,
}

impl Scope {
    fn code(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Session => "session",
        }
    }
}

/// A held slot; dropping it releases both the session and global share.
pub(super) struct Slot<'a> {
    admission: &'a Admission,
    session: String,
    _global: OwnedSemaphorePermit,
    pub waited: Duration,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.admission.release_session(&self.session);
    }
}

impl Admission {
    pub(super) fn new(global: usize, per_session: usize) -> Self {
        Self {
            global: Arc::new(Semaphore::new(global)),
            per_session,
            holders: Mutex::new(HashMap::new()),
            released: Notify::new(),
        }
    }

    /// The wait budget for a program with this deadline.
    pub(super) fn wait_budget(timeout_ms: u64) -> Duration {
        MAX_WAIT.min(Duration::from_millis(timeout_ms / 2))
    }

    pub(super) async fn acquire(&self, session: &str, wait: Duration) -> Result<Slot<'_>, Scope> {
        let started = Instant::now();
        let deadline = started + wait;
        // Session share first, so a session at its limit never occupies a
        // global slot while it waits for its own programs to finish.
        loop {
            let notified = self.released.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.reserve_session(session) {
                break;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(Scope::Session);
            }
        }
        let global = match Arc::clone(&self.global).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                match tokio::time::timeout_at(deadline, Arc::clone(&self.global).acquire_owned())
                    .await
                {
                    Ok(Ok(permit)) => permit,
                    _ => {
                        self.release_session(session);
                        return Err(Scope::Controller);
                    }
                }
            }
        };
        Ok(Slot {
            admission: self,
            session: session.to_owned(),
            _global: global,
            waited: started.elapsed(),
        })
    }

    fn reserve_session(&self, session: &str) -> bool {
        let mut holders = self.holders.lock().unwrap_or_else(|e| e.into_inner());
        let held = holders.entry(session.to_owned()).or_insert(0);
        if *held >= self.per_session {
            return false;
        }
        *held += 1;
        true
    }

    fn release_session(&self, session: &str) {
        {
            let mut holders = self.holders.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(held) = holders.get_mut(session) {
                *held = held.saturating_sub(1);
                if *held == 0 {
                    holders.remove(session);
                }
            }
        }
        self.released.notify_waiters();
    }

    /// Current holders as `session=count`, sorted, for refusal logs.
    pub(super) fn holders(&self) -> Vec<String> {
        let holders = self.holders.lock().unwrap_or_else(|e| e.into_inner());
        let mut listed: Vec<String> = holders
            .iter()
            .map(|(session, held)| format!("{session}={held}"))
            .collect();
        listed.sort();
        listed
    }

    pub(super) fn refusal(&self, scope: Scope, session: &str, waited: Duration) -> Value {
        tracing::warn!(
            session = session,
            scope = scope.code(),
            waited_ms = u64::try_from(waited.as_millis()).unwrap_or(u64::MAX),
            available = self.global.available_permits(),
            holders = ?self.holders(),
            "agent read program refused: no slot"
        );
        let message = match scope {
            Scope::Controller => {
                "All Controller read slots are busy (shared by every session). The program did not run; retry after retry_after_ms, and avoid long paging loops in one program."
            }
            Scope::Session => {
                "This session already runs the maximum concurrent read programs. The program did not run; wait for your other reads to finish, or combine them into one program."
            }
        };
        json!({"schema":1,"state":"error","error":{
            "code":"capacity",
            "scope":scope.code(),
            "retryable":true,
            "retry_after_ms":RETRY_AFTER_MS,
            "message":message,
        }})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_session_cannot_hold_every_slot() {
        let admission = Admission::new(3, 2);
        let a = admission.acquire("s1", Duration::ZERO).await.unwrap();
        let b = admission.acquire("s1", Duration::ZERO).await.unwrap();
        assert_eq!(
            admission.acquire("s1", Duration::ZERO).await.err(),
            Some(Scope::Session)
        );
        // Another session still gets the remaining slot.
        let c = admission.acquire("s2", Duration::ZERO).await.unwrap();
        assert_eq!(
            admission.acquire("s3", Duration::ZERO).await.err(),
            Some(Scope::Controller)
        );
        // A session-limited refusal must not have leaked a global slot.
        assert_eq!(admission.global.available_permits(), 0);
        drop((a, b, c));
        assert_eq!(admission.global.available_permits(), 3);
        assert!(admission.holders().is_empty());
    }

    #[tokio::test]
    async fn a_refused_global_wait_returns_the_session_share() {
        let admission = Admission::new(1, 2);
        let held = admission.acquire("s1", Duration::ZERO).await.unwrap();
        assert_eq!(
            admission
                .acquire("s2", Duration::from_millis(20))
                .await
                .err(),
            Some(Scope::Controller)
        );
        assert_eq!(admission.holders(), vec!["s1=1".to_owned()]);
        drop(held);
    }

    #[tokio::test]
    async fn a_waiter_is_admitted_when_a_slot_frees() {
        let admission = Arc::new(Admission::new(1, 1));
        let held = admission.acquire("s1", Duration::ZERO).await.unwrap();
        let waiter = {
            let admission = Arc::clone(&admission);
            tokio::spawn(async move {
                admission
                    .acquire("s1", Duration::from_secs(5))
                    .await
                    .map(|slot| slot.waited)
                    .is_ok()
            })
        };
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(held);
        assert!(waiter.await.unwrap());
        assert!(admission.holders().is_empty());
    }

    #[test]
    fn the_wait_never_exceeds_half_the_deadline() {
        assert_eq!(Admission::wait_budget(20_000), MAX_WAIT);
        assert_eq!(Admission::wait_budget(2_000), Duration::from_secs(1));
    }

    #[test]
    fn a_refusal_explains_its_scope() {
        let admission = Admission::new(1, 1);
        let value = admission.refusal(Scope::Controller, "s1", Duration::ZERO);
        assert_eq!(value["error"]["code"], "capacity");
        assert_eq!(value["error"]["scope"], "controller");
        assert_eq!(value["error"]["retry_after_ms"], RETRY_AFTER_MS);
        assert!(
            value["error"]["message"]
                .as_str()
                .unwrap()
                .contains("retry")
        );
    }
}

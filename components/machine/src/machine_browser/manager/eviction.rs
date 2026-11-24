//! Making room for an open on a full Machine. `browser.max_instances` caps
//! what runs; it is not a queue behind a browser nobody is viewing. An
//! instance stays for `browser.hibernate_after` after its viewer left, so
//! without this an owner who closed one account's browser and opened
//! another's was refused (`capacity`) until that grace ran out.

use super::supervision::still_idle;
use super::*;

/// How long an instance must have been detached before an open for another
/// account may take its slot. A viewer attaches moments after its open
/// answers (and re-attaches moments after a dropped stream); until then the
/// instance is detached but about to be viewed.
#[cfg(not(test))]
const EVICTION_SETTLE: Duration = Duration::from_secs(5);
#[cfg(test)]
const EVICTION_SETTLE: Duration = Duration::from_millis(300);

/// The longest an open waits for idle instances to make room, their settle
/// included. A stop already under way then finishes by itself; the open is
/// refused (`capacity`) and finds the slot free when it is retried.
#[cfg(not(test))]
const MAKE_ROOM_WAIT: Duration = Duration::from_secs(30);
#[cfg(test)]
const MAKE_ROOM_WAIT: Duration = Duration::from_secs(2);

/// What an open's caller waits for its answer, less a margin for the
/// answer's way back: the Controller's `OPEN_TIMEOUT` is 150 s.
const OPEN_ANSWER_WITHIN: Duration = Duration::from_secs(140);

/// Why an idle instance was left running after all.
struct Kept;

/// How long an open with `plan` may wait for room: never more than what the
/// start's own worst case (memory admission, unit activation, the control
/// connection and readiness) leaves of what the caller waits for, so making
/// room is never what lets an open outlive its caller.
fn room_wait(plan: &MachineBrowserRuntimePlan) -> Duration {
    let readiness = Duration::from_millis(plan.contract.launch.readiness_timeout_ms);
    let start = MEMORY_WAIT + UNIT_START_TIMEOUT + readiness * 2;
    MAKE_ROOM_WAIT.min(OPEN_ANSWER_WITHIN.saturating_sub(start))
}

impl BrowserManager {
    /// Reserve an instance slot for `account_id`'s open with `plan`.
    ///
    /// When every slot is taken, instances nobody is viewing make room,
    /// longest-detached first: each is hibernated gracefully (its stop-time
    /// snapshot is published, exactly as when its grace runs out) once it
    /// has been detached for [`EVICTION_SETTLE`]. An instance being started
    /// or viewed, or one a snapshot copy keeps busy, is never taken.
    ///
    /// Nothing is hibernated for an open that would be refused anyway. Its
    /// own record is checked first (a pending reset or revocation, a profile
    /// a newer browser wrote). Before every stop the reservation is tried
    /// again and the instances idle then (not when the open began) must
    /// suffice for the slots to free (a lowered limit, a unit whose exit is
    /// unconfirmed). And at the stop itself, under the instance's lifecycle
    /// and state locks, it must still be idle in the same detached interval
    /// with no copy running, a slot must still be missing, no Plugin
    /// uninstall may have begun, and this open must still be waiting.
    /// `max_instances = 0` turns browsers off on the Device and answers
    /// `not_available`.
    pub(super) async fn reserve_slot(
        self: &Arc<Self>,
        account_id: &ProviderAccountId,
        plan: &MachineBrowserRuntimePlan,
        max_instances: u32,
    ) -> Result<CapacityGuard, Reason> {
        if let Some(slot) = self.capacity.try_reserve(max_instances) {
            return Ok(slot);
        }
        let limit = usize::try_from(max_instances).unwrap_or(usize::MAX);
        if limit == 0 {
            return Err(Reason::NotAvailable);
        }
        self.preflight_record(account_id, plan)?;
        let until = Instant::now() + room_wait(plan);
        // Instances this open already decided about, stopped or kept.
        let mut tried: Vec<Arc<Instance>> = Vec::new();
        loop {
            if let Some(slot) = self.capacity.try_reserve(max_instances) {
                return Ok(slot);
            }
            if self.starts.draining() {
                return Err(Reason::NotAvailable);
            }
            let idle: Vec<_> = self
                .idle_instances(account_id)
                .into_iter()
                .filter(|(instance, _)| !tried.iter().any(|tried| Arc::ptr_eq(tried, instance)))
                .collect();
            let excess = (self.capacity.used() + 1).saturating_sub(limit);
            let Some((victim, since)) = idle.first().cloned() else {
                break;
            };
            let settled = since + EVICTION_SETTLE;
            if idle.len() < excess || settled > until {
                break;
            }
            if settled > Instant::now() {
                // Everything is decided again after the wait.
                tokio::time::sleep_until(settled).await;
                continue;
            }
            tried.push(Arc::clone(&victim));
            // A stop that began runs to its end whatever happens to this
            // open: an instance that entered `stopping` is never left there.
            let stop = tokio::spawn({
                let manager = Arc::clone(self);
                let opener = account_id.clone();
                async move {
                    manager
                        .make_room(&victim, since, (&opener, limit, until))
                        .await;
                }
            });
            if tokio::time::timeout_at(until, stop).await.is_err() {
                break;
            }
        }
        self.capacity.try_reserve(max_instances).ok_or_else(|| {
            tracing::info!(
                event_name = "browser.open.refused",
                browser_account = %account_id.as_str(),
                reason = "Capacity",
                slots_used = self.capacity.used(),
                max_instances,
                "browser open refused: every instance slot is in use"
            );
            Reason::Capacity
        })
    }

    /// Hibernate `victim` for `opener`'s open, unless by the time the stop
    /// would change anything (under the victim's lifecycle lock, where a
    /// copy is admitted, and its state lock) a copy runs, a slot is free
    /// after all, a Plugin uninstall began, the open stopped waiting, or the
    /// instance is no longer idle in the detached interval it was chosen
    /// for. Reports the instance's terminal state.
    async fn make_room(
        &self,
        victim: &Arc<Instance>,
        since: Instant,
        (opener, limit, until): (&ProviderAccountId, usize, Instant),
    ) {
        let stopped = self
            .stop_between_copies(
                victim,
                Reason::Reclaim,
                || {
                    let needed = self.capacity.used() >= limit;
                    if needed && !self.starts.draining() && Instant::now() < until {
                        Ok(())
                    } else {
                        Err(Kept)
                    }
                },
                |live| {
                    if still_idle(live, EVICTION_SETTLE, since) {
                        Ok(())
                    } else {
                        Err(Kept)
                    }
                },
            )
            .await;
        if let Ok(Some(event)) = stopped {
            tracing::info!(event_name = "browser.open.made_room",
                browser_account = %opener.as_str(),
                evicted_account = %victim.account_id.as_str(),
                evicted_instance = %victim.instance_id,
                evicted_state = ?event.state,
                "idle browser stopped to make room for an open");
            self.send_state(event);
        }
    }

    /// Running instances of other accounts that nobody is viewing and that
    /// no snapshot copy keeps busy, each with the start of its detached
    /// interval, longest-detached first.
    fn idle_instances(&self, except: &ProviderAccountId) -> Vec<(Arc<Instance>, Instant)> {
        let mut idle: Vec<_> = self
            .running()
            .into_iter()
            .filter(|instance| instance.account_id != *except && !instance.copy_running())
            .filter_map(|instance| {
                let since = {
                    let live = instance.live.lock();
                    (live.phase == Phase::Detached && live.attachment.is_none())
                        .then_some(live.detached_since)
                        .flatten()
                };
                since.map(|since| (instance, since))
            })
            .collect();
        idle.sort_by_key(|(_, since)| *since);
        idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_for_room_fits_what_the_start_leaves_of_the_caller_s_wait() {
        let mut plan = super::super::tests::plan();
        // The shipped contract (20 s readiness) leaves the whole wait.
        assert_eq!(plan.contract.launch.readiness_timeout_ms, 20_000);
        assert_eq!(room_wait(&plan), MAKE_ROOM_WAIT);
        // A start that may itself take the caller's whole wait leaves none.
        plan.contract.launch.readiness_timeout_ms = 60_000;
        assert_eq!(room_wait(&plan), Duration::ZERO);
        // In between, only what is left.
        plan.contract.launch.readiness_timeout_ms = 52_000;
        assert_eq!(room_wait(&plan), Duration::from_secs(1));
    }
}

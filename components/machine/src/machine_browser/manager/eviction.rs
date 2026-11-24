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
/// included: the settle and an ordinary graceful stop fit several times. A
/// stop already under way then finishes by itself; the open is refused
/// (`capacity`) and finds the slot free when it is retried.
#[cfg(not(test))]
const MAKE_ROOM_WAIT: Duration = Duration::from_secs(15);
#[cfg(test)]
const MAKE_ROOM_WAIT: Duration = Duration::from_secs(2);

/// What an open's caller waits for its answer, less a margin for the
/// answer's way back: the Controller's `OPEN_TIMEOUT` is 150 s.
const OPEN_ANSWER_WITHIN: Duration = Duration::from_secs(140);

/// Why an idle instance was left running after all.
struct Kept;

/// The open an idle instance would make room for.
struct Room {
    opener: ProviderAccountId,
    /// `browser.max_instances` as the open read it.
    limit: usize,
    /// When the open stops waiting.
    until: Instant,
    /// The instances the open decided about so far, this one included.
    tried: Vec<Arc<Instance>>,
}

/// How long an open with `plan` may wait for room: never more than every
/// bounded step of a start that succeeds leaves of what the caller waits
/// for (memory admission; `unit::start`'s status read and `systemd-run`;
/// activation with its last status read; the control connection and
/// readiness), so making room is never what lets a browser start behind a
/// caller that gave up. A start that fails answers a failure whenever it
/// answers, its termination included.
fn room_wait(plan: &MachineBrowserRuntimePlan) -> Duration {
    let readiness = Duration::from_millis(plan.contract.launch.readiness_timeout_ms);
    let start = MEMORY_WAIT + unit::SYSTEMCTL_TIMEOUT * 3 + UNIT_START_TIMEOUT + readiness * 2;
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
    /// again and the instances idle then (not when the open began) that can
    /// settle while the open waits must suffice for the slots to free (a
    /// lowered limit, a unit whose exit is unconfirmed). And at the stop
    /// itself, under the instance's lifecycle lock, that is decided once
    /// more with no copy running and no Plugin uninstall begun, and under
    /// its state lock, atomically with the change to `stopping`, it must
    /// still be idle in the same detached interval while this open is still
    /// waiting: no stop begins after the open answered.
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
            let idle = self.takeable(account_id, until, &tried);
            let Some((victim, since)) = idle.first().cloned() else {
                break;
            };
            if idle.len() < self.missing_slots(limit) {
                break;
            }
            let settled = since + EVICTION_SETTLE;
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
                let room = Room {
                    opener: account_id.clone(),
                    limit,
                    until,
                    tried: tried.clone(),
                };
                async move { manager.make_room(&victim, since, &room).await }
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

    /// Hibernate `victim` for the open `room` describes, unless by the time
    /// the stop would change anything a copy runs, no slot is missing any
    /// more or this stop with the instances still takeable would not free
    /// enough, a Plugin uninstall began (all under the victim's lifecycle
    /// lock, where a copy is admitted), or the instance is no longer idle in
    /// the detached interval it was chosen for or the open stopped waiting
    /// (under its state lock, with the change to `stopping`). Reports the
    /// instance's terminal state.
    async fn make_room(&self, victim: &Arc<Instance>, since: Instant, room: &Room) {
        let stopped = self
            .stop_between_copies(
                victim,
                Reason::Reclaim,
                LifecycleWait::Until,
                || {
                    let missing = self.missing_slots(room.limit);
                    // This one and those not decided about yet.
                    let able = 1 + self.takeable(&room.opener, room.until, &room.tried).len();
                    if (1..=able).contains(&missing) && !self.starts.draining() {
                        Ok(())
                    } else {
                        Err(Kept)
                    }
                },
                |live| {
                    if still_idle(live, EVICTION_SETTLE, since) && Instant::now() < room.until {
                        Ok(())
                    } else {
                        Err(Kept)
                    }
                },
            )
            .await;
        let opener = &room.opener;
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

    /// How many slots must be freed before one more instance fits `limit`.
    fn missing_slots(&self, limit: usize) -> usize {
        (self.capacity.used() + 1).saturating_sub(limit)
    }

    /// The idle instances an open waiting until `until` can still take:
    /// those that will have settled by then and that it has not decided
    /// about yet (`tried`), longest-detached first.
    fn takeable(
        &self,
        opener: &ProviderAccountId,
        until: Instant,
        tried: &[Arc<Instance>],
    ) -> Vec<(Arc<Instance>, Instant)> {
        self.idle_instances(opener)
            .into_iter()
            .filter(|(instance, since)| {
                *since + EVICTION_SETTLE <= until
                    && !tried.iter().any(|tried| Arc::ptr_eq(tried, instance))
            })
            .collect()
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
        plan.contract.launch.readiness_timeout_ms = 37_000;
        assert_eq!(room_wait(&plan), Duration::from_secs(1));
    }
}

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

/// How long an open waits for idle instances to make room, their settle
/// included. A stop still running then goes on by itself; the open is
/// refused (`capacity`) and finds the slot free when it is retried. With the
/// start that follows, the open stays within what its caller waits for.
#[cfg(not(test))]
const MAKE_ROOM_WAIT: Duration = Duration::from_secs(30);
#[cfg(test)]
const MAKE_ROOM_WAIT: Duration = Duration::from_secs(2);

/// Why an idle instance was left running after all.
struct Kept;

impl BrowserManager {
    /// Reserve an instance slot for `account_id`'s open with `plan`.
    ///
    /// When every slot is taken, instances nobody is viewing make room,
    /// longest-detached first: each is hibernated gracefully (its stop-time
    /// snapshot is published, exactly as when its grace runs out) once it
    /// has been detached for [`EVICTION_SETTLE`], re-checked under its live
    /// lock atomically with the change to `stopping`, so a viewer that
    /// attached meanwhile keeps it. An instance being started or viewed is
    /// never taken.
    ///
    /// Nothing is hibernated for an open that would be refused anyway: its
    /// own record is checked first (a pending reset or revocation, a profile
    /// a newer browser wrote), a Plugin uninstall that began meanwhile ends
    /// it, and before every stop the reservation is tried again and the
    /// instances still idle must suffice for the slots to free (a lowered
    /// limit, a unit whose exit is unconfirmed). `max_instances = 0` turns
    /// browsers off on the Device and answers `not_available`.
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
        let until = Instant::now() + MAKE_ROOM_WAIT;
        let idle = self.idle_instances(account_id);
        let candidates = idle.len();
        for (tried, (victim, since)) in idle.into_iter().enumerate() {
            let settled = since + EVICTION_SETTLE;
            if settled > until {
                break;
            }
            tokio::time::sleep_until(settled).await;
            // A slot freed meanwhile needs nobody hibernated.
            if let Some(slot) = self.capacity.try_reserve(max_instances) {
                return Ok(slot);
            }
            if self.starts.draining() {
                return Err(Reason::NotAvailable);
            }
            let excess = (self.capacity.used() + 1).saturating_sub(limit);
            if candidates - tried < excess {
                break;
            }
            // The stop runs to its end whatever happens to this open: an
            // instance that entered `stopping` is never left there.
            let stop = tokio::spawn({
                let manager = Arc::clone(self);
                let opener = account_id.clone();
                async move { manager.make_room(&victim, since, &opener).await }
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
                idle_instances = candidates,
                "browser open refused: every instance slot is in use"
            );
            Reason::Capacity
        })
    }

    /// Hibernate `victim` for `opener`'s open unless it stopped being idle
    /// (a viewer attached, another stop began) or a Plugin uninstall began,
    /// both checked after the stop's own waits, right before it changes
    /// anything. Reports the instance's terminal state.
    async fn make_room(&self, victim: &Arc<Instance>, since: Instant, opener: &ProviderAccountId) {
        let stopped = self
            .stop_instance_when(
                victim,
                Reason::Reclaim,
                true,
                None,
                || {
                    if self.starts.draining() {
                        Err(Kept)
                    } else {
                        Ok(())
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
        if let Ok(event) = stopped {
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

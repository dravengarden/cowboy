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

/// A viewer attached (or another stop began) while the eviction waited.
struct Renewed;

impl BrowserManager {
    /// Reserve an instance slot for `account_id`'s open. When every slot is
    /// taken, instances nobody is viewing make room, longest-detached first:
    /// each is hibernated gracefully (its stop-time snapshot is published,
    /// exactly as when its grace runs out) once it has been detached for
    /// [`EVICTION_SETTLE`], re-checked under its live lock atomically with
    /// the change to `stopping`, so a viewer that attached meanwhile keeps
    /// it. An instance being started or viewed is never taken: the open is
    /// then refused, as it is when the idle instances could not free a slot
    /// anyway (a lowered limit, a unit whose exit is unconfirmed).
    pub(super) async fn reserve_slot(
        &self,
        account_id: &ProviderAccountId,
        max_instances: u32,
    ) -> Option<CapacityGuard> {
        if let Some(slot) = self.capacity.try_reserve(max_instances) {
            return Some(slot);
        }
        let idle = self.idle_instances(account_id);
        let limit = usize::try_from(max_instances).unwrap_or(usize::MAX);
        let excess = (self.capacity.used() + 1).saturating_sub(limit);
        let refused = |idle: usize| {
            tracing::info!(
                event_name = "browser.open.refused",
                browser_account = %account_id.as_str(),
                reason = "Capacity",
                slots_used = self.capacity.used(),
                max_instances,
                idle_instances = idle,
                "browser open refused: every instance slot is in use"
            );
        };
        if limit == 0 || idle.len() < excess {
            refused(idle.len());
            return None;
        }
        for (victim, since) in idle {
            tokio::time::sleep_until(since + EVICTION_SETTLE).await;
            let stopped = self
                .stop_instance_when(
                    &victim,
                    Reason::Reclaim,
                    true,
                    None,
                    || Ok(()),
                    |live| {
                        if still_idle(live, EVICTION_SETTLE, since) {
                            Ok(())
                        } else {
                            Err(Renewed)
                        }
                    },
                )
                .await;
            if let Ok(event) = stopped {
                tracing::info!(event_name = "browser.open.made_room",
                    browser_account = %account_id.as_str(),
                    evicted_account = %victim.account_id.as_str(),
                    evicted_instance = %victim.instance_id,
                    evicted_state = ?event.state,
                    "idle browser stopped to make room for an open");
                self.send_state(event);
            }
            // Retried whatever happened to this one: a stop of its own (its
            // grace, a reset) may have freed the slot meanwhile.
            if let Some(slot) = self.capacity.try_reserve(max_instances) {
                return Some(slot);
            }
        }
        refused(0);
        None
    }

    /// Running instances of other accounts that nobody is viewing, each with
    /// the start of its detached interval, longest-detached first.
    fn idle_instances(&self, except: &ProviderAccountId) -> Vec<(Arc<Instance>, Instant)> {
        let mut idle: Vec<_> = self
            .running()
            .into_iter()
            .filter(|instance| instance.account_id != *except)
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

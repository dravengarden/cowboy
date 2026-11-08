//! Runtime generation pins: a browser instance runs from the exact installed
//! bytes of the generation it launched from, so Plugin generation retirement
//! keeps every generation a registered instance, an open still starting or a
//! unit retained until its confirmed exit names. Retirement runs under the
//! pin lock, and an open pins its generation under that lock only while the
//! generation still exists, so no start can launch from a retired generation.

use std::collections::BTreeMap;

use super::*;
use crate::machine_plugins::GenerationRetirement;

/// Generations pinned by opens that resolved a runtime but have not
/// registered their instance (or retained its unit) yet.
#[derive(Default)]
pub(super) struct GenerationPins {
    starting: parking_lot::Mutex<BTreeMap<String, usize>>,
    /// Whether a restart inventory pass has accounted for every surviving
    /// unit; before that the pins of surviving units are unknown.
    inventoried: std::sync::atomic::AtomicBool,
}

/// One open's pin; dropping it releases the generation.
pub(super) struct StartPin {
    pins: Arc<GenerationPins>,
    generation: String,
}

impl Drop for StartPin {
    fn drop(&mut self) {
        let mut starting = self.pins.starting.lock();
        if let Some(count) = starting.get_mut(&self.generation) {
            *count -= 1;
            if *count == 0 {
                starting.remove(&self.generation);
            }
        }
    }
}

impl GenerationPins {
    /// Pin `plan`'s generation for a start, or `None` when retirement
    /// removed it after the plan was resolved.
    pub(super) fn pin(self: &Arc<Self>, plan: &MachineBrowserRuntimePlan) -> Option<StartPin> {
        let mut starting = self.starting.lock();
        // Retirement renames a generation away under this lock, so the
        // launcher's presence here holds until the pin is released.
        if !std::fs::symlink_metadata(&plan.launcher).is_ok_and(|metadata| metadata.is_file()) {
            return None;
        }
        *starting.entry(plan.generation_digest.clone()).or_default() += 1;
        Some(StartPin {
            pins: Arc::clone(self),
            generation: plan.generation_digest.clone(),
        })
    }

    /// Record that a restart inventory pass completed.
    pub(super) fn inventoried(&self) {
        self.inventoried
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn starting(&self) -> Vec<String> {
        self.starting.lock().keys().cloned().collect()
    }
}

impl BrowserManager {
    /// Run `retire` with every pinned generation while new pins wait.
    pub(super) fn retire_unpinned(&self, retire: &mut GenerationRetirement<'_>) -> Result<()> {
        let starting = self.pins.starting.lock();
        ensure!(
            self.pins
                .inventoried
                .load(std::sync::atomic::Ordering::SeqCst)
                && !self.inventory_unresolved(),
            "the browser unit inventory is incomplete; generation pins are unknown"
        );
        let mut pinned: BTreeSet<String> = starting.keys().cloned().collect();
        pinned.extend(
            self.running()
                .iter()
                .map(|instance| instance.plan.generation_digest.clone()),
        );
        for retained in self.unconfirmed.lock().values() {
            let generation = retained
                .generation
                .clone()
                .context("a browser unit of unknown generation may still run")?;
            pinned.insert(generation);
        }
        retire(&pinned)
    }
}

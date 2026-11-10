//! Browser usage for the Machine's periodic host resource report: counts and
//! bytes from in-memory instance state and each instance's cgroup
//! `memory.current`. It never calls systemd and takes only the short
//! synchronous locks every reader takes, never an account lifecycle or
//! control lock a stop holds across awaits.

use super::{BrowserManager, Path};
use crate::machine_protocol::{BrowserResources, PluginInstallationState};

impl BrowserManager {
    /// Running instances and their memory, or `None` on a Machine without an
    /// active Machine browser Plugin and with no instance left running.
    #[must_use]
    pub fn resources(&self) -> Option<BrowserResources> {
        let instances = self.running();
        if instances.is_empty() && !self.plugin_active() {
            return None;
        }
        Some(aggregate(
            instances.iter().map(|instance| {
                let budget = instance
                    .capacity
                    .lock()
                    .as_ref()
                    .and_then(super::CapacityGuard::memory_bytes);
                (instance.cgroup_dir.as_path(), budget)
            }),
            self.settings().max_instances,
        ))
    }

    /// Whether a Machine browser Plugin is installed and active. Reads only
    /// the installation records, not the verified generation contents.
    fn plugin_active(&self) -> bool {
        self.providers.inventory().is_ok_and(|inventory| {
            inventory.iter().any(|installed| {
                installed.plugin_kind == cowboy_plugin_sdk::PluginKind::MachineBrowser
                    && installed.state == PluginInstallationState::Active
            })
        })
    }
}

/// Sum each instance's cgroup usage and admitted budget. A cgroup that is
/// gone or unreadable (an instance between exit and cleanup) counts zero.
fn aggregate<'a>(
    instances: impl Iterator<Item = (&'a Path, Option<u64>)>,
    max_instances: u32,
) -> BrowserResources {
    instances.fold(
        BrowserResources {
            max_instances,
            ..BrowserResources::default()
        },
        |mut total, (cgroup, budget)| {
            total.running = total.running.saturating_add(1);
            total.memory_bytes = total
                .memory_bytes
                .saturating_add(super::unit::memory_current(cgroup).unwrap_or(0));
            total.budget_bytes = total.budget_bytes.saturating_add(budget.unwrap_or(0));
            total
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1 << 20;

    #[test]
    fn aggregate_sums_live_cgroups_and_counts_unreadable_ones_as_zero() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let gone = temp.path().join("gone");
        for (cgroup, bytes) in [(&first, 640 * MIB), (&second, 50 * MIB)] {
            std::fs::create_dir_all(cgroup).unwrap();
            std::fs::write(cgroup.join("memory.current"), format!("{bytes}\n")).unwrap();
        }
        let total = aggregate(
            [
                (first.as_path(), Some(2048 * MIB)),
                (second.as_path(), Some(768 * MIB)),
                (gone.as_path(), None),
            ]
            .into_iter(),
            3,
        );
        assert_eq!(
            total,
            BrowserResources {
                running: 3,
                max_instances: 3,
                memory_bytes: 690 * MIB,
                budget_bytes: 2816 * MIB,
            }
        );
        assert_eq!(
            aggregate(std::iter::empty(), 2),
            BrowserResources {
                max_instances: 2,
                ..BrowserResources::default()
            }
        );
    }
}

//! Host resources a Machine reports with its heartbeat. Every field added
//! after the first release is optional so older Machines and Controllers
//! keep decoding each other's reports.

use serde::{Deserialize, Serialize};

/// Host resources a Machine observes for its own runtime. Load is reported in
/// thousandths so the type stays `Eq` like the rest of the protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResources {
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
    pub load_1m_milli: u64,
    pub cpu_count: u32,
    /// Filesystem holding the Machine state directory.
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    /// Memory charged to the Agent worker slice, when the host uses one.
    /// Machine browser units run in the same slice, so this includes
    /// [`BrowserResources::memory_bytes`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_memory_bytes: Option<u64>,
    /// Memory Agent workers can still use before a cgroup limit above them
    /// throttles them: the smallest `limit - current + inactive file cache`
    /// over the worker slice and its ancestors. Host `memory_available_bytes`
    /// can look ample while the enclosing user slice is at its `memory.high`.
    /// Absent where no ancestor is limited or on older Machines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_headroom_bytes: Option<u64>,
    pub uptime_seconds: u64,
    /// The Device's configured `disk.low_watermark`: below this much available
    /// disk the Controller runs an extra retention pass. Absent from Devices
    /// without the setting, and `0` disables the trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_low_watermark_bytes: Option<u64>,
    /// Machine browser instances and their memory. Absent on Machines
    /// without an active Machine browser Plugin and on older Machines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<BrowserResources>,
}

impl HostResources {
    /// Whether the Device reports less available disk than it declared it
    /// needs, so its Plugin generations should be retired now.
    #[must_use]
    pub fn below_disk_watermark(&self) -> bool {
        self.disk_low_watermark_bytes
            .is_some_and(|watermark| watermark > 0 && self.disk_available_bytes < watermark)
    }
}

/// Counts and bytes only: no account, instance or URL leaves the Machine in
/// a resource report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserResources {
    /// Instances this Machine currently runs, starting or stopping included.
    pub running: u32,
    /// The Device's `browser.max_instances`.
    pub max_instances: u32,
    /// Sum of the running instances' cgroup `memory.current`.
    pub memory_bytes: u64,
    /// Sum of the running instances' admitted budgets (each unit's
    /// `MemoryMax`).
    pub budget_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedHostResources {
    #[serde(flatten)]
    pub resources: HostResources,
    pub observed_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn older_report() -> serde_json::Value {
        serde_json::json!({
            "memory_total_bytes": 1, "memory_available_bytes": 1,
            "swap_total_bytes": 0, "swap_free_bytes": 0, "load_1m_milli": 0,
            "cpu_count": 1, "disk_total_bytes": 100, "disk_available_bytes": 10,
            "uptime_seconds": 1,
        })
    }

    #[test]
    fn disk_watermark_is_optional_compatible_and_zero_disables_it() {
        let mut resources: HostResources = serde_json::from_value(older_report())
            .expect("a report from an older Device still decodes");
        assert_eq!(resources.disk_low_watermark_bytes, None);
        assert!(!resources.below_disk_watermark());
        resources.disk_low_watermark_bytes = Some(0);
        assert!(!resources.below_disk_watermark());
        resources.disk_low_watermark_bytes = Some(10);
        assert!(!resources.below_disk_watermark());
        resources.disk_low_watermark_bytes = Some(11);
        assert!(resources.below_disk_watermark());
        let encoded = serde_json::to_value(&resources).unwrap();
        assert_eq!(encoded["disk_low_watermark_bytes"], 11);
    }

    #[test]
    fn browser_resources_are_optional_and_round_trip_through_the_observation() {
        let older: HostResources = serde_json::from_value(older_report()).unwrap();
        assert_eq!(older.browser, None);
        let encoded = serde_json::to_value(&older).unwrap();
        assert!(encoded.get("browser").is_none(), "absent stays absent");

        let browser = BrowserResources {
            running: 1,
            max_instances: 2,
            memory_bytes: 690 << 20,
            budget_bytes: 2 << 30,
        };
        let observed = ObservedHostResources {
            resources: HostResources {
                browser: Some(browser),
                ..older
            },
            observed_at_ms: 42,
        };
        let encoded = serde_json::to_value(&observed).unwrap();
        assert_eq!(
            encoded["browser"],
            serde_json::json!({
                "running": 1, "max_instances": 2,
                "memory_bytes": 690u64 << 20, "budget_bytes": 2u64 << 30,
            })
        );
        assert_eq!(encoded["observed_at_ms"], 42);
        let decoded: ObservedHostResources = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, observed);

        // A later Machine may add fields an older Controller ignores.
        let mut newer = older_report();
        newer["browser"] = serde_json::json!({
            "running": 0, "max_instances": 1, "memory_bytes": 0,
            "budget_bytes": 0, "future": true,
        });
        let decoded: HostResources = serde_json::from_value(newer).unwrap();
        assert_eq!(
            decoded.browser,
            Some(BrowserResources {
                max_instances: 1,
                ..BrowserResources::default()
            })
        );
    }
}

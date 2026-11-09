//! Device settings of the Machine browser (`browser.*`), read at every open.
//! Running units keep the properties they were started with.

use std::time::Duration;

use crate::config::{Config, Handle, Scope, schema};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub max_instances: u32,
    pub memory_max: u64,
    pub memory_high: u64,
    /// Bytes an open reserves in the launch ledger: the expected resident
    /// working set, not the unit's hard limit.
    pub memory_reserve: u64,
    pub cpu_quota_percent: u32,
    pub tasks_max: u32,
    pub hibernate_after: Duration,
    pub max_runtime: Duration,
    pub profile_max_bytes: u64,
    pub instance_max_bytes: u64,
    pub snapshot_interval: Duration,
    pub staging_max_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub disk_low_watermark: u64,
}

impl Settings {
    #[must_use]
    pub fn from_config(handle: Option<&Handle>) -> Self {
        let config = handle.map_or_else(
            || std::sync::Arc::new(Config::defaults(Scope::Device)),
            Handle::current,
        );
        let integer =
            |key: &crate::config::Key<i64>| u32::try_from(config.get(key)).unwrap_or(u32::MAX);
        Self {
            max_instances: integer(&schema::DEVICE_BROWSER_MAX_INSTANCES),
            memory_max: config.get(&schema::DEVICE_BROWSER_MEMORY_MAX),
            memory_high: config.get(&schema::DEVICE_BROWSER_MEMORY_HIGH),
            memory_reserve: config.get(&schema::DEVICE_BROWSER_MEMORY_RESERVE),
            cpu_quota_percent: integer(&schema::DEVICE_BROWSER_CPU_QUOTA),
            tasks_max: integer(&schema::DEVICE_BROWSER_TASKS_MAX),
            hibernate_after: config.get(&schema::DEVICE_BROWSER_HIBERNATE_AFTER),
            max_runtime: config.get(&schema::DEVICE_BROWSER_MAX_RUNTIME),
            profile_max_bytes: config.get(&schema::DEVICE_BROWSER_PROFILE_MAX_BYTES),
            instance_max_bytes: config.get(&schema::DEVICE_BROWSER_INSTANCE_MAX_BYTES),
            snapshot_interval: config.get(&schema::DEVICE_BROWSER_SNAPSHOT_INTERVAL),
            staging_max_bytes: config.get(&schema::DEVICE_BROWSER_STAGING_MAX_BYTES),
            max_width: integer(&schema::DEVICE_BROWSER_MAX_WIDTH),
            max_height: integer(&schema::DEVICE_BROWSER_MAX_HEIGHT),
            disk_low_watermark: config.get(&schema::DEVICE_DISK_LOW_WATERMARK),
        }
    }

    /// Whether a requested framebuffer fits this Device's bound and the
    /// protocol minimum.
    #[must_use]
    pub fn geometry_allowed(&self, width: u32, height: u32) -> bool {
        use crate::machine_protocol::machine_browser::{MIN_HEIGHT, MIN_WIDTH};
        (MIN_WIDTH..=self.max_width).contains(&width)
            && (MIN_HEIGHT..=self.max_height).contains(&height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_design_table() {
        let settings = Settings::from_config(None);
        assert_eq!(settings.max_instances, 1);
        assert_eq!(settings.memory_max, 2 << 30);
        assert_eq!(settings.memory_high, 1536 << 20);
        assert_eq!(settings.memory_reserve, 768 << 20);
        assert!(settings.memory_reserve <= settings.memory_high);
        assert!(settings.memory_high <= settings.memory_max);
        assert_eq!(settings.cpu_quota_percent, 200);
        assert_eq!(settings.tasks_max, 512);
        assert_eq!(settings.hibernate_after, Duration::from_secs(120));
        assert_eq!(settings.max_runtime, Duration::from_secs(30 * 60));
        assert_eq!(settings.profile_max_bytes, 256 << 20);
        assert_eq!(settings.instance_max_bytes, 128 << 20);
        assert_eq!(settings.snapshot_interval, Duration::from_secs(600));
        assert_eq!(settings.staging_max_bytes, 1 << 30);
        assert!(settings.geometry_allowed(2560, 1600));
        assert!(!settings.geometry_allowed(2561, 1600));
        assert!(!settings.geometry_allowed(319, 800));
    }
}

//! Every Service and Device setting, declared once.
//!
//! To add a setting: append a [`Field`] to the right scope, add a typed
//! [`Key`] constant beside it, read it through `Config::get`/`Handle::get`, and
//! regenerate `config/*.example.toml` with `cowboy config init --print`.
//! Decide the scope with docs/configuration.md: a setting lives with the
//! process that enforces it. Keep the fields of one section adjacent.

use std::time::Duration;

use super::{Field, Key, Kind, Reload, Scope, Value};

// ---- Service (Controller) -------------------------------------------------

pub const PLUGIN_GENERATION_RETENTION_INTERVAL: Key<Duration> =
    Key::new(Scope::Service, "plugins.generation_retention_interval");
pub const PLUGIN_URGENT_RETENTION_COOLDOWN: Key<Duration> =
    Key::new(Scope::Service, "plugins.urgent_retention_cooldown");
pub const PLUGIN_REPIN_DORMANT_SESSIONS: Key<bool> =
    Key::new(Scope::Service, "plugins.repin_dormant_sessions");
pub const PLUGIN_REPIN_DORMANT_AFTER: Key<Duration> =
    Key::new(Scope::Service, "plugins.repin_dormant_after");
pub const SESSIONS_RECLAIM_ON_CAPACITY: Key<bool> =
    Key::new(Scope::Service, "sessions.reclaim_on_capacity");
pub const SESSIONS_RECLAIM_MIN_IDLE: Key<Duration> =
    Key::new(Scope::Service, "sessions.reclaim_min_idle");

pub static SERVICE_FIELDS: &[Field] = &[
    Field {
        key: "plugins.generation_retention_interval",
        kind: Kind::Duration {
            min: Duration::from_mins(10),
            max: Duration::from_hours(7 * 24),
        },
        default: Value::Duration(Duration::from_hours(6)),
        reload: Reload::Live,
        doc: "How often the Controller asks every connected Device to retire Plugin\ngenerations that no open, hibernated or unpurged session pins.",
    },
    Field {
        key: "plugins.urgent_retention_cooldown",
        kind: Kind::Duration {
            min: Duration::from_mins(5),
            max: Duration::from_hours(7 * 24),
        },
        default: Value::Duration(Duration::from_hours(1)),
        reload: Reload::Live,
        doc: "Minimum time between extra retention passes for one Device whose\navailable disk is below its declared `disk.low_watermark`.",
    },
    Field {
        key: "plugins.repin_dormant_sessions",
        kind: Kind::Bool,
        default: Value::Bool(false),
        reload: Reload::Live,
        doc: "Move sessions that have been exited without a worker for\n`plugins.repin_dormant_after` to their Device's installed Provider release,\nwhen its native session contract is unchanged, so their old generation can be\nretired. They resume on the new release when opened, like an explicit Reload.",
    },
    Field {
        key: "plugins.repin_dormant_after",
        kind: Kind::Duration {
            min: Duration::from_hours(1),
            max: Duration::from_hours(365 * 24),
        },
        default: Value::Duration(Duration::from_hours(7 * 24)),
        reload: Reload::Live,
        doc: "How long a session must stay dormant before it is re-pinned. Measured\nacross Controller restarts from the first retention pass that saw it dormant.",
    },
    Field {
        key: "sessions.reclaim_on_capacity",
        kind: Kind::Bool,
        default: Value::Bool(false),
        reload: Reload::Live,
        doc: "When a new session targets a Device whose session slots are full, hibernate\nthe longest-idle eligible session on it first instead of refusing.\nHibernation sends no model request and the session resumes when opened.",
    },
    Field {
        key: "sessions.reclaim_min_idle",
        kind: Kind::Duration {
            min: Duration::from_mins(5),
            max: Duration::from_hours(7 * 24),
        },
        default: Value::Duration(Duration::from_hours(1)),
        reload: Reload::Live,
        doc: "A session is eligible for reclaim only after this long without any event.\nKeep it at or above the longest Provider prompt-cache lifetime so a\nreclaimed session's cache has already expired and resuming costs nothing\nextra.",
    },
];

// ---- Device (Machine) -----------------------------------------------------

pub const DEVICE_MAX_SESSIONS: Key<i64> = Key::new(Scope::Device, "capacity.max_sessions");
pub const DEVICE_DRAINING: Key<bool> = Key::new(Scope::Device, "capacity.draining");
pub const DEVICE_DISK_LOW_WATERMARK: Key<u64> = Key::new(Scope::Device, "disk.low_watermark");
pub const DEVICE_ARTIFACT_CACHE_UNREFERENCED_AFTER: Key<Duration> =
    Key::new(Scope::Device, "retention.artifact_cache_unreferenced_after");

pub static DEVICE_FIELDS: &[Field] = &[
    Field {
        key: "capacity.max_sessions",
        kind: Kind::Integer { min: 1, max: 1024 },
        default: Value::Integer(8),
        reload: Reload::Restart,
        doc: "Maximum detached ACP sessions this Device accepts.\nAn explicit --max-sessions / COWBOY_MACHINE_MAX_SESSIONS still takes precedence.",
    },
    Field {
        key: "capacity.draining",
        kind: Kind::Bool,
        default: Value::Bool(false),
        reload: Reload::Restart,
        doc: "Keep existing sessions alive while refusing new placement.\nAn explicit --draining / COWBOY_MACHINE_DRAINING still takes precedence.",
    },
    Field {
        key: "disk.low_watermark",
        kind: Kind::Bytes {
            min: 0,
            max: 1 << 44,
        },
        default: Value::Bytes(15 << 30),
        reload: Reload::Live,
        doc: "When free space on the filesystem holding the Device state falls below\nthis, the Controller runs an extra retention pass for this Device.\n\"0B\" disables the trigger.",
    },
    Field {
        key: "retention.artifact_cache_unreferenced_after",
        kind: Kind::Duration {
            min: Duration::from_hours(1),
            max: Duration::from_hours(365 * 24),
        },
        default: Value::Duration(Duration::from_hours(7 * 24)),
        reload: Reload::Live,
        doc: "Remove a cached runtime artifact once no retained Plugin generation\nreferences it and it has not been written for this long.",
    },
];

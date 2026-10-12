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
pub const PLUGIN_AUTO_UPDATE_IDLE_SESSIONS: Key<bool> =
    Key::new(Scope::Service, "plugins.auto_update_idle_sessions");
pub const PLUGIN_AUTO_UPDATE_IDLE_AFTER: Key<Duration> =
    Key::new(Scope::Service, "plugins.auto_update_idle_after");
pub const PLUGIN_AUTO_UPDATE_IDLE_LIMIT: Key<Duration> =
    Key::new(Scope::Service, "plugins.auto_update_idle_limit");
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
        key: "plugins.auto_update_idle_sessions",
        kind: Kind::Bool,
        default: Value::Bool(false),
        reload: Reload::Live,
        doc: "Move every idle, non-system session to its Device's newer installed\nProvider release once it has had no event for `plugins.auto_update_idle_after`,\nthrough the same gate and native resume as an explicit Reload. A session\nwithout a worker is only re-pinned, so it opens on the new release. Sessions\nthat opted in, or asked to update after their turn, update as soon as idle.",
    },
    Field {
        key: "plugins.auto_update_idle_after",
        kind: Kind::Duration {
            min: Duration::from_mins(5),
            max: Duration::from_hours(7 * 24),
        },
        default: Value::Duration(Duration::from_hours(1)),
        reload: Reload::Live,
        doc: "Idle time before the fleet policy updates a session. Keep it at or above the\nlongest Provider prompt-cache lifetime so the next prompt costs nothing extra.",
    },
    Field {
        key: "plugins.auto_update_idle_limit",
        kind: Kind::Duration {
            min: Duration::from_hours(1),
            max: Duration::from_hours(30 * 24),
        },
        default: Value::Duration(Duration::from_hours(12)),
        reload: Reload::Live,
        doc: "Idle time after which the fleet policy stops restarting a session's worker\nfor each new release. The session then adopts the installed release when it\nis next opened, so one nobody is using does not resume its native session on\nevery release. Workerless sessions are still re-pinned.",
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

pub const DEVICE_CONTROLLER_PROBE_TIMEOUT: Key<Duration> =
    Key::new(Scope::Device, "connection.controller_probe_timeout");

pub const DEVICE_FILE_BUFFERS_ENABLED: Key<bool> = Key::new(Scope::Device, "file_buffers.enabled");
pub const DEVICE_FILE_BUFFERS_MAX_FILE_SIZE: Key<u64> =
    Key::new(Scope::Device, "file_buffers.max_file_size");
pub const DEVICE_FILE_BUFFERS_EXCLUDED: Key<Vec<String>> =
    Key::new(Scope::Device, "file_buffers.excluded");
pub const DEVICE_FILE_BUFFERS_CACHE_MAX_BYTES: Key<u64> =
    Key::new(Scope::Device, "file_buffers.cache_max_bytes");
pub const DEVICE_FILE_BUFFERS_CACHE_MAX_AGE: Key<Duration> =
    Key::new(Scope::Device, "file_buffers.cache_max_age");
pub const DEVICE_FILE_BUFFERS_TARGET_TEMP_MAX_AGE: Key<Duration> =
    Key::new(Scope::Device, "file_buffers.target_temp_max_age");

/// Secrets, binary media and live databases never get a local buffer copy.
const FILE_BUFFERS_EXCLUDED: &[std::borrow::Cow<'static, str>] = &[
    std::borrow::Cow::Borrowed(".env"),
    std::borrow::Cow::Borrowed(".env.*"),
    std::borrow::Cow::Borrowed("*.pem"),
    std::borrow::Cow::Borrowed("*.key"),
    std::borrow::Cow::Borrowed("*.p12"),
    std::borrow::Cow::Borrowed("*.pfx"),
    std::borrow::Cow::Borrowed("*.kdbx"),
    std::borrow::Cow::Borrowed("id_rsa*"),
    std::borrow::Cow::Borrowed("id_ecdsa*"),
    std::borrow::Cow::Borrowed("id_ed25519*"),
    std::borrow::Cow::Borrowed("**/.ssh/**"),
    std::borrow::Cow::Borrowed("**/.gnupg/**"),
    std::borrow::Cow::Borrowed("*.png"),
    std::borrow::Cow::Borrowed("*.jpg"),
    std::borrow::Cow::Borrowed("*.jpeg"),
    std::borrow::Cow::Borrowed("*.gif"),
    std::borrow::Cow::Borrowed("*.webp"),
    std::borrow::Cow::Borrowed("*.pdf"),
    std::borrow::Cow::Borrowed("*.ipynb"),
    std::borrow::Cow::Borrowed("*.db"),
    std::borrow::Cow::Borrowed("*.sqlite"),
    std::borrow::Cow::Borrowed("*.sqlite3"),
];

pub static DEVICE_FIELDS: &[Field] = &[
    Field {
        key: "connection.controller_probe_timeout",
        kind: Kind::Duration {
            min: Duration::from_secs(5),
            max: Duration::from_mins(10),
        },
        default: Value::Duration(Duration::from_secs(45)),
        reload: Reload::Live,
        doc: "Reconnect when the Controller does not answer a WebSocket Ping within\nthis budget, even if a local proxy still accepts writes. Applies to new probes.",
    },
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
    Field {
        key: "file_buffers.enabled",
        kind: Kind::Bool,
        default: Value::Bool(true),
        reload: Reload::Live,
        doc: "Remote sessions keep a verified copy of each file they read and edit it\nwith a hash-checked splice on the target instead of resending the file.\nfalse returns every file to whole-file reads and writes.",
    },
    Field {
        key: "file_buffers.max_file_size",
        kind: Kind::Bytes {
            min: 1024,
            max: 4 << 20,
        },
        default: Value::Bytes(2 << 20),
        reload: Reload::Live,
        doc: "Larger files are read and written whole, without a buffered copy.",
    },
    Field {
        key: "file_buffers.excluded",
        kind: Kind::Patterns { max_items: 256 },
        default: Value::Patterns(std::borrow::Cow::Borrowed(FILE_BUFFERS_EXCLUDED)),
        reload: Reload::Live,
        doc: "Files matching any glob are read and written whole and never copied to\nthis Device. A pattern without `/` matches the file name; one with `/`\nmatches the absolute target path, where `**` spans directories. Matching\nignores ASCII case. Replacing the list drops the defaults.",
    },
    Field {
        key: "file_buffers.cache_max_bytes",
        kind: Kind::Bytes {
            min: 0,
            max: 1 << 30,
        },
        default: Value::Bytes(64 << 20),
        reload: Reload::Live,
        doc: "Each session keeps at most this many bytes of buffered copies, evicting\nthe least recently used first. \"0B\" keeps none.",
    },
    Field {
        key: "file_buffers.cache_max_age",
        kind: Kind::Duration {
            min: Duration::from_mins(1),
            max: Duration::from_hours(30 * 24),
        },
        default: Value::Duration(Duration::from_hours(24)),
        reload: Reload::Live,
        doc: "A buffered copy unused for this long is deleted from this Device.",
    },
    Field {
        key: "file_buffers.target_temp_max_age",
        kind: Kind::Duration {
            min: Duration::from_mins(10),
            max: Duration::from_hours(7 * 24),
        },
        default: Value::Duration(Duration::from_hours(1)),
        reload: Reload::Live,
        doc: "On the target, a temporary or request file an interrupted write left\nbehind is deleted by the next write in its directory once it is this old.",
    },
];

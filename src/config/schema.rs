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

pub const ACCOUNT_REVOCATION_RETRY_INTERVAL: Key<Duration> =
    Key::new(Scope::Service, "accounts.revocation_retry_interval");
pub const ACCOUNT_NATIVE_CLAUDE_LOGIN: Key<bool> =
    Key::new(Scope::Service, "accounts.native_claude_login");
pub const ACCOUNT_NATIVE_CLAUDE_SESSIONS: Key<bool> =
    Key::new(Scope::Service, "accounts.native_claude_sessions");
pub const USAGE_PROVIDER_WINDOW: Key<Duration> = Key::new(Scope::Service, "usage.provider_window");
pub const USAGE_REQUESTS_PER_WINDOW: Key<i64> =
    Key::new(Scope::Service, "usage.requests_per_window");
pub const USAGE_ACCOUNT_INTERVAL: Key<Duration> =
    Key::new(Scope::Service, "usage.account_interval");
pub const USAGE_FAILURE_BACKOFF_MAX: Key<Duration> =
    Key::new(Scope::Service, "usage.failure_backoff_max");

pub const PLUGIN_GENERATION_RETENTION_INTERVAL: Key<Duration> =
    Key::new(Scope::Service, "plugins.generation_retention_interval");
pub const PLUGIN_DISABLED: Key<Vec<String>> = Key::new(Scope::Service, "plugins.disabled");
pub const PLUGIN_URGENT_RETENTION_COOLDOWN: Key<Duration> =
    Key::new(Scope::Service, "plugins.urgent_retention_cooldown");
pub const PLUGIN_CATALOG_KEEP_RELEASES: Key<i64> =
    Key::new(Scope::Service, "plugins.catalog_keep_releases");
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
pub const SESSIONS_MEMORY_RECLAIM: Key<bool> = Key::new(Scope::Service, "sessions.memory_reclaim");
pub const SESSIONS_MEMORY_RECLAIM_BELOW: Key<u64> =
    Key::new(Scope::Service, "sessions.memory_reclaim_below");
pub const SESSIONS_MEMORY_RECLAIM_UNTIL: Key<u64> =
    Key::new(Scope::Service, "sessions.memory_reclaim_until");
pub const SESSIONS_MEMORY_RECLAIM_MIN_IDLE: Key<Duration> =
    Key::new(Scope::Service, "sessions.memory_reclaim_min_idle");

pub const MEMORY_HOME_MACHINES: Key<Vec<String>> =
    Key::new(Scope::Service, "memory.home_machines");

pub static SERVICE_FIELDS: &[Field] = &[
    Field {
        key: "memory.home_machines",
        kind: Kind::Identifiers { max_items: 1 },
        default: Value::Patterns(std::borrow::Cow::Borrowed(&[])),
        reload: Reload::Live,
        doc: "The Machine that hosts the memory backend. Sessions on every other Machine\nreach it over their Machine connection, with no network path or credential\nof their own. Empty: those sessions run without memory.",
    },
    Field {
        key: "usage.provider_window",
        kind: Kind::Duration {
            min: Duration::from_mins(2),
            max: Duration::from_hours(1),
        },
        default: Value::Duration(Duration::from_mins(2)),
        reload: Reload::Live,
        doc: "Rolling admission window shared by every account of one usage Provider.\nAll Service quota collectors, manual and automatic, use this durable budget.\nPassive session usage and reads of cached values do not consume it.",
    },
    Field {
        key: "usage.requests_per_window",
        kind: Kind::Integer { min: 1, max: 10 },
        default: Value::Integer(2),
        reload: Reload::Live,
        doc: "Maximum quota collector invocations per Provider window across all accounts.\nStarts are evenly spaced; only one collector per Provider runs at a time.\nNative CLIs own internal HTTP; this is not a model-inference request limit.",
    },
    Field {
        key: "usage.account_interval",
        kind: Kind::Duration {
            min: Duration::from_mins(5),
            max: Duration::from_hours(1),
        },
        default: Value::Duration(Duration::from_mins(5)),
        reload: Reload::Live,
        doc: "Minimum interval between quota queries for the same account, including\nfailures and manual refresh. Changing Machine or login does not reset it.",
    },
    Field {
        key: "usage.failure_backoff_max",
        kind: Kind::Duration {
            min: Duration::from_hours(1),
            max: Duration::from_hours(24),
        },
        default: Value::Duration(Duration::from_hours(1)),
        reload: Reload::Live,
        doc: "Maximum exponential backoff after quota collection failures.\nAuthentication failures wait this long; cached reads remain available.",
    },
    Field {
        key: "accounts.native_claude_sessions",
        kind: Kind::Bool,
        default: Value::Bool(false),
        reload: Reload::Live,
        doc: "Allow new named Claude sessions with an admitted native login.\nDoes not start logins or bypass Machine, release, identity or runtime checks.\nDisabling refuses new creation; existing sessions keep their lifecycle checks.",
    },
    Field {
        key: "accounts.native_claude_login",
        kind: Kind::Bool,
        default: Value::Bool(false),
        reload: Reload::Live,
        doc: "Allow explicit named Claude native logins on account-aware Machines.\nDoes not enable named session creation, copy credentials, or start a login.\nDisabling refuses new logins; existing receipt recovery and cleanup remain available.",
    },
    Field {
        key: "accounts.revocation_retry_interval",
        kind: Kind::Duration {
            min: Duration::from_secs(30),
            max: Duration::from_hours(24),
        },
        default: Value::Duration(Duration::from_mins(1)),
        reload: Reload::Live,
        doc: "Delay between bounded recovery pages of existing account revocation intents.\nDoes not start login, probe Providers, or declare upstream logout complete.",
    },
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
        key: "plugins.disabled",
        kind: Kind::Identifiers { max_items: 128 },
        default: Value::Patterns(std::borrow::Cow::Borrowed(&[])),
        reload: Reload::Restart,
        doc: "Plugin IDs excluded from the Service Catalog, Provider discovery and host\nactivation. Also blocks new installations and login/session admission.\nUninstall idle Machine installations separately; history and credentials are retained.",
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
        key: "plugins.catalog_keep_releases",
        kind: Kind::Integer { min: 0, max: 64 },
        default: Value::Integer(3),
        reload: Reload::Live,
        doc: "Newest releases of each Plugin the Service Catalog keeps beyond the ones a\nrecoverable session pins, a Device reports installed or targets in an install\noperation, or the host policy selects. Each scheduled retention pass retires\nthe other releases once they are a day old, with the artifacts only they used.\n`0` keeps every published release.",
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
        doc: "A session is eligible for reclaim only after this long without any event.\nA Provider prompt cache outlives the process only while unexpired and when\nthe resumed request repeats the same prefix, so below the cache lifetime a\nresume can turn a cache read into a cache write.",
    },
    Field {
        key: "sessions.memory_reclaim",
        kind: Kind::Bool,
        default: Value::Bool(false),
        reload: Reload::Live,
        doc: "Hibernate idle sessions on a Device whose reported memory headroom (the\ntightest cgroup limit above its Agent workers, less their usage, plus\ndroppable file cache) falls below memory_reclaim_below, longest-idle first\nand one at a time, until it is back above memory_reclaim_until.",
    },
    Field {
        key: "sessions.memory_reclaim_below",
        kind: Kind::Bytes {
            min: 128 << 20,
            max: 1 << 40,
        },
        default: Value::Bytes(1 << 30),
        reload: Reload::Live,
        doc: "Headroom below which memory reclaim starts.",
    },
    Field {
        key: "sessions.memory_reclaim_until",
        kind: Kind::Bytes {
            min: 128 << 20,
            max: 1 << 40,
        },
        default: Value::Bytes(2 << 30),
        reload: Reload::Live,
        doc: "Headroom at which memory reclaim stops; never below memory_reclaim_below.",
    },
    Field {
        key: "sessions.memory_reclaim_min_idle",
        kind: Kind::Duration {
            min: Duration::from_mins(5),
            max: Duration::from_hours(7 * 24),
        },
        default: Value::Duration(Duration::from_mins(15)),
        reload: Reload::Live,
        doc: "Memory reclaim only hibernates sessions this long without any event. Busy,\npermission-waiting, background-working, system and cache-protected sessions\nare never chosen.",
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

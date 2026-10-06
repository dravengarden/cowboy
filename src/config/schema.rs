//! Every Service and Device setting, declared once.
//!
//! To add a setting: append a [`Field`] to the right scope, add a typed
//! [`Key`] constant beside it, read it through `Config::get`/`Handle::get`, and
//! regenerate `config/*.example.toml` with `cowboy config init --print`.
//! Decide the scope with docs/configuration.md: a setting lives with the
//! process that enforces it.

use std::time::Duration;

use super::{Field, Key, Kind, Reload, Scope, Value};

// ---- Service (Controller) -------------------------------------------------

pub const PLUGIN_GENERATION_RETENTION_INTERVAL: Key<Duration> =
    Key::new(Scope::Service, "plugins.generation_retention_interval");

pub static SERVICE_FIELDS: &[Field] = &[Field {
    key: "plugins.generation_retention_interval",
    kind: Kind::Duration {
        min: Duration::from_mins(10),
        max: Duration::from_hours(7 * 24),
    },
    default: Value::Duration(Duration::from_hours(6)),
    reload: Reload::Live,
    doc: "How often the Controller asks every connected Device to retire Plugin\ngenerations that no open, hibernated or unpurged session pins.",
}];

// ---- Device (Machine) -----------------------------------------------------

pub const DEVICE_MAX_SESSIONS: Key<i64> = Key::new(Scope::Device, "capacity.max_sessions");
pub const DEVICE_DRAINING: Key<bool> = Key::new(Scope::Device, "capacity.draining");

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
];

//! Cowboy's unified configuration framework.
//!
//! The Service (Controller) and each Device (Machine) own one independent TOML
//! file. Both use the same declarative schema: every setting is a [`Field`] with
//! a kind, default, reload class and documentation, so parsing, validation,
//! diagnostics, `cowboy config explain/schema`, the generated example files and
//! hot reload come from one place. Adding a setting means adding one `Field` and
//! one typed [`Key`] in `config/schema.rs`; nothing else changes.
//!
//! Identity, wiring (paths, sockets, URLs) and secrets never belong here; they
//! stay command-line or environment inputs. See `docs/configuration.md`.

#![warn(clippy::pedantic)]

pub mod cli;
pub mod schema;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

/// The only schema version this build understands. A future incompatible
/// layout bumps it and ships an explicit migration.
pub const SCHEMA_VERSION: i64 = 1;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const RELOAD_POLL: Duration = Duration::from_secs(5);
const HISTORY_LIMIT: usize = 50;

/// Which independent configuration a file belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The Cowboy Service (Controller): fleet-wide behaviour and decisions.
    Service,
    /// One Cowboy Device (Machine): its own capacity, budgets and local data.
    Device,
}

impl Scope {
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Service => "cowboy-service.toml",
            Self::Device => "cowboy-device.toml",
        }
    }

    /// `<data-dir>/config/…` for the Service, `<state-dir>/config/…` for a Device.
    #[must_use]
    pub fn path_in(self, root: &Path) -> PathBuf {
        root.join("config").join(self.file_name())
    }

    #[must_use]
    pub fn fields(self) -> &'static [Field] {
        match self {
            Self::Service => schema::SERVICE_FIELDS,
            Self::Device => schema::DEVICE_FIELDS,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Device => "device",
        }
    }
}

/// When a changed value takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reload {
    /// Applied by the running process within seconds of a valid file change.
    Live,
    /// Read once at startup; a change needs a (rolling) restart.
    Restart,
}

/// The accepted shape and bounds of one value.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Bool,
    Integer {
        min: i64,
        max: i64,
    },
    /// Written as `"30s"`, `"10m"`, `"6h"` or `"7d"`.
    Duration {
        min: Duration,
        max: Duration,
    },
    /// Written as `"512MiB"`, `"1.5GiB"` or a plain byte integer.
    Bytes {
        min: u64,
        max: u64,
    },
    Choice(&'static [&'static str]),
}

impl Kind {
    fn describe(&self) -> String {
        match self {
            Self::Bool => "boolean (true or false)".to_owned(),
            Self::Integer { min, max } => format!("integer from {min} to {max}"),
            Self::Duration { min, max } => format!(
                "duration string such as \"6h\", from {} to {}",
                format_duration(*min),
                format_duration(*max)
            ),
            Self::Bytes { min, max } => format!(
                "size string such as \"1.5GiB\", from {} to {}",
                format_bytes(*min),
                format_bytes(*max)
            ),
            Self::Choice(options) => format!("one of {}", quoted_list(options)),
        }
    }

    const fn json_type(&self) -> &'static str {
        match self {
            Self::Bool => "boolean",
            Self::Integer { .. } => "integer",
            Self::Duration { .. } | Self::Bytes { .. } | Self::Choice(_) => "string",
        }
    }
}

/// A validated configuration value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    Duration(Duration),
    Bytes(u64),
    Choice(&'static str),
}

impl Value {
    /// The TOML spelling, used by `show`, `explain` and the generated examples.
    #[must_use]
    pub fn to_toml(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::Integer(value) => value.to_string(),
            Self::Duration(value) => format!("\"{}\"", format_duration(*value)),
            Self::Bytes(value) => format!("\"{}\"", format_bytes(*value)),
            Self::Choice(value) => format!("\"{value}\""),
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Bool(value) => (*value).into(),
            Self::Integer(value) => (*value).into(),
            Self::Duration(value) => format_duration(*value).into(),
            Self::Bytes(value) => format_bytes(*value).into(),
            Self::Choice(value) => (*value).into(),
        }
    }
}

/// One declared setting. `key` is the dotted path inside the file, for example
/// `capacity.max_sessions` is `max_sessions` in the `[capacity]` table.
#[derive(Debug)]
pub struct Field {
    pub key: &'static str,
    pub kind: Kind,
    pub default: Value,
    pub reload: Reload,
    pub doc: &'static str,
}

/// Typed, compile-time handle to a declared field. A test proves every key
/// exists in its scope with a matching kind, so lookups cannot fail at runtime.
pub struct Key<T> {
    pub scope: Scope,
    pub path: &'static str,
    marker: PhantomData<fn() -> T>,
}

impl<T> Key<T> {
    #[must_use]
    pub const fn new(scope: Scope, path: &'static str) -> Self {
        Self {
            scope,
            path,
            marker: PhantomData,
        }
    }
}

/// Conversion from a validated [`Value`] to the type a [`Key`] promises.
pub trait FromValue: Sized {
    fn from_value(value: &Value) -> Option<Self>;
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

impl FromValue for i64 {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Integer(value) => Some(*value),
            _ => None,
        }
    }
}

impl FromValue for Duration {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Duration(value) => Some(*value),
            _ => None,
        }
    }
}

impl FromValue for u64 {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Bytes(value) => Some(*value),
            Value::Integer(value) => u64::try_from(*value).ok(),
            _ => None,
        }
    }
}

/// Where an effective value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Default,
    File,
}

/// A fully validated configuration. Every declared field has a value.
#[derive(Clone, Debug)]
pub struct Config {
    scope: Scope,
    values: BTreeMap<&'static str, (Value, Source)>,
}

impl Config {
    #[must_use]
    pub fn defaults(scope: Scope) -> Self {
        Self {
            scope,
            values: scope
                .fields()
                .iter()
                .map(|field| (field.key, (field.default.clone(), Source::Default)))
                .collect(),
        }
    }

    #[must_use]
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// # Panics
    /// Only if a [`Key`] disagrees with its declared field, which the schema
    /// consistency test rules out.
    #[must_use]
    pub fn get<T: FromValue>(&self, key: &Key<T>) -> T {
        assert_eq!(
            key.scope, self.scope,
            "configuration key from another scope"
        );
        self.values
            .get(key.path)
            .and_then(|(value, _)| T::from_value(value))
            .unwrap_or_else(|| panic!("undeclared configuration key {}", key.path))
    }

    #[must_use]
    pub fn source(&self, path: &str) -> Option<Source> {
        self.values.get(path).map(|(_, source)| *source)
    }

    /// Declared fields in order with their effective value and source.
    pub fn entries(&self) -> impl Iterator<Item = (&'static Field, &Value, Source)> + '_ {
        self.scope.fields().iter().map(|field| {
            let (value, source) = &self.values[field.key];
            (field, value, *source)
        })
    }

    /// Fields whose effective values differ, with their reload class.
    #[must_use]
    pub fn changes(&self, next: &Self) -> Vec<Change> {
        self.entries()
            .filter_map(|(field, before, _)| {
                let after = &next.values[field.key].0;
                (before != after).then(|| Change {
                    key: field.key,
                    before: before.to_toml(),
                    after: after.to_toml(),
                    reload: field.reload,
                })
            })
            .collect()
    }

    /// The result of applying `next` to a running process: live fields take
    /// the new value, restart fields keep the value the process started with.
    #[must_use]
    pub fn with_live_fields_from(&self, next: &Self) -> Self {
        let mut merged = self.clone();
        for field in self.scope.fields() {
            if field.reload == Reload::Live {
                merged
                    .values
                    .insert(field.key, next.values[field.key].clone());
            }
        }
        merged
    }

    fn to_json(&self) -> serde_json::Value {
        let fields = self
            .entries()
            .map(|(field, value, source)| {
                serde_json::json!({
                    "key": field.key,
                    "value": value.to_json(),
                    "source": source,
                    "reload": field.reload,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({ "scope": self.scope, "fields": fields })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub key: &'static str,
    pub before: String,
    pub after: String,
    pub reload: Reload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

/// One machine-readable finding. Every field an agent needs to fix the file
/// without guessing: exact path and position, what was expected and found, and
/// a concrete hint.
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    /// Dotted key path, or empty for whole-file problems.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The outcome of checking one file.
#[derive(Debug)]
pub struct Report {
    pub scope: Scope,
    pub file: Option<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
    /// Present only when there are no errors.
    pub config: Option<Config>,
}

impl Report {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.config.is_some()
    }

    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let file = self
            .file
            .as_ref()
            .map_or_else(|| "<input>".to_owned(), |path| path.display().to_string());
        for diagnostic in &self.diagnostics {
            let severity = match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            let location = match (diagnostic.line, diagnostic.column) {
                (Some(line), Some(column)) => format!("{file}:{line}:{column}"),
                _ => file.clone(),
            };
            let path = if diagnostic.path.is_empty() {
                String::new()
            } else {
                format!(" {}", diagnostic.path)
            };
            let _ = writeln!(
                out,
                "{severity}[{}]:{path} ({location})\n  {}",
                diagnostic.code, diagnostic.message
            );
            if let Some(expected) = &diagnostic.expected {
                let _ = writeln!(out, "  expected: {expected}");
            }
            if let Some(found) = &diagnostic.found {
                let _ = writeln!(out, "  found: {found}");
            }
            if let Some(hint) = &diagnostic.hint {
                let _ = writeln!(out, "  hint: {hint}");
            }
        }
        let errors = self
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count();
        let warnings = self.diagnostics.len() - errors;
        let _ = writeln!(
            out,
            "{} {} configuration: {errors} error(s), {warnings} warning(s)",
            if errors == 0 { "valid" } else { "invalid" },
            self.scope.as_str()
        );
        out
    }

    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "scope": self.scope,
            "file": self.file,
            "valid": self.is_valid(),
            "diagnostics": self.diagnostics,
        })
    }
}

/// Parse and validate configuration text. Never fails: problems are reported.
#[must_use]
pub fn check_text(scope: Scope, text: &str, file: Option<PathBuf>) -> Report {
    let mut checker = Checker {
        scope,
        text,
        diagnostics: Vec::new(),
        values: BTreeMap::new(),
    };
    checker.run();
    let Checker {
        mut diagnostics,
        values,
        ..
    } = checker;
    diagnostics.sort_by_key(|diagnostic| (diagnostic.severity, diagnostic.line, diagnostic.column));
    let config = (!diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error))
    .then(|| {
        let mut config = Config::defaults(scope);
        for (key, value) in values {
            config.values.insert(key, (value, Source::File));
        }
        config
    });
    Report {
        scope,
        file,
        diagnostics,
        config,
    }
}

/// Check the file at `path`. A missing file is valid and yields all defaults.
#[must_use]
pub fn check_file(scope: Scope, path: &Path) -> Report {
    match read_file(path) {
        Ok(Some(text)) => check_text(scope, &text, Some(path.to_owned())),
        Ok(None) => Report {
            scope,
            file: Some(path.to_owned()),
            diagnostics: Vec::new(),
            config: Some(Config::defaults(scope)),
        },
        Err(diagnostic) => Report {
            scope,
            file: Some(path.to_owned()),
            diagnostics: vec![*diagnostic],
            config: None,
        },
    }
}

/// Load for a starting process: an invalid file refuses startup with the full
/// rendered report, so a bad edit can never silently fall back to defaults.
///
/// # Errors
/// When the file exists and is invalid.
pub fn load(scope: Scope, path: &Path) -> anyhow::Result<Config> {
    let report = check_file(scope, path);
    for diagnostic in &report.diagnostics {
        tracing::warn!(scope = scope.as_str(), path = %diagnostic.path, code = diagnostic.code,
            message = %diagnostic.message, "configuration warning");
    }
    let rendered = report.render_text();
    report.config.ok_or_else(|| {
        anyhow::anyhow!(
            "refusing to start with an invalid {} configuration; fix it and run `cowboy config check --scope {}`\n{rendered}",
            scope.as_str(),
            scope.as_str(),
        )
    })
}

fn read_file(path: &Path) -> Result<Option<String>, Box<Diagnostic>> {
    let whole_file = |code, message: String, hint: Option<&str>| {
        Box::new(Diagnostic {
            severity: Severity::Error,
            code,
            path: String::new(),
            line: None,
            column: None,
            message,
            expected: None,
            found: None,
            hint: hint.map(str::to_owned),
        })
    };
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(whole_file("unreadable", error.to_string(), None)),
    };
    if !metadata.is_file() {
        return Err(whole_file(
            "not-a-regular-file",
            "the configuration path is a symlink or not a regular file".to_owned(),
            Some("replace it with a regular file; `cowboy config apply` writes one atomically"),
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(whole_file(
            "file-too-large",
            format!("the file exceeds {MAX_FILE_BYTES} bytes"),
            None,
        ));
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|error| whole_file("unreadable", error.to_string(), None))
}

struct Checker<'a> {
    scope: Scope,
    text: &'a str,
    diagnostics: Vec<Diagnostic>,
    values: BTreeMap<&'static str, Value>,
}

impl Checker<'_> {
    fn run(&mut self) {
        let root = match toml::de::DeTable::parse(self.text) {
            Ok(root) => root,
            Err(error) => {
                let (line, column) = error
                    .span()
                    .map_or((None, None), |span| self.position(span.start));
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "syntax",
                    path: String::new(),
                    line,
                    column,
                    message: format!("not valid TOML: {}", error.message().trim()),
                    expected: None,
                    found: None,
                    hint: Some("fix the TOML syntax first; nothing else was checked".to_owned()),
                });
                return;
            }
        };
        let root = root.into_inner();
        let mut saw_schema = false;
        for (key, value) in &root {
            if key.get_ref().as_ref() == "schema" {
                saw_schema = true;
                self.check_schema(value);
            }
        }
        if !saw_schema {
            self.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "missing-schema",
                path: "schema".to_owned(),
                line: Some(1),
                column: Some(1),
                message: "the file must declare its schema version".to_owned(),
                expected: Some(format!("schema = {SCHEMA_VERSION}")),
                found: None,
                hint: Some(format!("add `schema = {SCHEMA_VERSION}` as the first line")),
            });
        }
        self.walk("", &root);
    }

    fn check_schema(&mut self, value: &toml::Spanned<toml::de::DeValue<'_>>) {
        let found = match value.get_ref() {
            toml::de::DeValue::Integer(integer) => {
                i64::from_str_radix(integer.as_str(), integer.radix()).ok()
            }
            _ => None,
        };
        if found != Some(SCHEMA_VERSION) {
            let (line, column) = self.position(value.span().start);
            self.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "unsupported-schema",
                path: "schema".to_owned(),
                line,
                column,
                message: "this Cowboy build does not understand the declared schema version"
                    .to_owned(),
                expected: Some(SCHEMA_VERSION.to_string()),
                found: Some(self.snippet(value.span())),
                hint: Some(
                    "a newer schema needs a newer Cowboy; an older one needs `cowboy config` from the release that wrote it"
                        .to_owned(),
                ),
            });
        }
    }

    fn walk(&mut self, prefix: &str, table: &toml::de::DeTable<'_>) {
        for (key, value) in table {
            let name = key.get_ref().as_ref();
            let path = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}.{name}")
            };
            if path == "schema" {
                continue;
            }
            if let Some(field) = self.scope.fields().iter().find(|field| field.key == path) {
                self.check_value(field, value);
                continue;
            }
            let is_section = self.scope.fields().iter().any(|field| {
                field
                    .key
                    .strip_prefix(path.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
            });
            match value.get_ref() {
                toml::de::DeValue::Table(inner) if is_section => self.walk(&path, inner),
                _ => self.unknown(&path, key.span()),
            }
        }
    }

    fn unknown(&mut self, path: &str, span: std::ops::Range<usize>) {
        let (line, column) = self.position(span.start);
        let known = self
            .scope
            .fields()
            .iter()
            .map(|field| field.key)
            .collect::<Vec<_>>();
        let suggestion = closest(path, &known);
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: "unknown-key",
            path: path.to_owned(),
            line,
            column,
            message: format!(
                "`{path}` is not a {} configuration setting",
                self.scope.as_str()
            ),
            expected: None,
            found: None,
            hint: Some(suggestion.map_or_else(
                || {
                    format!(
                        "run `cowboy config explain --scope {}` to list every setting",
                        self.scope.as_str()
                    )
                },
                |key| format!("did you mean `{key}`?"),
            )),
        });
    }

    fn check_value(&mut self, field: &'static Field, value: &toml::Spanned<toml::de::DeValue<'_>>) {
        let span = value.span();
        match parse_value(&field.kind, value.get_ref()) {
            Ok(parsed) => {
                self.values.insert(field.key, parsed);
            }
            Err(problem) => {
                let (line, column) = self.position(span.start);
                let (code, message) = match problem {
                    Problem::Type => ("wrong-type", format!("`{}` has the wrong type", field.key)),
                    Problem::Range => (
                        "out-of-range",
                        format!("`{}` is outside its allowed range", field.key),
                    ),
                    Problem::Syntax(detail) => {
                        ("invalid-value", format!("`{}`: {detail}", field.key))
                    }
                };
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code,
                    path: field.key.to_owned(),
                    line,
                    column,
                    message,
                    expected: Some(field.kind.describe()),
                    found: Some(self.snippet(span)),
                    hint: Some(format!(
                        "the default is {}; run `cowboy config explain --scope {} {}`",
                        field.default.to_toml(),
                        self.scope.as_str(),
                        field.key
                    )),
                });
            }
        }
    }

    fn position(&self, offset: usize) -> (Option<usize>, Option<usize>) {
        let before = &self.text[..offset.min(self.text.len())];
        let line = before.matches('\n').count() + 1;
        let column = before.rfind('\n').map_or(before.chars().count(), |index| {
            before[index + 1..].chars().count()
        }) + 1;
        (Some(line), Some(column))
    }

    fn snippet(&self, span: std::ops::Range<usize>) -> String {
        let text = self.text.get(span).unwrap_or_default().trim();
        if text.chars().count() > 80 {
            format!("{}…", text.chars().take(80).collect::<String>())
        } else {
            text.to_owned()
        }
    }
}

enum Problem {
    Type,
    Range,
    Syntax(String),
}

fn parse_value(kind: &Kind, value: &toml::de::DeValue<'_>) -> Result<Value, Problem> {
    use toml::de::DeValue;
    match (kind, value) {
        (Kind::Bool, DeValue::Boolean(value)) => Ok(Value::Bool(*value)),
        (Kind::Integer { min, max }, DeValue::Integer(integer)) => {
            let value = i64::from_str_radix(integer.as_str(), integer.radix())
                .map_err(|_| Problem::Range)?;
            if (*min..=*max).contains(&value) {
                Ok(Value::Integer(value))
            } else {
                Err(Problem::Range)
            }
        }
        (Kind::Duration { min, max }, DeValue::String(text)) => {
            let value = parse_duration(text).map_err(Problem::Syntax)?;
            if (*min..=*max).contains(&value) {
                Ok(Value::Duration(value))
            } else {
                Err(Problem::Range)
            }
        }
        (Kind::Bytes { min, max }, DeValue::String(text)) => {
            let value = parse_bytes(text).map_err(Problem::Syntax)?;
            if (*min..=*max).contains(&value) {
                Ok(Value::Bytes(value))
            } else {
                Err(Problem::Range)
            }
        }
        (Kind::Bytes { min, max }, DeValue::Integer(integer)) => {
            let value = u64::from_str_radix(integer.as_str(), integer.radix())
                .map_err(|_| Problem::Range)?;
            if (*min..=*max).contains(&value) {
                Ok(Value::Bytes(value))
            } else {
                Err(Problem::Range)
            }
        }
        (Kind::Choice(options), DeValue::String(text)) => options
            .iter()
            .find(|option| **option == text.as_ref())
            .map(|option| Value::Choice(option))
            .ok_or_else(|| {
                Problem::Syntax(format!(
                    "{:?} is not one of {}",
                    text.as_ref(),
                    quoted_list(options)
                ))
            }),
        _ => Err(Problem::Type),
    }
}

/// `"90s"`, `"10m"`, `"6h"`, `"7d"`; one unit, whole numbers.
///
/// # Errors
/// On anything else, with a message naming the accepted units.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let split = text
        .find(|character: char| !character.is_ascii_digit())
        .ok_or_else(|| format!("{text:?} has no unit; use s, m, h or d, for example \"6h\""))?;
    let (number, unit) = text.split_at(split);
    let number: u64 = number
        .parse()
        .map_err(|_| format!("{text:?} must start with a whole number"))?;
    let seconds = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => {
            return Err(format!(
                "{text:?} has unknown unit {unit:?}; use s, m, h or d"
            ));
        }
    };
    number
        .checked_mul(seconds)
        .map(Duration::from_secs)
        .ok_or_else(|| format!("{text:?} is too large"))
}

/// `"512MiB"`, `"1.5GiB"`, `"4096B"`; binary units only, to avoid GB/GiB doubt.
///
/// # Errors
/// On anything else, with a message naming the accepted units.
pub fn parse_bytes(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let split = text
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .ok_or_else(|| format!("{text:?} has no unit; use B, KiB, MiB, GiB or TiB"))?;
    let (number, unit) = text.split_at(split);
    let multiplier: u64 = match unit.trim() {
        "B" => 1,
        "KiB" => 1 << 10,
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        "TiB" => 1 << 40,
        other => {
            return Err(format!(
                "{text:?} has unknown unit {other:?}; use B, KiB, MiB, GiB or TiB (binary units)"
            ));
        }
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let whole: u64 = whole
        .parse()
        .map_err(|_| format!("{text:?} must start with a number"))?;
    if fraction.len() > 3 || !fraction.chars().all(|character| character.is_ascii_digit()) {
        return Err(format!("{text:?} allows at most three decimal places"));
    }
    let scale = 10_u64.pow(u32::try_from(fraction.len()).unwrap_or(0));
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        fraction.parse().unwrap_or(0)
    };
    whole
        .checked_mul(multiplier)
        .and_then(|bytes| bytes.checked_add(fraction.checked_mul(multiplier)? / scale))
        .ok_or_else(|| format!("{text:?} is too large"))
}

#[must_use]
pub fn format_duration(value: Duration) -> String {
    let seconds = value.as_secs();
    for (unit, size) in [("d", 86_400), ("h", 3600), ("m", 60)] {
        if seconds >= size && seconds.is_multiple_of(size) {
            return format!("{}{unit}", seconds / size);
        }
    }
    format!("{seconds}s")
}

#[must_use]
pub fn format_bytes(value: u64) -> String {
    for (unit, size) in [
        ("TiB", 1_u64 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
    ] {
        if value >= size && value.is_multiple_of(size) {
            return format!("{}{unit}", value / size);
        }
    }
    format!("{value}B")
}

fn quoted_list(options: &[&str]) -> String {
    options
        .iter()
        .map(|option| format!("\"{option}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The closest declared key within a small edit distance, compared both as a
/// full path and by its last segment so `capacity.max_sesions` and
/// `max_sessions` both find `capacity.max_sessions`.
fn closest<'a>(input: &str, known: &[&'a str]) -> Option<&'a str> {
    let leaf = input.rsplit('.').next().unwrap_or(input);
    known
        .iter()
        .map(|candidate| {
            let candidate_leaf = candidate.rsplit('.').next().unwrap_or(candidate);
            let distance = edit_distance(input, candidate).min(edit_distance(leaf, candidate_leaf));
            (distance, *candidate)
        })
        .filter(|(distance, candidate)| *distance <= 2.max(candidate.len() / 5))
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = vec![row + 1; right.len() + 1];
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        previous = current;
    }
    previous[right.len()]
}

/// Commented template listing every setting at its default.
#[must_use]
pub fn example(scope: Scope) -> String {
    let mut out = format!(
        "# Cowboy {} configuration (`{}`).\n# Every setting is optional; the values below are the defaults.\n# Validate edits with `cowboy config check --scope {}`; see docs/configuration.md.\nschema = {SCHEMA_VERSION}\n",
        scope.as_str(),
        scope.file_name(),
        scope.as_str()
    );
    let mut section = "";
    for field in scope.fields() {
        let (table, name) = field.key.rsplit_once('.').unwrap_or(("", field.key));
        if table != section {
            section = table;
            let _ = write!(out, "\n[{table}]\n");
        }
        for line in field.doc.lines() {
            let _ = writeln!(out, "# {line}");
        }
        let reload = match field.reload {
            Reload::Live => "applies live",
            Reload::Restart => "needs a restart",
        };
        let _ = writeln!(out, "# {}; {reload}.", field.kind.describe());
        let _ = writeln!(out, "# {name} = {}", field.default.to_toml());
    }
    out
}

/// JSON Schema-style description for agents and editors.
#[must_use]
pub fn json_schema(scope: Scope) -> serde_json::Value {
    let fields = scope
        .fields()
        .iter()
        .map(|field| {
            let mut entry = serde_json::json!({
                "key": field.key,
                "type": field.kind.json_type(),
                "accepts": field.kind.describe(),
                "default": field.default.to_json(),
                "reload": field.reload,
                "description": field.doc,
            });
            if let Kind::Choice(options) = field.kind {
                entry["enum"] = serde_json::json!(options);
            }
            entry
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "scope": scope,
        "file": scope.file_name(),
        "schema_version": SCHEMA_VERSION,
        "fields": fields,
    })
}

/// Atomically install validated text as the configuration, keeping the
/// previous file in `config/history/`. Refuses invalid text.
///
/// # Errors
/// When the text is invalid or the filesystem write fails.
pub fn install(scope: Scope, path: &Path, text: &str) -> anyhow::Result<Report> {
    let report = check_text(scope, text, Some(path.to_owned()));
    anyhow::ensure!(
        report.is_valid(),
        "refusing to install an invalid configuration\n{}",
        report.render_text()
    );
    let directory = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("configuration path has no directory"))?;
    create_private_dir(directory)?;
    if let Ok(Some(previous)) = read_file(path) {
        let history = directory.join("history");
        create_private_dir(&history)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        write_private(
            &history.join(format!("{stamp}-{}", scope.file_name())),
            previous.as_bytes(),
        )?;
        prune_history(&history, scope)?;
    }
    let temporary = directory.join(format!(".{}.{}.tmp", scope.file_name(), std::process::id()));
    write_private(&temporary, text.as_bytes())?;
    std::fs::rename(&temporary, path)?;
    Ok(report)
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn prune_history(history: &Path, scope: Scope) -> std::io::Result<()> {
    let mut entries = std::fs::read_dir(history)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(scope.file_name()))
        })
        .collect::<Vec<_>>();
    entries.sort();
    let excess = entries.len().saturating_sub(HISTORY_LIMIT);
    for path in entries.into_iter().take(excess) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// The running process's view of its configuration. Live fields follow valid
/// file edits; restart fields keep their startup values; an invalid edit is
/// logged and ignored so a typo never changes running behaviour.
#[derive(Clone)]
pub struct Handle {
    current: Arc<parking_lot::RwLock<Arc<Config>>>,
}

impl Handle {
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            current: Arc::new(parking_lot::RwLock::new(Arc::new(config))),
        }
    }

    #[must_use]
    pub fn current(&self) -> Arc<Config> {
        Arc::clone(&self.current.read())
    }

    #[must_use]
    pub fn get<T: FromValue>(&self, key: &Key<T>) -> T {
        self.current.read().get(key)
    }

    /// Apply a newly read file's report. Returns the changes that were applied
    /// live and those that wait for a restart.
    pub fn reload(&self, report: &Report) -> Option<(Vec<Change>, Vec<Change>)> {
        let Some(next) = &report.config else {
            tracing::error!(
                scope = report.scope.as_str(),
                diagnostics = %report.render_text(),
                "ignoring invalid configuration edit; the running values are unchanged"
            );
            return None;
        };
        let mut guard = self.current.write();
        let changes = guard.changes(next);
        let (live, restart): (Vec<_>, Vec<_>) = changes
            .into_iter()
            .partition(|change| change.reload == Reload::Live);
        *guard = Arc::new(guard.with_live_fields_from(next));
        drop(guard);
        for change in &live {
            tracing::info!(scope = report.scope.as_str(), key = change.key,
                before = %change.before, after = %change.after, "configuration applied live");
        }
        for change in &restart {
            tracing::warn!(scope = report.scope.as_str(), key = change.key,
                after = %change.after, "configuration change waits for the next restart");
        }
        Some((live, restart))
    }

    /// Poll the file and apply valid changes. Polling, not inotify, keeps one
    /// behaviour on Linux and macOS Devices and survives atomic renames.
    #[must_use]
    pub fn watch(&self, scope: Scope, path: PathBuf) -> tokio::task::JoinHandle<()> {
        let handle = self.clone();
        tokio::spawn(async move {
            let mut last = fingerprint(&path);
            let mut tick = tokio::time::interval(RELOAD_POLL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let now = fingerprint(&path);
                if now == last {
                    continue;
                }
                last = now;
                let report = check_file(scope, &path);
                handle.reload(&report);
            }
        })
    }
}

fn fingerprint(path: &Path) -> Option<(u64, u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::symlink_metadata(path).ok().map(|metadata| {
        (
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
        )
    })
}

#[cfg(test)]
mod tests;

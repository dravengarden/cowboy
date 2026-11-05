//! Project checks (`check@1`): declared commands and their parsed results.
//!
//! A worktree declares its checks in `.cowboy/checks.json`. Running one runs
//! project code, so it is always an explicit request under the owner's
//! "Project language servers" grant, in the session's own execution
//! environment; this module only validates declarations and parses output.
//! A parsed result is evidence for the inputs the command saw, not proof that
//! the worktree is correct.

use serde::{Deserialize, Serialize};

pub const CONFIG_PATH: &str = ".cowboy/checks.json";
pub const MAX_CHECKS: usize = 8;
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 600;
pub const MAX_TIMEOUT_SECONDS: u64 = 1_800;
pub const MAX_DIAGNOSTICS: usize = 1_000;
const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_BYTES: usize = 4_096;
const MAX_MESSAGE_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// `cargo … --message-format=json` compiler messages.
    CargoJson,
    /// `tsc --pretty false`: `file(line,col): error TS1234: message`.
    Tsc,
    /// Generic `file:line[:col]: severity: message` lines (gcc, go vet, eslint unix).
    Lines,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Check {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub argv: Vec<String>,
    pub format: Format,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

impl Check {
    #[must_use]
    pub fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.timeout_seconds
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
                .clamp(1, MAX_TIMEOUT_SECONDS),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub schema_version: u8,
    pub checks: Vec<Check>,
}

/// # Errors
/// Malformed JSON, an unknown schema or an invalid declaration.
pub fn parse_config(bytes: &[u8]) -> Result<Config, String> {
    let config: Config =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid {CONFIG_PATH}: {error}"))?;
    if config.schema_version != 1 {
        return Err(format!("{CONFIG_PATH}: unsupported schemaVersion"));
    }
    if config.checks.is_empty() || config.checks.len() > MAX_CHECKS {
        return Err(format!("{CONFIG_PATH}: declare 1..={MAX_CHECKS} checks"));
    }
    let mut seen = std::collections::HashSet::new();
    for check in &config.checks {
        let id_ok = !check.id.is_empty()
            && check.id.len() <= 32
            && check
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !id_ok || !seen.insert(check.id.as_str()) {
            return Err(format!(
                "{CONFIG_PATH}: check ids must be unique [a-z0-9-]{{1,32}}"
            ));
        }
        if check.argv.is_empty()
            || check.argv.len() > MAX_ARGUMENTS
            || check.argv.iter().any(|argument| {
                argument.is_empty()
                    || argument.len() > MAX_ARGUMENT_BYTES
                    || argument.contains('\0')
            })
        {
            return Err(format!("{CONFIG_PATH}: {}: invalid argv", check.id));
        }
        if check.title.as_ref().is_some_and(|title| title.len() > 200) {
            return Err(format!("{CONFIG_PATH}: {}: title is too long", check.id));
        }
    }
    Ok(config)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Worktree-relative when the tool reported a path inside the worktree.
    pub path: String,
    /// 1-based.
    pub line: u32,
    /// 1-based; 0 when the tool reported no column.
    pub column: u32,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parsed {
    pub diagnostics: Vec<Diagnostic>,
    /// More diagnostics were reported than are retained.
    pub truncated: bool,
}

fn relative(root: &str, path: &str) -> String {
    let root = root.trim_end_matches('/');
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .trim_start_matches("./")
        .to_owned()
}

fn message(text: &str) -> String {
    text.trim().chars().take(MAX_MESSAGE_CHARS).collect()
}

fn push(parsed: &mut Parsed, diagnostic: Diagnostic) {
    if parsed.diagnostics.len() < MAX_DIAGNOSTICS {
        parsed.diagnostics.push(diagnostic);
    } else {
        parsed.truncated = true;
    }
}

fn severity(word: &str) -> Option<Severity> {
    match word.to_ascii_lowercase().as_str() {
        "error" | "fatal error" | "fatal" => Some(Severity::Error),
        "warning" | "warn" => Some(Severity::Warning),
        "note" | "info" | "help" | "hint" => Some(Severity::Note),
        _ => None,
    }
}

fn cargo_json(stdout: &str, root: &str, parsed: &mut Parsed) {
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value["reason"] != "compiler-message" {
            continue;
        }
        let message_value = &value["message"];
        let Some(level) = message_value["level"].as_str().and_then(severity) else {
            continue;
        };
        let Some(span) = message_value["spans"]
            .as_array()
            .and_then(|spans| spans.iter().find(|span| span["is_primary"] == true))
        else {
            continue;
        };
        let (Some(file), Some(line), Some(column)) = (
            span["file_name"].as_str(),
            span["line_start"].as_u64(),
            span["column_start"].as_u64(),
        ) else {
            continue;
        };
        push(
            parsed,
            Diagnostic {
                path: relative(root, file),
                line: u32::try_from(line).unwrap_or(u32::MAX),
                column: u32::try_from(column).unwrap_or(u32::MAX),
                severity: level,
                code: message_value["code"]["code"].as_str().map(str::to_owned),
                message: message(message_value["message"].as_str().unwrap_or_default()),
            },
        );
    }
}

fn tsc(output: &str, root: &str, parsed: &mut Parsed) {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(.+?)\((\d+),(\d+)\): (error|warning|message) (TS\d+): (.*)$")
            .expect("valid tsc pattern")
    });
    for line in output.lines() {
        let Some(captures) = PATTERN.captures(line.trim_end()) else {
            continue;
        };
        push(
            parsed,
            Diagnostic {
                path: relative(root, &captures[1]),
                line: captures[2].parse().unwrap_or(0),
                column: captures[3].parse().unwrap_or(0),
                severity: severity(&captures[4]).unwrap_or(Severity::Note),
                code: Some(captures[5].to_owned()),
                message: message(&captures[6]),
            },
        );
    }
}

fn lines(output: &str, root: &str, parsed: &mut Parsed) {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"^([^\s:][^:]*):(\d+)(?::(\d+))?:\s*(?:(fatal error|error|warning|note|info|help)\s*:\s*)?(.*)$",
        )
        .expect("valid lines pattern")
    });
    for line in output.lines() {
        let Some(captures) = PATTERN.captures(line.trim_end()) else {
            continue;
        };
        push(
            parsed,
            Diagnostic {
                path: relative(root, &captures[1]),
                line: captures[2].parse().unwrap_or(0),
                column: captures
                    .get(3)
                    .and_then(|column| column.as_str().parse().ok())
                    .unwrap_or(0),
                severity: captures
                    .get(4)
                    .and_then(|word| severity(word.as_str()))
                    .unwrap_or(Severity::Error),
                code: None,
                message: message(&captures[5]),
            },
        );
    }
}

/// Parse a finished run. `root` is the worktree path the tool saw.
#[must_use]
pub fn parse(format: Format, stdout: &str, stderr: &str, root: &str) -> Parsed {
    let mut parsed = Parsed::default();
    match format {
        Format::CargoJson => cargo_json(stdout, root, &mut parsed),
        Format::Tsc => tsc(&format!("{stdout}\n{stderr}"), root, &mut parsed),
        Format::Lines => lines(&format!("{stdout}\n{stderr}"), root, &mut parsed),
    }
    parsed
}

#[cfg(test)]
mod tests;

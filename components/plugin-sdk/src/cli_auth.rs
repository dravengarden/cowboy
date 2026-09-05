//! Data-only CLI authentication rules embedded in the Plugin host contract.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

const MAX_RULES: usize = 64;
const MAX_CONDITIONS: usize = 32;
const MAX_DEPTH: usize = 16;
const MAX_TEXT: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliAuthRuleSet {
    pub rules: Vec<CliAuthRule>,
    pub fallback: CliAuthOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliAuthRule {
    pub when: CliAuthCondition,
    pub outcome: CliAuthOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliAuthOutcome {
    pub state: CliAuthProbeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CliAuthProbeState {
    SignedIn,
    SignedOut,
    Error,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CliAuthCondition {
    Nonempty {
        source: CliAuthSource,
    },
    Equals {
        source: CliAuthSource,
        value: String,
        #[serde(default)]
        ignore_case: bool,
    },
    Exists {
        source: CliAuthSource,
    },
    JsonString {
        source: CliAuthSource,
        pointer: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(default)]
        ignore_case: bool,
    },
    JsonObject {
        source: CliAuthSource,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        nonempty_string_fields: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        string_fields: Vec<String>,
        #[serde(default)]
        root_or_values: bool,
    },
    All {
        values: Vec<CliAuthCondition>,
    },
    Any {
        values: Vec<CliAuthCondition>,
    },
    Not {
        value: Box<CliAuthCondition>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CliAuthSource {
    Env { name: String },
    File { path: String },
    Dotenv { path: String, key: String },
}

impl CliAuthRuleSet {
    /// Validate the complete bounded rule tree without reading host state.
    ///
    /// # Errors
    /// Returns when a rule, outcome, source, or nested condition is outside
    /// the closed host contract.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.rules.is_empty(),
            "cli_auth_rules needs at least one rule"
        );
        ensure!(self.rules.len() <= MAX_RULES, "too many cli auth rules");
        validate_outcome(&self.fallback)?;
        for rule in &self.rules {
            validate_condition(&rule.when, 0)?;
            validate_outcome(&rule.outcome)?;
        }
        Ok(())
    }
}

fn validate_outcome(outcome: &CliAuthOutcome) -> Result<()> {
    if let Some(detail) = &outcome.detail {
        ensure!(!detail.is_empty(), "cli auth outcome detail is empty");
        ensure!(
            detail.len() <= MAX_TEXT,
            "cli auth outcome detail is too long"
        );
        ensure!(
            !detail.contains('\0'),
            "cli auth outcome detail contains NUL"
        );
    }
    Ok(())
}

fn validate_condition(condition: &CliAuthCondition, depth: usize) -> Result<()> {
    ensure!(
        depth <= MAX_DEPTH,
        "cli auth condition is too deeply nested"
    );
    match condition {
        CliAuthCondition::Nonempty { source } | CliAuthCondition::Exists { source } => {
            validate_source(source)
        }
        CliAuthCondition::Equals { source, value, .. } => {
            validate_source(source)?;
            validate_text(value, "cli auth comparison")
        }
        CliAuthCondition::JsonString {
            source,
            pointer,
            value,
            ..
        } => {
            validate_source(source)?;
            ensure!(
                pointer.starts_with('/'),
                "cli auth JSON pointer must start with /"
            );
            validate_text(pointer, "cli auth JSON pointer")?;
            if let Some(value) = value {
                validate_text(value, "cli auth JSON value")?;
            }
            Ok(())
        }
        CliAuthCondition::JsonObject {
            source,
            nonempty_string_fields,
            string_fields,
            ..
        } => {
            validate_source(source)?;
            ensure!(
                !nonempty_string_fields.is_empty() || !string_fields.is_empty(),
                "cli auth JSON object condition needs fields"
            );
            ensure!(
                nonempty_string_fields.len() + string_fields.len() <= MAX_CONDITIONS,
                "too many cli auth JSON fields"
            );
            let mut seen = BTreeSet::new();
            for field in nonempty_string_fields.iter().chain(string_fields) {
                validate_key(field, "cli auth JSON field")?;
                ensure!(seen.insert(field), "duplicate cli auth JSON field");
            }
            Ok(())
        }
        CliAuthCondition::All { values } | CliAuthCondition::Any { values } => {
            ensure!(!values.is_empty(), "cli auth boolean condition is empty");
            ensure!(
                values.len() <= MAX_CONDITIONS,
                "too many cli auth conditions"
            );
            for value in values {
                validate_condition(value, depth + 1)?;
            }
            Ok(())
        }
        CliAuthCondition::Not { value } => validate_condition(value, depth + 1),
    }
}

fn validate_source(source: &CliAuthSource) -> Result<()> {
    match source {
        CliAuthSource::Env { name } => validate_env_name(name),
        CliAuthSource::File { path } => validate_path_template(path),
        CliAuthSource::Dotenv { path, key } => {
            validate_path_template(path)?;
            validate_env_name(key)
        }
    }
}

fn validate_env_name(value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "cli auth environment name is empty");
    ensure!(value.len() <= 128, "cli auth environment name is too long");
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'),
        "invalid cli auth environment name"
    );
    Ok(())
}

fn validate_path_template(value: &str) -> Result<()> {
    validate_text(value, "cli auth file path")?;
    ensure!(
        value.starts_with("${"),
        "cli auth file path must start with an environment variable"
    );
    let Some((name, suffix)) = value[2..].split_once('}') else {
        anyhow::bail!("invalid cli auth file path template");
    };
    validate_env_name(name)?;
    ensure!(
        !suffix.contains(".."),
        "cli auth file path cannot traverse parents"
    );
    Ok(())
}

fn validate_key(value: &str, label: &str) -> Result<()> {
    validate_text(value, label)?;
    ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'),
        "invalid {label}"
    );
    Ok(())
}

fn validate_text(value: &str, label: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{label} is empty");
    ensure!(value.len() <= MAX_TEXT, "{label} is too long");
    ensure!(!value.contains('\0'), "{label} contains NUL");
    Ok(())
}

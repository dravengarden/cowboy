//! Machine evaluation for data-only CLI authentication probe contracts.

#![warn(clippy::pedantic)]

#[cfg(feature = "machine-host")]
use std::path::PathBuf;

#[cfg(all(test, feature = "machine-host"))]
pub(crate) use cowboy_plugin_sdk::CliAuthRule;
#[cfg(feature = "machine-host")]
pub(crate) use cowboy_plugin_sdk::{
    CliAuthCondition, CliAuthOutcome, CliAuthProbeState, CliAuthRuleSet, CliAuthSource,
};

#[cfg(feature = "machine-host")]
pub(crate) fn evaluate(rules: &CliAuthRuleSet) -> CliAuthOutcome {
    rules
        .rules
        .iter()
        .find(|rule| evaluate_condition(&rule.when))
        .map_or_else(|| rules.fallback.clone(), |rule| rule.outcome.clone())
}

#[cfg(feature = "machine-host")]
fn evaluate_condition(condition: &CliAuthCondition) -> bool {
    match condition {
        CliAuthCondition::Nonempty { source } => {
            read_source(source).is_some_and(|value| !value.trim().is_empty())
        }
        CliAuthCondition::Equals {
            source,
            value,
            ignore_case,
        } => read_source(source).is_some_and(|candidate| {
            let candidate = candidate.trim();
            if *ignore_case {
                candidate.eq_ignore_ascii_case(value)
            } else {
                candidate == value
            }
        }),
        CliAuthCondition::Exists { source } => source_exists(source),
        CliAuthCondition::JsonString {
            source,
            pointer,
            value,
            ignore_case,
        } => read_source_json(source)
            .and_then(|document| document.pointer(pointer).cloned())
            .and_then(|candidate| candidate.as_str().map(str::to_owned))
            .is_some_and(|candidate| {
                value.as_ref().is_none_or(|expected| {
                    if *ignore_case {
                        candidate.eq_ignore_ascii_case(expected)
                    } else {
                        candidate == *expected
                    }
                })
            }),
        CliAuthCondition::JsonObject {
            source,
            nonempty_string_fields,
            string_fields,
            root_or_values,
        } => read_source_json(source).is_some_and(|document| {
            json_object_matches(
                &document,
                nonempty_string_fields,
                string_fields,
                *root_or_values,
            )
        }),
        CliAuthCondition::All { values } => values.iter().all(evaluate_condition),
        CliAuthCondition::Any { values } => values.iter().any(evaluate_condition),
        CliAuthCondition::Not { value } => !evaluate_condition(value),
    }
}

#[cfg(feature = "machine-host")]
fn json_object_matches(
    document: &serde_json::Value,
    nonempty_string_fields: &[String],
    string_fields: &[String],
    root_or_values: bool,
) -> bool {
    let matches = |candidate: &serde_json::Value| {
        let Some(object) = candidate.as_object() else {
            return false;
        };
        nonempty_string_fields.iter().all(|field| {
            object
                .get(field)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        }) && string_fields
            .iter()
            .all(|field| object.get(field).is_some_and(serde_json::Value::is_string))
    };
    matches(document)
        || (root_or_values
            && document
                .as_object()
                .is_some_and(|object| object.values().any(matches)))
}

#[cfg(feature = "machine-host")]
fn read_source(source: &CliAuthSource) -> Option<String> {
    match source {
        CliAuthSource::Env { name } => std::env::var(name).ok(),
        CliAuthSource::File { path } => std::fs::read_to_string(expand_path(path)?).ok(),
        CliAuthSource::Dotenv { path, key } => {
            let contents = std::fs::read_to_string(expand_path(path)?).ok()?;
            dotenv_value(&contents, key)
        }
    }
}

#[cfg(feature = "machine-host")]
fn read_source_json(source: &CliAuthSource) -> Option<serde_json::Value> {
    serde_json::from_str(&read_source(source)?).ok()
}

#[cfg(feature = "machine-host")]
fn source_exists(source: &CliAuthSource) -> bool {
    match source {
        CliAuthSource::Env { name } => std::env::var_os(name).is_some(),
        CliAuthSource::File { path } => expand_path(path).is_some_and(|path| path.is_file()),
        CliAuthSource::Dotenv { .. } => read_source(source).is_some(),
    }
}

#[cfg(feature = "machine-host")]
fn expand_path(template: &str) -> Option<PathBuf> {
    let template = template.strip_prefix("${")?;
    let (name, suffix) = template.split_once('}')?;
    let mut path = PathBuf::from(std::env::var_os(name)?);
    let suffix = suffix.strip_prefix('/').unwrap_or(suffix);
    if !suffix.is_empty() {
        path.push(suffix);
    }
    Some(path)
}

#[cfg(feature = "machine-host")]
pub(crate) fn dotenv_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (candidate, value) = line.split_once('=')?;
        (candidate.trim() == key).then(|| {
            value
                .trim()
                .trim_matches(|character| character == '\'' || character == '"')
                .to_owned()
        })
    })
}

#[cfg(all(test, feature = "machine-host"))]
mod tests {
    use super::*;

    #[test]
    fn nested_json_credentials_are_shape_driven() {
        let document = serde_json::json!({
            "profile": { "key": "secret", "mode": "oauth" }
        });
        assert!(json_object_matches(
            &document,
            &["key".to_owned()],
            &["mode".to_owned()],
            true,
        ));
        assert!(!json_object_matches(
            &serde_json::json!({ "profile": { "key": "" } }),
            &["key".to_owned()],
            &["mode".to_owned()],
            true,
        ));
    }

    #[test]
    fn rule_order_selects_the_first_matching_outcome() {
        let rules = CliAuthRuleSet {
            rules: vec![CliAuthRule {
                when: CliAuthCondition::Not {
                    value: Box::new(CliAuthCondition::Nonempty {
                        source: CliAuthSource::Env {
                            name: "COWBOY_TEST_PLUGIN_AUTH_MISSING".to_owned(),
                        },
                    }),
                },
                outcome: CliAuthOutcome {
                    state: CliAuthProbeState::SignedOut,
                    detail: Some("configure the plugin".to_owned()),
                },
            }],
            fallback: CliAuthOutcome {
                state: CliAuthProbeState::Unsupported,
                detail: None,
            },
        };
        rules.validate().unwrap();
        assert_eq!(evaluate(&rules).state, CliAuthProbeState::SignedOut);
    }
}

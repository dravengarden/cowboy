//! `code.post_edit`: the bounded diagnostics summary a Provider delivers to
//! its model after an edit. It is the explicit `code.diagnostics` read of the
//! edited files (acquisition, agent-owned refresh and read baselines), reduced
//! to what changed. Repository text stays data: the delivering adapter must
//! present every string as untrusted.
//!
//! `deliver` is true only when there is something to say: new or escalated
//! diagnostics against a content-matched baseline, or current errors when no
//! baseline exists. Silence is never a claim that the edit is clean.

use serde_json::{Value, json};

pub(super) const MAX_PATHS: usize = 5;
const MAX_ERRORS: usize = 10;
const MAX_WARNINGS: usize = 5;
const MAX_MESSAGE_CHARS: usize = 200;
const MAX_TOTAL_CHARS: usize = 2000;

struct Budget {
    errors: usize,
    warnings: usize,
    chars: usize,
    truncated: bool,
}

impl Budget {
    /// Admit one diagnostic within the per-severity and text bounds.
    fn admit(&mut self, diagnostic: &Value) -> Option<Value> {
        let slot = match diagnostic["severity"].as_str() {
            Some("error") => &mut self.errors,
            Some("warning") => &mut self.warnings,
            _ => return None,
        };
        let message: String = diagnostic["message"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(MAX_MESSAGE_CHARS)
            .collect();
        let length = message.chars().count();
        if *slot == 0 || self.chars + length > MAX_TOTAL_CHARS {
            self.truncated = true;
            return None;
        }
        *slot -= 1;
        self.chars += length;
        Some(json!({
            "severity": diagnostic["severity"],
            "line": diagnostic["line"],
            "column": diagnostic["column"],
            "source": diagnostic["source"],
            "code": diagnostic["code"],
            "message": message,
        }))
    }
}

fn counted(comparison: &Value, key: &str) -> u64 {
    comparison[key].as_u64().unwrap_or(0)
}

/// Diagnostics a comparison names as new: the listed candidates, matched back
/// to the observation by position and identity to recover their messages.
/// An ambiguous candidate shares its identity with a pre-existing instance.
fn introduced(file: &Value, comparison: &Value, budget: &mut Budget) -> Vec<Value> {
    let diagnostics = file["diagnostics"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let mut listed = Vec::new();
    for candidate in comparison["candidates"].as_array().into_iter().flatten() {
        let found = diagnostics.iter().find(|diagnostic| {
            ["line", "column", "code", "source", "severity"]
                .iter()
                .all(|key| diagnostic[*key] == candidate[*key])
        });
        // Beyond the read's per-file list: counted, not shown.
        let Some(found) = found else {
            budget.truncated = true;
            continue;
        };
        if let Some(mut value) = budget.admit(found) {
            value["ambiguous"] = candidate["ambiguous"].clone();
            listed.push(value);
        }
    }
    listed
}

pub(super) fn project(observation: &Value) -> Value {
    let mut budget = Budget {
        errors: MAX_ERRORS,
        warnings: MAX_WARNINGS,
        chars: 0,
        truncated: false,
    };
    let mut deliver = false;
    let mut files = Vec::new();
    for file in observation["files"].as_array().into_iter().flatten() {
        let mut item = json!({"path":file["path"],"state":file["state"]});
        let comparison = &file["comparison"];
        let compared = matches!(comparison["state"].as_str(), Some("complete" | "partial"));
        let listed: Vec<Value> = if compared {
            let changed = counted(comparison, "introduced") + counted(comparison, "escalated");
            item["comparison"] = json!({
                "state": comparison["state"],
                "introduced": counted(comparison, "introduced"),
                "resolved": counted(comparison, "resolved"),
                "escalated": counted(comparison, "escalated"),
            });
            deliver |= changed > 0;
            introduced(file, comparison, &mut budget)
        } else {
            if comparison.is_object() {
                item["comparison"] = json!({"state":"unknown","reason":comparison["reason"]});
            }
            let errors: Vec<Value> = if file["state"] == "document_current" {
                file["diagnostics"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|value| value["severity"] == "error")
                    .filter_map(|value| budget.admit(value))
                    .collect()
            } else {
                Vec::new()
            };
            deliver |= !errors.is_empty();
            errors
        };
        if !listed.is_empty() {
            item["diagnostics"] = Value::Array(listed);
        }
        files.push(item);
    }
    json!({"deliver":deliver,"files":files,"truncated":budget.truncated})
}

#[cfg(test)]
mod tests;

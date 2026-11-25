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

/// Whether the analyzed text is exactly the last executor write the
/// Controller observed for that file. An observation is a hint: no turn or
/// tool identity, and other writers are not seen.
pub(super) fn attach_writes(
    projected: &mut Value,
    observation: &Value,
    latest: impl Fn(&str) -> Option<String>,
) {
    let analyzed: Vec<(Option<&str>, Option<&str>)> = observation["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|file| {
            (
                file["path"].as_str(),
                file["bufferSnapshot"]["content"]["sha256"].as_str(),
            )
        })
        .collect();
    let Some(files) = projected["files"].as_array_mut() else {
        return;
    };
    for (item, (path, sha256)) in files.iter_mut().zip(analyzed) {
        let Some(written) = path.and_then(&latest) else {
            continue;
        };
        item["write"] =
            json!({"observed":true,"matches_analyzed":sha256 == Some(written.as_str())});
    }
}

/// Project a single observed file for the R4 `/api/machine/post-edit-inspect`
/// response. Enforces:
/// - `matches_expected`: exact match against the caller's LF-normalized SHA-256.
/// - `current_errors` fallback ONLY for `no_baseline` or `buffer_replaced` on
///   `document_current` (never `stale_observation`).
/// - Strict 12 KiB serialized payload cap via tail-trimming, recomputing
///   `deliver` AFTER trimming (`R2-M12`).
pub(super) fn project_for_inspect(
    file: &Value,
    expected_path: &str,
    expected_lf_sha256: &str,
) -> (Value, bool) {
    let path_ok = file["path"].as_str() == Some(expected_path);
    let is_current = file["state"].as_str() == Some("document_current");
    let comparison = &file["comparison"];
    let reason = comparison["reason"].as_str();
    let ordering_admissible = !matches!(
        reason,
        Some("out_of_order_observation" | "unorderable_observation" | "stale_observation")
    );
    let analyzed_sha = file
        .pointer("/bufferSnapshot/content/sha256")
        .and_then(Value::as_str);
    let matches_expected =
        path_ok && is_current && ordering_admissible && analyzed_sha == Some(expected_lf_sha256);
    if !matches_expected {
        // H-Ctrl-1: Never disclose diagnostics, counts, or file metadata on mismatch or stale/unorderable observations.
        return (
            json!({
                "deliver": false,
                "files": [],
                "truncated": false,
            }),
            false,
        );
    }

    let mut budget = Budget {
        errors: MAX_ERRORS,
        warnings: MAX_WARNINGS,
        chars: 0,
        truncated: false,
    };
    let mut item = json!({
        "path": expected_path,
        "state": "document_current",
    });
    let compared = matches!(comparison["state"].as_str(), Some("complete" | "partial"));
    let mut current_errors: u64 = 0;
    let mut listed: Vec<Value> = if compared {
        item["comparison"] = json!({
            "state": comparison["state"],
            "introduced": counted(comparison, "introduced"),
            "resolved": counted(comparison, "resolved"),
            "escalated": counted(comparison, "escalated"),
        });
        introduced(file, comparison, &mut budget)
    } else {
        if comparison.is_object() {
            item["comparison"] = json!({"state": "unknown", "reason": comparison["reason"]});
        }
        // M14: Stale and unorderable comparisons were already rejected by
        // `ordering_admissible` above. Apply the current-error fallback to all
        // remaining admissible unknown comparisons (including `servers_changed`
        // and `over_capture_bound`).
        let error_diags: Vec<&Value> = file["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|value| value["severity"] == "error")
            .collect();
        current_errors = file
            .pointer("/counts/error")
            .and_then(Value::as_u64)
            .unwrap_or(error_diags.len() as u64);
        if current_errors > 0 {
            item["current_errors"] = json!(current_errors);
        }
        error_diags
            .into_iter()
            .filter_map(|value| budget.admit(value))
            .collect()
    };

    let build_payload = |listed: &[Value], truncated: bool| -> Value {
        let mut f = item.clone();
        if !listed.is_empty() {
            f["diagnostics"] = Value::Array(listed.to_vec());
        }
        // M-Ctrl-9: Pure severity escalations (escalated > 0 with empty rows) and
        // positive current_errors still deliver even if rows were trimmed or absent.
        let has_findings = if compared {
            let changed = counted(comparison, "introduced") + counted(comparison, "escalated");
            changed > 0 || !listed.is_empty()
        } else {
            !listed.is_empty() || current_errors > 0
        };
        json!({
            "deliver": has_findings,
            "files": [f],
            "truncated": truncated,
        })
    };

    let mut truncated = budget.truncated;
    let mut payload = build_payload(&listed, truncated);
    while serde_json::to_vec(&payload)
        .is_ok_and(|bytes| bytes.len() > crate::post_edit_inspect::MAX_POST_EDIT_PAYLOAD_BYTES)
        && !listed.is_empty()
    {
        listed.pop();
        truncated = true;
        payload = build_payload(&listed, truncated);
    }
    (payload, true)
}

#[cfg(test)]
mod tests;

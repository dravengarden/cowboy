//! Counts over this Session's tracked observations, never project totals.
use serde_json::{Value, json};

const SEVERITIES: [&str; 4] = ["error", "warning", "information", "hint"];

pub(super) fn project(observation: Value) -> Value {
    let files = observation["files"]
        .as_array()
        .expect("internal diagnostics shape");
    let mut counts = json!({"error":0,"warning":0,"information":0,"hint":0});
    let mut observed = 0;
    let mut all_current = true;
    let mut stale = false;
    let mut listed = Vec::with_capacity(files.len());
    for file in files {
        let mut item = json!({"path":file["path"],"state":file["state"]});
        if file["counts"].is_object() {
            observed += 1;
            all_current &= file["state"] == "document_current";
            stale |= file["state"] == "stale";
            for severity in SEVERITIES {
                counts[severity] = json!(
                    counts[severity]
                        .as_u64()
                        .unwrap()
                        .saturating_add(file["counts"][severity].as_u64().unwrap_or(0))
                );
            }
            item["counts"] = file["counts"].clone();
        }
        listed.push(item);
    }
    let state = if observed == 0 {
        "unobserved"
    } else if observed != files.len() {
        "partial"
    } else if stale {
        "stale"
    } else if all_current {
        "document_current"
    } else {
        "last_observed"
    };
    json!({
        "coverage":"agent_observed_files", "state":state,
        "tracked":files.len(), "observed":observed,
        "counts":if observed == 0 { Value::Null } else { counts },
        "files":listed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(state: &str, errors: u64) -> Value {
        json!({"path":"a.rs","state":state,"counts":{
            "error":errors,"warning":0,"information":0,"hint":0},
            "diagnostics":[{"message":"repository text must not enter the summary"}]})
    }

    #[test]
    fn absent_or_restricted_observations_are_not_zero_diagnostics() {
        for files in [
            json!([]),
            json!([{"path":"a.rs","state":"not_open"}]),
            json!([{"path":"a.rs","state":"restricted"}]),
        ] {
            let result = project(json!({"files":files}));
            assert_eq!(result["state"], "unobserved");
            assert!(result["counts"].is_null());
        }
    }

    #[test]
    fn incomplete_coverage_and_unknown_servers_cannot_become_current() {
        let result = project(json!({"files":[file("document_current",2),
            {"path":"b.rs","state":"unavailable"}]}));
        assert_eq!(result["state"], "partial");
        assert_eq!(result["counts"]["error"], 2);
        assert_eq!(result["tracked"], 2);
        assert_eq!(result["observed"], 1);
        assert!(result["files"][0].get("diagnostics").is_none());
        for (second, expected) in [
            ("last_observed", "last_observed"),
            ("stale", "stale"),
            ("document_current", "document_current"),
        ] {
            let result = project(json!({"files":[file("document_current",0),file(second,3)]}));
            assert_eq!(result["state"], expected);
            assert_eq!(result["counts"]["error"], 3);
        }
    }
}

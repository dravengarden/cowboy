//! Match an actual Provider configuration value, including exact
//! Provider-declared aliases. The worker's set-config fence and the
//! Controller's managed-call confirmation share this one rule.
use serde_json::Value;

pub fn confirmed(option: &Value, expected: &Value) -> bool {
    let Some(current) = option.get("currentValue") else {
        return false;
    };
    if current == expected {
        return true;
    }
    // Only the connected worker's provider-owned metadata can declare an
    // equivalence. No suffix stripping, fuzzy model matching or UI projections.
    let Some(equivalence) = option.pointer("/_meta/cowboy.dev~1configuration-equivalence") else {
        return false;
    };
    equivalence["schema"] == 1
        && expected.is_string()
        && current.is_string()
        && equivalence.get("value") == Some(current)
        && equivalence["equivalent_values"]
            .as_array()
            .is_some_and(|values| {
                values.len() <= 8
                    && values.iter().all(Value::is_string)
                    && values.contains(expected)
            })
}

#[cfg(test)]
mod tests {
    use super::confirmed;
    use serde_json::json;

    #[test]
    fn only_the_current_values_own_declaration_confirms_another_spelling() {
        let option = |current: &str| {
            json!({"id": "model", "currentValue": current, "_meta": {
                "cowboy.dev/configuration-equivalence": {"schema": 1, "value": "fable",
                    "equivalent_values": ["claude-fable-5-2", "claude-fable-5-2[1m]"]}}})
        };
        assert!(confirmed(&option("fable"), &json!("fable")));
        assert!(confirmed(&option("fable"), &json!("claude-fable-5-2[1m]")));
        assert!(!confirmed(&option("fable"), &json!("claude-fable-5-1[1m]")));
        // Metadata left over from a previous value proves nothing.
        assert!(!confirmed(&option("opus"), &json!("claude-fable-5-2[1m]")));
        assert!(!confirmed(
            &json!({"id": "model", "currentValue": "fable"}),
            &json!("claude-fable-5-2")
        ));
    }
}

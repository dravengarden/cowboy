//! Match actual worker configuration, including exact Provider-declared aliases.
use serde_json::Value;

pub(super) fn confirmed(option: &Value, expected: &Value) -> bool {
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

pub(super) fn matches(options: Option<&Value>, expected: &Value) -> bool {
    let Some(options) = options.and_then(Value::as_array) else {
        return false;
    };
    expected.as_object().is_some_and(|expected| {
        expected.iter().all(|(key, value)| {
            options
                .iter()
                .any(|option| option["id"].as_str() == Some(key) && confirmed(option, value))
        })
    })
}

//! Match actual worker configuration, including exact Provider-declared aliases.
use serde_json::Value;

pub(super) use cowboy::native_configuration::confirmed;

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

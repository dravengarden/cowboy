//! A browsing preference refers to an existing code context; it grants no file
//! access and never moves the Agent's execution workspace.
pub fn valid(value: &str) -> bool {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return false;
    }
    if value == "project" {
        return true;
    }
    let parts: Vec<_> = value.split("::").collect();
    parts.iter().all(|part| !part.is_empty())
        && matches!(parts.as_slice(), ["workspace", _, _] | ["session", _])
}

/// Preserve the distinction between an absent override (inherit) and null
/// (stop inheritance and use automatic session/worktree resolution).
pub fn deserialize_override<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    use serde::Deserialize as _;
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub scope: String,
    /// Omit to change the calling session; otherwise its owner's directory.
    pub folder_id: Option<String>,
    pub expected: serde_json::Value,
    pub change: serde_json::Value,
}

pub fn override_value(value: Option<&Option<String>>) -> serde_json::Value {
    value.map_or_else(
        || serde_json::json!({}),
        |value| serde_json::json!({"directory":value}),
    )
}

pub fn parse_change(value: &serde_json::Value) -> Result<Option<Option<String>>, &'static str> {
    let map = value.as_object().ok_or("invalid_argument")?;
    if map.len() != 1 {
        return Err("invalid_argument");
    }
    if value.get("inherit") == Some(&serde_json::Value::Bool(true)) {
        return Ok(None);
    }
    match value.get("directory") {
        Some(serde_json::Value::Null) => Ok(Some(None)),
        Some(serde_json::Value::String(value)) if valid(value) => Ok(Some(Some(value.clone()))),
        _ => Err("invalid_argument"),
    }
}

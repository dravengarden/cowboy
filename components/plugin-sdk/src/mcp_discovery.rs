//! Signed, bounded startup metadata. It permits discovery before a parent
//! gateway exists, but never authorizes starting a process or invoking a tool.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Validate bounded definitions against the exact selected tool names.
/// # Errors
/// Rejects mismatched names, oversized definitions and unsafe schemas.
pub fn validate_mcp_discovery(definitions: &[McpToolDefinition], tools: &[String]) -> Result<()> {
    ensure!(
        serde_json::to_vec(definitions)?.len() <= 32 * 1024,
        "MCP discovery too large"
    );
    let names: BTreeSet<_> = definitions.iter().map(|tool| &tool.name).collect();
    ensure!(
        definitions.len() == tools.len()
            && names.len() == definitions.len()
            && names == tools.iter().collect(),
        "MCP discovery must exactly describe signed tools"
    );
    for tool in definitions {
        ensure!(
            !tool.description.trim().is_empty()
                && tool.description.len() <= 2048
                && tool.input_schema.is_object()
                && tool.input_schema["type"] == "object"
                && bounded_schema(&tool.input_schema, 0),
            "invalid MCP tool schema"
        );
    }
    Ok(())
}

fn bounded_schema(value: &Value, depth: usize) -> bool {
    if depth > 24 {
        return false;
    }
    match value {
        Value::Object(fields) => fields.iter().all(|(key, value)| {
            !(key == "$id" && value.is_string())
                && (!matches!(key.as_str(), "$ref" | "$dynamicRef")
                    || value
                        .as_str()
                        .is_some_and(|reference| reference.starts_with('#')))
                && bounded_schema(value, depth + 1)
        }),
        Value::Array(values) => values.iter().all(|value| bounded_schema(value, depth + 1)),
        _ => true,
    }
}

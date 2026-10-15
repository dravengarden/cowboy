//! `cowboy mcp`: a stdio MCP server for the agent of a Cowboy session.
//!
//! It forwards each tool call as one gateway action and holds no connection
//! between calls, so an idle server costs nothing. Outside a Cowboy session,
//! or under a Controller without the Agent API, it advertises no tools, which
//! keeps a globally registered server free of per-turn token cost.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Mutex;

use super::{INSTRUCTIONS, READ_DESCRIPTION, SDK, STATUS_DESCRIPTION};
use crate::managed_calls::cli::{exchange, wiring};
use crate::managed_calls::protocol::Action;

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const SDK_URI: &str = "cowboy://sdk/v1";
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(clap::Args)]
pub struct McpArgs {}

fn tools() -> Value {
    json!([
        {
            "name": "cowboy_status",
            "description": STATUS_DESCRIPTION,
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        },
        {
            "name": "cowboy_read",
            "description": READ_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {"type": "string"},
                    "language": {"enum": ["ts", "js"]},
                    "format": {"enum": ["body", "module"]},
                    "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 60000},
                },
                "required": ["code"],
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        },
    ])
}

/// Whether this process can reach a Controller that serves the Agent API.
/// Decided once at startup: a grant can change later, and a call then reports
/// its own error, but the advertised surface stays stable for the session.
async fn available() -> bool {
    let Ok(context) = wiring() else {
        return false;
    };
    let reply = exchange(context, Action::AgentStatus {}, 0).await;
    reply.get("state").and_then(Value::as_str) == Some("ok")
}

fn action(name: &str, arguments: &Value) -> Result<Action, String> {
    let arguments = match arguments {
        Value::Null => json!({}),
        // The tag selects the action; a tool argument never may.
        Value::Object(map) if !map.contains_key("kind") => arguments.clone(),
        _ => return Err("invalid arguments".into()),
    };
    let mut tagged = arguments;
    tagged["kind"] = json!(match name {
        "cowboy_status" => "agent_status",
        "cowboy_read" => "agent_read",
        _ => return Err(format!("unknown tool {name}")),
    });
    let action: Action =
        serde_json::from_value(tagged).map_err(|_| "invalid arguments".to_owned())?;
    action
        .validate()
        .map_err(|error| format!("invalid arguments: {error}"))?;
    Ok(action)
}

/// One compact text block: the gateway envelope minus its wire schema.
fn tool_result(mut reply: Value) -> Value {
    let failed = reply.get("state").and_then(Value::as_str) != Some("ok");
    if let Some(object) = reply.as_object_mut() {
        object.remove("schema");
        if !failed {
            object.remove("state");
        }
    }
    let text = if !failed
        && reply
            .as_object()
            .is_some_and(|o| o.len() == 1 && o.contains_key("value"))
    {
        reply["value"].to_string()
    } else {
        reply.to_string()
    };
    json!({"content": [{"type": "text", "text": text}], "isError": failed})
}

async fn handle(method: &str, params: &Value, enabled: bool) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let requested = params.get("protocolVersion").and_then(Value::as_str);
            let version = requested
                .filter(|version| PROTOCOL_VERSIONS.contains(version))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}, "resources": {}},
                "serverInfo": {"name": "cowboy", "version": env!("CARGO_PKG_VERSION")},
                "instructions": if enabled { INSTRUCTIONS } else { "" },
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": if enabled { tools() } else { json!([]) }})),
        "resources/list" => Ok(json!({"resources": if enabled {
            json!([{"uri": SDK_URI, "name": "Cowboy agent SDK", "mimeType": "text/typescript"}])
        } else {
            json!([])
        }})),
        "resources/read" => {
            if enabled && params.get("uri").and_then(Value::as_str) == Some(SDK_URI) {
                Ok(
                    json!({"contents": [{"uri": SDK_URI, "mimeType": "text/typescript", "text": SDK}]}),
                )
            } else {
                Err((-32002, "resource not found".into()))
            }
        }
        "tools/call" => {
            if !enabled {
                return Err((-32602, "Cowboy agent tools are unavailable here".into()));
            }
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let action = match action(name, params.get("arguments").unwrap_or(&Value::Null)) {
                Ok(action) => action,
                Err(message) => {
                    return Ok(json!({"content":[{"type":"text","text":message}],"isError":true}));
                }
            };
            let Ok(context) = wiring() else {
                return Ok(tool_result(
                    json!({"state":"error","error":{"code":"context_unavailable"}}),
                ));
            };
            let timeout = action.wait_ms();
            Ok(tool_result(exchange(context, action, timeout).await))
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

pub async fn serve(_args: McpArgs) -> anyhow::Result<()> {
    let enabled = available().await;
    let output = std::sync::Arc::new(Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(tokio::io::stdin());
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        let mut line = Vec::new();
        let read = (&mut lines)
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .await?;
        if read == 0 {
            break;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        // Notifications (no id) need no reply; cancellation of a read program
        // has no effect to fence, so it is ignored.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let output = std::sync::Arc::clone(&output);
        tasks.spawn(async move {
            let reply = match handle(&method, &params, enabled).await {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err((code, message)) => {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
                }
            };
            let mut bytes = serde_json::to_vec(&reply).unwrap_or_default();
            bytes.push(b'\n');
            let mut output = output.lock().await;
            let _ = output.write_all(&bytes).await;
            let _ = output.flush().await;
        });
        while tasks.try_join_next().is_some() {}
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_arguments_become_validated_gateway_actions() {
        assert!(matches!(
            action("cowboy_status", &Value::Null),
            Ok(Action::AgentStatus {})
        ));
        assert!(matches!(
            action(
                "cowboy_read",
                &json!({"code": "return 1", "timeout_ms": 5000})
            ),
            Ok(Action::AgentRead { .. })
        ));
        for (name, arguments) in [
            ("cowboy_read", json!({})),
            (
                "cowboy_read",
                json!({"code": "return 1", "timeout_ms": 60001}),
            ),
            ("cowboy_read", json!({"code": "return 1", "language": "py"})),
            ("cowboy_read", json!({"code": "return 1", "kind": "start"})),
            ("cowboy_read", json!({"code": "x".repeat(32 * 1024 + 1)})),
            ("cowboy_status", json!({"parent_session_id": "victim"})),
            ("other", json!({})),
        ] {
            assert!(action(name, &arguments).is_err(), "{name} {arguments}");
        }
    }

    #[test]
    fn results_are_one_compact_text_block() {
        let ok = tool_result(json!({"schema":1,"state":"ok","value":{"a":1}}));
        assert_eq!(
            ok,
            json!({"content":[{"type":"text","text":"{\"a\":1}"}],"isError":false})
        );
        let logged = tool_result(json!({"schema":1,"state":"ok","value":1,"logs":"x\n"}));
        let text: Value =
            serde_json::from_str(logged["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, json!({"value":1,"logs":"x\n"}));
        let failed = tool_result(json!({"schema":1,"state":"error","error":{"code":"timeout"}}));
        assert_eq!(failed["isError"], true);
        let text: Value =
            serde_json::from_str(failed["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, json!({"state":"error","error":{"code":"timeout"}}));
    }

    #[tokio::test]
    async fn outside_a_session_nothing_is_advertised() {
        let list = handle("tools/list", &Value::Null, false).await.unwrap();
        assert_eq!(list, json!({"tools": []}));
        let init = handle("initialize", &json!({"protocolVersion":"2025-03-26"}), true)
            .await
            .unwrap();
        assert_eq!(init["protocolVersion"], "2025-03-26");
        let sdk = handle("resources/read", &json!({"uri": SDK_URI}), true)
            .await
            .unwrap();
        assert_eq!(sdk["contents"][0]["text"], SDK);
        assert!(
            handle("tools/call", &json!({"name":"cowboy_status"}), false)
                .await
                .is_err()
        );
    }
}

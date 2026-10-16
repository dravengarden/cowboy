//! `cowboy mcp`: a stdio MCP server for the agent of a Cowboy session.
//!
//! It forwards each tool call as one gateway action and holds no connection
//! between calls, so an idle server costs nothing. Outside a Cowboy session,
//! or under a Controller without the Agent API, it advertises no tools, which
//! keeps a globally registered server free of per-turn token cost.

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Mutex;

use super::{
    EXECUTE_DESCRIPTION, INSTRUCTIONS, READ_DESCRIPTION, RECEIPT_DESCRIPTION, SDK,
    STATUS_DESCRIPTION,
};
use crate::managed_calls::cli::{exchange, wiring};
use crate::managed_calls::protocol::Action;

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const SDK_URI: &str = "cowboy://sdk/v1";
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(clap::Args)]
pub struct McpArgs {}

fn tools(actions: bool) -> Value {
    let mut tools = json!([
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
                    "help": {"type":"string"},
                    "language": {"enum": ["ts", "js"]},
                    "format": {"enum": ["body", "module"]},
                    "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 60000},
                },
                "additionalProperties": false,
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": false},
        },
        {
            "name":"cowboy_execute","description":EXECUTE_DESCRIPTION,
            "inputSchema":{"type":"object","properties":{
                "action":{"type":"string"},"arguments":{"type":"object"},"operation":{"type":"string"}
            },"required":["action","arguments","operation"],"additionalProperties":false},
            "annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":true,"openWorldHint":false}
        },
        {
            "name":"cowboy_receipt","description":RECEIPT_DESCRIPTION,
            "inputSchema":{"type":"object","properties":{
                "domain":{"enum":["calls","documents"]},"operation":{"type":"string"}
            },"required":["domain","operation"],"additionalProperties":false},
            "annotations":{"readOnlyHint":true,"openWorldHint":false}
        },
    ]);
    if !actions {
        tools.as_array_mut().unwrap().truncate(2);
    }
    tools
}

#[derive(Clone, Copy, Default)]
struct Surface {
    enabled: bool,
    actions: bool,
}

impl From<bool> for Surface {
    fn from(enabled: bool) -> Self {
        Self {
            enabled,
            actions: enabled,
        }
    }
}

/// Whether this process can reach a Controller that serves the Agent API.
/// Decided once at startup: a grant can change later, and a call then reports
/// its own error, but the advertised surface stays stable for the session.
async fn available() -> Surface {
    let Ok(context) = wiring() else {
        return Surface::default();
    };
    let reply = exchange(context, Action::AgentStatus {}, 0).await;
    Surface {
        enabled: reply.get("state").and_then(Value::as_str) == Some("ok"),
        actions: reply["actions"] == true,
    }
}

fn action(name: &str, arguments: &Value) -> Result<Action, String> {
    let arguments = match arguments {
        Value::Null => json!({}),
        // The tag selects the action; a tool argument never may.
        Value::Object(map) if !map.contains_key("kind") => arguments.clone(),
        _ => return Err("invalid arguments".into()),
    };
    let mut tagged = arguments;
    if name == "cowboy_receipt" {
        if tagged.as_object().is_none_or(|map| map.len() != 2)
            || !matches!(tagged["domain"].as_str(), Some("calls" | "documents"))
            || tagged["operation"]
                .as_str()
                .is_none_or(|id| !crate::managed_calls::valid_id(id))
        {
            return Err("invalid receipt arguments".into());
        }
        return Ok(Action::AgentInvoke {
            action: "receipts.get".into(),
            arguments: tagged,
            operation: None,
        });
    }
    tagged["kind"] = json!(match name {
        "cowboy_status" => "agent_status",
        "cowboy_read" => "agent_read",
        "cowboy_execute" => "agent_invoke",
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

async fn handle(
    method: &str,
    params: &Value,
    surface: impl Into<Surface>,
) -> Result<Value, (i64, String)> {
    let Surface { enabled, actions } = surface.into();
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
        "tools/list" => Ok(json!({"tools": if enabled { tools(actions) } else { json!([]) }})),
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
            if !actions && matches!(name, "cowboy_execute" | "cowboy_receipt") {
                return Ok(tool_result(
                    json!({"state":"error","error":{"code":"unsupported_capability"}}),
                ));
            }
            if name == "cowboy_read" && params["arguments"].get("help").is_some() {
                let args = &params["arguments"];
                let result = args
                    .as_object()
                    .filter(|v| v.len() == 1)
                    .and_then(|_| args["help"].as_str())
                    .and_then(super::help);
                return Ok(match result {
                    Some(mut value) => {
                        if !actions {
                            value["available_actions"] = json!([]);
                            if value.get("actions").is_some() {
                                value["actions"] = json!([]);
                            }
                        }
                        tool_result(json!({"state":"ok","value":value}))
                    }
                    None => tool_result(json!({"state":"error","error":{"code":"invalid_help"}})),
                });
            }
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

    #[tokio::test]
    async fn discovery_is_local_and_older_machines_keep_only_read_tools() {
        let old = Surface {
            enabled: true,
            actions: false,
        };
        let list = handle("tools/list", &Value::Null, old).await.unwrap();
        assert_eq!(list["tools"].as_array().unwrap().len(), 2);
        let modern = handle("tools/list", &Value::Null, true).await.unwrap();
        assert_eq!(modern["tools"].as_array().unwrap().len(), 4);
        let bytes = serde_json::to_vec(&modern["tools"]).unwrap().len() + INSTRUCTIONS.len();
        assert!(bytes < 4096, "resident definitions: {bytes} UTF-8 bytes");
        eprintln!("Cowboy resident definitions and instructions: {bytes} UTF-8 bytes (not tokens)");
        let help = handle(
            "tools/call",
            &json!({"name":"cowboy_read","arguments":{"help":"index"}}),
            old,
        )
        .await
        .unwrap();
        let index: Value =
            serde_json::from_str(help["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(index["actions"], json!([]));
        let unavailable = handle("tools/call", &json!({"name":"cowboy_execute","arguments":{"action":"review.start","arguments":{},"operation":"test"}}), old).await.unwrap();
        assert_eq!(unavailable["isError"], true);
        let help = handle(
            "tools/call",
            &json!({"name":"cowboy_read","arguments":{"help":"actions"}}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(help["isError"], false);
        for arguments in [
            json!({"help":"actions","code":"return 1"}),
            json!({"help":"missing"}),
        ] {
            assert_eq!(
                handle(
                    "tools/call",
                    &json!({"name":"cowboy_read","arguments":arguments}),
                    true
                )
                .await
                .unwrap()["isError"],
                true
            );
        }
        assert!(matches!(
            action(
                "cowboy_execute",
                &json!({"action":"review.start","arguments":{},"operation":"review-1"})
            ),
            Ok(Action::AgentInvoke {
                operation: Some(_),
                ..
            })
        ));
        assert!(matches!(
            action(
                "cowboy_receipt",
                &json!({"domain":"documents","operation":"edit-1"})
            ),
            Ok(Action::AgentInvoke {
                operation: None,
                ..
            })
        ));
        for arguments in [
            json!({"action":"shell","arguments":{},"operation":"x"}),
            json!({"action":"review.start","arguments":{},"operation":"../x"}),
            json!({"action":"review.start","arguments":[],"operation":"x"}),
        ] {
            assert!(action("cowboy_execute", &arguments).is_err());
        }
    }
}

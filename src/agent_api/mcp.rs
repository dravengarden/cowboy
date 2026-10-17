//! `cowboy mcp`: a stdio MCP server for the agent of a Cowboy session.
//!
//! It forwards each tool call as one gateway action and holds no connection
//! between calls, so an idle server costs nothing. Outside a Cowboy session,
//! or under a Controller without the Agent API, it advertises no tools, which
//! keeps a globally registered server free of per-turn token cost.

use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};
use tokio::sync::Mutex;

use super::{
    EXECUTE_DESCRIPTION, INSTRUCTIONS, READ_DESCRIPTION, RECEIPT_DESCRIPTION, SDK,
    STATUS_DESCRIPTION,
};
use crate::managed_calls::cli::{exchange, wiring};
use crate::managed_calls::protocol::Action;

const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const SDK_URI: &str = "cowboy://sdk/v1";
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_IN_FLIGHT: usize = 16;

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
                "domain":{"enum":["calls","documents","preferences"]},"operation":{"type":"string"}
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
            || !matches!(
                tagged["domain"].as_str(),
                Some("calls" | "documents" | "preferences")
            )
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
        "resources/templates/list" => Ok(json!({"resourceTemplates":[]})),
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
            if !matches!(
                name,
                "cowboy_status" | "cowboy_read" | "cowboy_execute" | "cowboy_receipt"
            ) {
                return Err((-32602, "unknown tool".into()));
            }
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
                Err(_) => {
                    return Ok(tool_result(
                        json!({"state":"error","error":{"code":"invalid_argument"}}),
                    ));
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
        _ => Err((-32601, "method not found".into())),
    }
}

pub async fn serve(_args: McpArgs) -> anyhow::Result<()> {
    serve_io(tokio::io::stdin(), tokio::io::stdout(), available().await).await
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

async fn write_reply(
    output: &Mutex<impl AsyncWrite + Unpin>,
    reply: &Value,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(reply)?;
    bytes.push(b'\n');
    let mut output = output.lock().await;
    output.write_all(&bytes).await?;
    output.flush().await
}

async fn serve_io(
    input: impl AsyncRead + Unpin,
    output: impl AsyncWrite + Unpin + Send + 'static,
    surface: Surface,
) -> anyhow::Result<()> {
    let output = std::sync::Arc::new(Mutex::new(output));
    let mut lines = BufReader::new(input);
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
        if read > MAX_MESSAGE_BYTES {
            // Drain this frame, without buffering it. Its suffix must never
            // become a new request (and could otherwise execute an action).
            while !line.ends_with(b"\n") {
                line.clear();
                if (&mut lines)
                    .take(MAX_MESSAGE_BYTES as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .await?
                    == 0
                {
                    break;
                }
            }
            write_reply(
                &output,
                &rpc_error(Value::Null, -32600, "message too large"),
            )
            .await?;
            continue;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            write_reply(&output, &rpc_error(Value::Null, -32700, "parse error")).await?;
            continue;
        };
        let valid_id = message
            .get("id")
            .is_none_or(|id| id.is_string() || id.is_i64() || id.is_u64());
        if !message.is_object()
            || message["jsonrpc"] != "2.0"
            || !message["method"].is_string()
            || !valid_id
        {
            write_reply(&output, &rpc_error(Value::Null, -32600, "invalid request")).await?;
            continue;
        }
        // Notifications never have replies. Cancellation does not roll back
        // domain writes; their original operation/receipt remains authoritative.
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        if message
            .get("params")
            .is_some_and(|params| !params.is_object())
        {
            write_reply(&output, &rpc_error(id, -32602, "invalid params")).await?;
            continue;
        }
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        // Backpressure bounds both running requests and completed replies
        // waiting for stdout; idle servers still hold no gateway connection.
        while let Some(result) = tasks.try_join_next() {
            result??;
        }
        if tasks.len() >= MAX_IN_FLIGHT
            && let Some(result) = tasks.join_next().await
        {
            result??;
        }
        let output = std::sync::Arc::clone(&output);
        tasks.spawn(async move {
            let reply = match handle(&method, &params, surface).await {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err((code, message)) => {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
                }
            };
            write_reply(&output, &reply).await
        });
    }
    while let Some(result) = tasks.join_next().await {
        result??;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn rpc_exchange(input: Vec<u8>) -> Vec<Value> {
        let (mut client, output) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve_io(
            std::io::Cursor::new(input),
            output,
            Surface::from(true),
        ));
        let mut bytes = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap().unwrap();
        bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn oversized_frames_cannot_turn_their_suffix_into_another_request() {
        let mut input = vec![b' '; MAX_MESSAGE_BYTES + 1];
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":\"suffix\",\"method\":\"ping\"}\n");
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":\"next\",\"method\":\"ping\"}\n");
        let replies = rpc_exchange(input).await;
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["error"]["code"], -32600);
        assert_eq!(replies[1], json!({"jsonrpc":"2.0","id":"next","result":{}}));
    }

    #[tokio::test]
    async fn malformed_requests_reply_and_notifications_stay_silent() {
        let mut input = b"{broken\n".to_vec();
        for value in [
            json!([]),
            json!({"id":1,"method":"ping"}),
            json!({"jsonrpc":"2.0","id":null,"method":"ping"}),
            json!({"jsonrpc":"2.0","id":{},"method":"ping"}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"ping","params":[]}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"unknown"}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cowboy_read","arguments":{}}}),
        ] {
            input.extend(serde_json::to_vec(&value).unwrap());
            input.push(b'\n');
        }
        let replies = rpc_exchange(input).await;
        assert_eq!(replies.len(), 8);
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(
            replies
                .iter()
                .filter(|r| r["error"]["code"] == -32600)
                .count(),
            4
        );
        for id in [2, 3] {
            assert_eq!(
                replies.iter().find(|r| r["id"] == id).unwrap()["error"]["code"],
                -32602
            );
        }
        let result = &replies.iter().find(|r| r["id"] == 4).unwrap()["result"];
        assert_eq!(result["isError"], true);
        let value: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(value["error"]["code"], "invalid_argument");
    }

    #[tokio::test]
    async fn pipelined_requests_keep_every_response_under_backpressure() {
        let mut input = Vec::new();
        for id in 0..MAX_IN_FLIGHT * 4 {
            input.extend(
                serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":"ping"})).unwrap(),
            );
            input.push(b'\n');
        }
        let mut replies = rpc_exchange(input).await;
        replies.sort_by_key(|v| v["id"].as_u64().unwrap());
        assert_eq!(replies.len(), MAX_IN_FLIGHT * 4);
        for (id, reply) in replies.iter().enumerate() {
            assert_eq!(reply, &json!({"jsonrpc":"2.0","id":id,"result":{}}));
        }
        let init = handle("initialize", &json!({"protocolVersion":"2025-11-25"}), true)
            .await
            .unwrap();
        assert_eq!(init["protocolVersion"], "2025-11-25");
        assert_eq!(
            handle("resources/templates/list", &json!({}), true)
                .await
                .unwrap(),
            json!({"resourceTemplates":[]})
        );
    }

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

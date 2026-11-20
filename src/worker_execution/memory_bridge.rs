//! Matrix memory for an account-sandboxed session (Antigravity). The unit
//! reaches no host loopback service, so the worker forwards the proxy's
//! recall and learning requests to this session's relay on the runtime
//! Machine: only to that relay, only the actions its signed configuration
//! enables (`context` with recall, `observe` with observation), with the
//! relay token the unit never sees. Memory's tools are the worker's own
//! (`extension_broker`), limited to the signed tool list.

use std::time::Duration;

use serde_json::{Value, json};

/// The endpoint's own request: one memory exchange for the session's relay.
/// Never forwarded to the target.
pub const MEMORY_REQUEST: &str = "cowboy/memory";
const MAX_BODY: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
/// Request headers the proxy may set; everything else is the worker's.
const FORWARDED_REQUEST_HEADERS: &[&str] = &["accept", "content-type", "x-matrix-binding"];

/// The session's memory relay, from its extension descriptor.
#[derive(Clone)]
pub struct MemoryBridge {
    endpoint: reqwest::Url,
    token: String,
    client: reqwest::Client,
    recall: bool,
    observe: bool,
}

impl std::fmt::Debug for MemoryBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryBridge")
            .field("endpoint", &self.endpoint.as_str())
            .finish_non_exhaustive()
    }
}

impl MemoryBridge {
    /// The relay of `extensions` (`memory.relay`), when it is the Machine's
    /// loopback relay this worker can reach.
    #[must_use]
    pub fn from_extensions(extensions: &Value) -> Option<Self> {
        let memory = extensions.get("memory")?;
        let flag = |key: &str| memory.get(key).and_then(Value::as_bool).unwrap_or(false);
        let relay = memory.get("relay")?;
        let endpoint = reqwest::Url::parse(relay.get("endpoint")?.as_str()?).ok()?;
        let token = relay.get("token")?.as_str()?.to_owned();
        let loopback = endpoint.scheme() == "http"
            && endpoint.host_str() == Some("127.0.0.1")
            && endpoint.path() == "/"
            && endpoint.query().is_none();
        if !loopback || token.is_empty() || token.len() > 512 {
            return None;
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(TIMEOUT)
            .build()
            .ok()?;
        Some(Self {
            endpoint,
            token,
            client,
            recall: flag("enable_recall"),
            observe: flag("observe"),
        })
    }

    /// Forward one `{path, headers, body}` exchange of the unit's memory
    /// client and answer `{status, headers, body}`: `POST /v1/context` with
    /// recall and `POST /v1/observe` with observation, nothing else.
    pub async fn forward(&self, params: &Value) -> Value {
        let Some(path) = params.get("path").and_then(Value::as_str) else {
            return refused("memory request needs a path");
        };
        let allowed = match path {
            "/v1/context" => self.recall,
            "/v1/observe" => self.observe,
            _ => false,
        };
        let method = params
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("POST");
        if !allowed || method != "POST" {
            return refused("memory request refused");
        }
        let body = params.get("body").and_then(Value::as_str).unwrap_or("");
        if body.len() > MAX_BODY {
            return refused("memory request exceeds its size limit");
        }
        let Ok(url) = self.endpoint.join(path) else {
            return refused("memory request refused");
        };
        let mut request = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .body(body.to_owned());
        if let Some(headers) = params.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                let name = name.to_ascii_lowercase();
                if let (true, Some(value)) = (
                    FORWARDED_REQUEST_HEADERS.contains(&name.as_str()),
                    value.as_str().filter(|value| value.len() <= 8192),
                ) {
                    request = request.header(name, value);
                }
            }
        }
        let Ok(response) = request.send().await else {
            return refused("memory relay unavailable");
        };
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let Ok(bytes) = read_bounded(response).await else {
            return refused("memory response exceeds its size limit");
        };
        json!({"result": {
            "status": status,
            "headers": content_type.map_or_else(|| json!({}), |value| json!({"content-type": value})),
            "body": String::from_utf8_lossy(&bytes),
        }})
    }

    async fn post_mcp(&self, message: &Value) -> Result<(u16, bool, Vec<u8>), String> {
        let url = self
            .endpoint
            .join("/mcp")
            .map_err(|_| "relay URL".to_owned())?;
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .body(message.to_string())
            .send()
            .await
            .map_err(|_| "memory relay unavailable".to_owned())?;
        let status = response.status().as_u16();
        let event_stream = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
        let bytes = read_bounded(response)
            .await
            .map_err(|()| "memory response exceeds its size limit".to_owned())?;
        Ok((status, event_stream, bytes))
    }

    /// One MCP request of the worker's own to the relay's `/mcp`: the
    /// answer's `result`, else `(refused, text)`, where `refused` means the
    /// backend definitively declined it (an error answer or a client-error
    /// status) and not that its outcome is unknown.
    pub async fn mcp(&self, message: &Value) -> Result<Value, (bool, String)> {
        let (status, event_stream, bytes) =
            self.post_mcp(message).await.map_err(|text| (false, text))?;
        if !(200..300).contains(&status) {
            let definitive = (400..500).contains(&status) && ![408, 429].contains(&status);
            return Err((definitive, format!("memory answered {status}")));
        }
        answer(
            &String::from_utf8_lossy(&bytes),
            event_stream,
            message.get("id"),
        )
    }

    /// One MCP notification (no answer is read).
    pub async fn notify(&self, message: &Value) -> Result<(), String> {
        let (status, _, _) = self.post_mcp(message).await?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(format!("memory answered {status}"))
        }
    }
}

/// The answer to request `id` in a JSON or server-sent-events body; an error
/// answer is a definitive refusal (`true`), no answer is not.
fn answer(text: &str, event_stream: bool, id: Option<&Value>) -> Result<Value, (bool, String)> {
    let payloads: Vec<String> = if event_stream {
        text.split("\n\n")
            .map(|event| {
                event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(str::trim_start)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect()
    } else {
        vec![text.to_owned()]
    };
    for payload in payloads {
        let Ok(value) = serde_json::from_str::<Value>(&payload) else {
            continue;
        };
        if value.get("method").is_some() || value.get("id") != id {
            continue;
        }
        if let Some(result) = value.get("result") {
            return Ok(result.clone());
        }
        let error = value.pointer("/error/message").and_then(Value::as_str);
        return Err((
            true,
            error.unwrap_or("memory error").chars().take(500).collect(),
        ));
    }
    Err((false, "memory gave no answer".into()))
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len() + chunk.len() > MAX_BODY {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn refused(message: &str) -> Value {
    json!({"error": {"code": -32000, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(endpoint: &str, token: &str, recall: bool, observe: bool) -> Value {
        json!({"memory": {"enable_recall": recall, "observe": observe,
            "relay": {"endpoint": endpoint, "token": token, "state_dir": "/x"}}})
    }

    #[test]
    fn only_a_loopback_relay_with_a_token_is_bridged() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        assert!(
            MemoryBridge::from_extensions(&relay("http://127.0.0.1:4100", "t", true, true))
                .is_some()
        );
        for (endpoint, token) in [
            ("http://127.0.0.1:4100", ""),
            ("https://127.0.0.1:4100", "t"),
            ("http://10.0.0.1:4100", "t"),
            ("http://127.0.0.1:4100/base", "t"),
        ] {
            assert!(
                MemoryBridge::from_extensions(&relay(endpoint, token, true, true)).is_none(),
                "{endpoint}"
            );
        }
        assert!(MemoryBridge::from_extensions(&json!({"memory": null})).is_none());
    }

    #[tokio::test]
    async fn the_unit_reaches_only_the_actions_its_switches_enable() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let recall_only =
            MemoryBridge::from_extensions(&relay("http://127.0.0.1:9", "t", true, false)).unwrap();
        for params in [
            json!({"path": "/v1/observe"}),
            json!({"path": "/mcp"}),
            json!({"path": "/v1/../admin"}),
            json!({"path": "/v1/context", "method": "GET"}),
            json!({"path": "/v1/context", "body": "x".repeat(MAX_BODY + 1)}),
        ] {
            let reply = recall_only.forward(&params).await;
            let message = reply["error"]["message"].as_str().unwrap();
            assert!(!message.contains("unavailable"), "{params}: {message}");
        }
        // An allowed action is sent (here to a closed port).
        let reply = recall_only.forward(&json!({"path": "/v1/context"})).await;
        assert_eq!(reply["error"]["message"], "memory relay unavailable");
    }

    #[test]
    fn answers_are_found_in_json_and_event_streams() {
        assert_eq!(
            answer(
                r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#,
                false,
                Some(&json!(2))
            ),
            Ok(json!({"tools": []}))
        );
        let stream = "event: message\ndata: {\"id\":1,\"method\":\"ping\"}\n\ndata: {\"id\":1,\"result\":{\"ok\":true}}\n\n";
        assert_eq!(
            answer(stream, true, Some(&json!(1))),
            Ok(json!({"ok": true}))
        );
        assert_eq!(
            answer("nope", false, Some(&json!(1))),
            Err((false, "memory gave no answer".into()))
        );
        assert_eq!(
            answer(
                r#"{"id":1,"error":{"message":"bad"}}"#,
                false,
                Some(&json!(1))
            ),
            Err((true, "bad".into()))
        );
    }
}

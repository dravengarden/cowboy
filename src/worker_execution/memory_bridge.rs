//! Matrix memory for an account-sandboxed session (Antigravity). The unit
//! reaches no host loopback service, so the worker forwards the proxy's
//! memory requests to this session's relay on the runtime Machine: only to
//! that relay, only its two routes, with the relay token the unit never
//! sees. The relay itself scopes every request to this session.

use std::time::Duration;

use serde_json::{Value, json};

/// The endpoint's own request: one memory exchange for the session's relay.
/// Never forwarded to the target.
pub const MEMORY_REQUEST: &str = "cowboy/memory";
const MAX_BODY: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
/// Request headers the proxy may set; everything else is the worker's.
const FORWARDED_REQUEST_HEADERS: &[&str] = &[
    "accept",
    "content-type",
    "mcp-session-id",
    "mcp-protocol-version",
    "x-matrix-binding",
];
/// Response headers returned to the proxy.
const FORWARDED_RESPONSE_HEADERS: &[&str] = &["content-type", "mcp-session-id"];

/// The session's memory relay, from its extension descriptor.
#[derive(Clone)]
pub struct MemoryBridge {
    endpoint: reqwest::Url,
    token: String,
    client: reqwest::Client,
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
        let relay = extensions.get("memory")?.get("relay")?;
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
        })
    }

    /// Forward one `{path, headers, body}` exchange and answer
    /// `{status, headers, body}`. Only `POST /v1/<action>` and `POST /mcp`
    /// (and `DELETE /mcp`, which ends an MCP session) reach the relay.
    pub async fn forward(&self, params: &Value) -> Value {
        let Some(path) = params.get("path").and_then(Value::as_str) else {
            return refused("memory request needs a path");
        };
        let action = path.strip_prefix("/v1/").is_some_and(|action| {
            !action.is_empty()
                && action.len() <= 64
                && action
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        });
        let method = match params
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("POST")
        {
            "POST" if action || path == "/mcp" => reqwest::Method::POST,
            "DELETE" if path == "/mcp" => reqwest::Method::DELETE,
            _ => return refused("memory request refused"),
        };
        let body = params.get("body").and_then(Value::as_str).unwrap_or("");
        if body.len() > MAX_BODY {
            return refused("memory request exceeds its size limit");
        }
        let Ok(url) = self.endpoint.join(path) else {
            return refused("memory request refused");
        };
        let mut request = self
            .client
            .request(method, url)
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
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => return refused("memory relay unavailable"),
        };
        let status = response.status().as_u16();
        let mut headers = serde_json::Map::new();
        for name in FORWARDED_RESPONSE_HEADERS {
            if let Some(value) = response.headers().get(*name).and_then(|v| v.to_str().ok()) {
                headers.insert((*name).to_owned(), json!(value));
            }
        }
        let Ok(bytes) = read_bounded(response).await else {
            return refused("memory response exceeds its size limit");
        };
        json!({"result": {
            "status": status,
            "headers": headers,
            "body": String::from_utf8_lossy(&bytes),
        }})
    }
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

    #[test]
    fn only_a_loopback_relay_with_a_token_is_bridged() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let relay = |endpoint: &str, token: &str| json!({"memory": {"relay": {"endpoint": endpoint, "token": token, "state_dir": "/x"}}});
        assert!(MemoryBridge::from_extensions(&relay("http://127.0.0.1:4100", "t")).is_some());
        for (endpoint, token) in [
            ("http://127.0.0.1:4100", ""),
            ("https://127.0.0.1:4100", "t"),
            ("http://10.0.0.1:4100", "t"),
            ("http://127.0.0.1:4100/base", "t"),
        ] {
            assert!(
                MemoryBridge::from_extensions(&relay(endpoint, token)).is_none(),
                "{endpoint}"
            );
        }
        assert!(MemoryBridge::from_extensions(&json!({"memory": null})).is_none());
    }

    #[tokio::test]
    async fn requests_outside_the_two_routes_are_refused_unsent() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let bridge = MemoryBridge::from_extensions(
            &json!({"memory": {"relay": {"endpoint": "http://127.0.0.1:9", "token": "t"}}}),
        )
        .unwrap();
        for params in [
            json!({"path": "/v1/../admin"}),
            json!({"path": "/admin"}),
            json!({"path": "/v1/Context"}),
            json!({"path": "/v1/context", "method": "GET"}),
            json!({"path": "/v1/context", "method": "DELETE"}),
            json!({"path": "/mcp", "body": "x".repeat(MAX_BODY + 1)}),
        ] {
            let reply = bridge.forward(&params).await;
            let message = reply["error"]["message"].as_str().unwrap();
            assert!(!message.contains("unavailable"), "{params}: {message}");
        }
    }
}

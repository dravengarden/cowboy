//! One memory request relayed over the authenticated Machine channel.
//!
//! A Provider on any Machine reaches its memory backend through this link, so
//! memory follows the transport every Machine already uses and needs no
//! network path, tunnel or credential of its own. The caller names only its
//! session; the Controller derives who it is, and the home Machine holds the
//! backend credential. No effect here is retried or replayed by the relay.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Largest request or response body carried over the Machine link.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The only request headers relayed: the MCP protocol binds them to the body.
pub const MCP_HEADERS: [&str; 3] = ["mcp-protocol-version", "mcp-method", "mcp-name"];
const MAX_HEADER_BYTES: usize = 1024;

/// Status the relay answers with when the backend could not be reached. It is
/// indistinguishable from a backend outage on purpose: every client must
/// already treat both as "continue without memory".
pub const UNAVAILABLE: u16 = 503;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// `/mcp` or `/v1/<action>`.
    pub path: String,
    /// JSON text; the relay never interprets it beyond its binding.
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.path == "/mcp"
                || self.path.strip_prefix("/v1/").is_some_and(|action| {
                    !action.is_empty()
                        && action.len() <= 64
                        && action
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                }),
            "unsupported memory path"
        );
        ensure!(
            self.body.len() <= MAX_BODY_BYTES,
            "memory request exceeds its limit"
        );
        ensure!(
            self.headers.iter().all(|(name, value)| {
                MCP_HEADERS.contains(&name.as_str()) && value.len() <= MAX_HEADER_BYTES
            }),
            "unsupported memory header"
        );
        Ok(())
    }
}

/// The session identity the Controller derived from its own records. The home
/// Machine maps it to the backend's binding; a client's claim never reaches it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub provider: String,
    pub session: String,
    /// The Machine that executes the session's tools.
    pub machine: String,
    pub workspace: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            status: UNAVAILABLE,
            body: r#"{"error":"memory_unavailable"}"#.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str) -> Request {
        Request {
            path: path.to_owned(),
            body: "{}".to_owned(),
            headers: Vec::new(),
        }
    }

    #[test]
    fn only_protocol_headers_are_relayed() {
        let mut allowed = request("/mcp");
        allowed.headers = vec![("mcp-method".to_owned(), "tools/call".to_owned())];
        allowed.validate().unwrap();
        let mut refused = request("/mcp");
        refused.headers = vec![("authorization".to_owned(), "Bearer x".to_owned())];
        assert!(refused.validate().is_err());
    }

    #[test]
    fn only_backend_routes_are_relayed() {
        for path in ["/mcp", "/v1/context", "/v1/observe"] {
            request(path).validate().unwrap();
        }
        for path in ["/", "/v1/", "/v1/../x", "/v1/a/b", "/healthz", "/mcp/x"] {
            assert!(request(path).validate().is_err(), "{path}");
        }
    }

    #[test]
    fn oversized_bodies_are_refused() {
        let mut oversized = request("/mcp");
        oversized.body = "x".repeat(MAX_BODY_BYTES + 1);
        assert!(oversized.validate().is_err());
    }
}

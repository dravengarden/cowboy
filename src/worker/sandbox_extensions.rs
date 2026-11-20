//! The session extensions an account-sandboxed unit (Antigravity) may use.
//! Its confined proxy cannot read the Machine's extension descriptor, so the
//! worker derives from it, once per launch: the exact MCP servers the proxy
//! may keep running on the target (Cowboy MCP and the folder's bindings, as
//! a Claude or Codex session runs them there), and whether memory is
//! bridged. Relay addresses, tokens and Machine paths stay with the worker.

use serde_json::{Value, json};

use crate::worker_execution::MemoryBridge;

/// What the worker holds for one sandboxed session's extensions.
pub(super) struct SandboxExtensions {
    /// Sent to the unit inside its execution descriptor.
    pub summary: Value,
    /// The exact argv of each target MCP server the ledger admits.
    pub servers: Vec<Vec<String>>,
    pub memory: Option<MemoryBridge>,
}

/// The session's extension descriptor (`COWBOY_SESSION_EXTENSIONS`), when the
/// Machine wrote one this worker can read.
pub(super) fn load() -> Option<Value> {
    let path = std::env::var_os("COWBOY_SESSION_EXTENSIONS")?;
    let bytes = crate::logs::storage::private_read(std::path::Path::new(&path), 65536).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    (value.get("schema") == Some(&json!(1))).then_some(value)
}

fn absolute(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| value.starts_with('/') && value.len() <= 4096 && !value.contains('\0'))
        .map(str::to_owned)
}

fn name(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
        .map(str::to_owned)
}

pub(super) fn derive(extensions: &Value) -> SandboxExtensions {
    let plan = extensions.get("mcp").filter(|plan| !plan.is_null());
    // The Cowboy CLI on the target: the binding plan's verified command, else
    // the one on the target's PATH (as Claude's runtime does).
    let command = plan
        .and_then(|plan| plan.get("command"))
        .and_then(absolute)
        .unwrap_or_else(|| "cowboy".to_owned());
    let mut servers = Vec::new();
    let mut shown = Vec::new();
    // Cowboy MCP needs the session's call context, which the target keeper
    // supplies when the session has calls.
    if extensions
        .get("calls")
        .is_some_and(|calls| !calls.is_null())
    {
        let argv = vec![command.clone(), "mcp".to_owned()];
        shown.push(json!({"name": "cowboy", "argv": argv}));
        servers.push(argv);
    }
    for binding in plan
        .and_then(|plan| plan.get("bindings"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(8)
    {
        let (Some(binding_name), Some(descriptor)) = (
            binding.get("name").and_then(name),
            binding.get("descriptor").and_then(absolute),
        ) else {
            continue;
        };
        let argv = vec![
            command.clone(),
            "mcp".to_owned(),
            "--binding".to_owned(),
            descriptor,
        ];
        shown.push(json!({"name": binding_name, "argv": argv}));
        servers.push(argv);
    }
    let memory = MemoryBridge::from_extensions(extensions);
    let memory_summary = extensions
        .get("memory")
        .filter(|_| memory.is_some())
        .map(|memory| {
            let flag = |key: &str| memory.get(key).and_then(Value::as_bool).unwrap_or(false);
            json!({
                "server": memory.get("plugin").and_then(|plugin| plugin.get("id")).and_then(name),
                "tools": memory.get("tools").cloned().unwrap_or_else(|| json!([])),
                "enable_tools": flag("enable_tools"),
                "enable_recall": flag("enable_recall"),
                "observe": flag("observe"),
            })
        });
    let guidance = extensions
        .get("calls")
        .and_then(|calls| calls.get("profiles"))
        .and_then(Value::as_array)
        .and_then(|profiles| profiles.first())
        .and_then(|profile| profile.get("guidance"))
        .and_then(Value::as_str)
        .filter(|text| text.len() <= 4096)
        .map(str::to_owned);
    SandboxExtensions {
        summary: json!({
            "schema": 1,
            "servers": shown,
            "memory": memory_summary,
            "call_guidance": guidance,
        }),
        servers,
        memory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_learns_servers_and_memory_switches_but_never_the_relay() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let extensions = json!({
            "schema": 1,
            "mcp": {"schema": 1, "command": "/nix/store/x/bin/cowboy", "bindings": [
                {"name": "suger", "descriptor": "/private/suger.json"},
                {"name": "../bad", "descriptor": "/private/bad.json"},
            ]},
            "calls": {"profiles": [{"guidance": "Prefer Cowboy MCP."}]},
            "memory": {"plugin": {"id": "matrix"}, "tools": ["memory_search"],
                "enable_tools": true, "enable_recall": true, "observe": true,
                "relay": {"endpoint": "http://127.0.0.1:4100", "token": "secret-token",
                    "state_dir": "/home/ubuntu/private"}},
        });
        let derived = derive(&extensions);
        assert_eq!(
            derived.servers,
            vec![
                vec!["/nix/store/x/bin/cowboy".to_owned(), "mcp".to_owned()],
                vec![
                    "/nix/store/x/bin/cowboy".to_owned(),
                    "mcp".to_owned(),
                    "--binding".to_owned(),
                    "/private/suger.json".to_owned()
                ],
            ]
        );
        assert!(derived.memory.is_some());
        let text = derived.summary.to_string();
        assert!(!text.contains("secret-token") && !text.contains("4100"));
        assert!(!text.contains("/home/ubuntu/private"));
        assert_eq!(derived.summary["memory"]["server"], "matrix");
        assert_eq!(derived.summary["call_guidance"], "Prefer Cowboy MCP.");
    }

    #[test]
    fn without_calls_or_a_plan_nothing_runs_on_the_target() {
        let derived = derive(&json!({"schema": 1, "mcp": null, "calls": null, "memory": null}));
        assert!(derived.servers.is_empty());
        assert!(derived.memory.is_none());
        assert_eq!(derived.summary["servers"], json!([]));
        let calls_only = derive(&json!({"schema": 1, "calls": {"profiles": []}}));
        assert_eq!(
            calls_only.servers,
            vec![vec!["cowboy".to_owned(), "mcp".to_owned()]]
        );
    }
}

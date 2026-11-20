//! The session extensions of an account-sandboxed unit (Antigravity),
//! derived once per launch from the worker's own extension descriptor (the
//! frozen copy `mcp_launch` verified, when the session has MCP bindings).
//! The worker runs the target MCP servers and memory's tools itself
//! (`worker_execution::extension_broker`); the unit learns only what its
//! proxy needs: memory's recall and learning switches, its own outbox key
//! and the installed call guidance. Relay addresses, tokens, Machine paths
//! and server commands stay with the worker.

use serde_json::{Value, json};

use crate::worker_execution::{ExtensionPlan, MemoryBridge, ServerPlan};

/// What the worker holds for one sandboxed session's extensions.
pub(super) struct SandboxExtensions {
    /// Sent to the unit inside its execution descriptor.
    pub summary: Value,
    pub plan: ExtensionPlan,
    pub memory: Option<MemoryBridge>,
}

/// The session's extension descriptor: `frozen` (the copy `mcp_launch`
/// verified) when there is one, else the Machine's file for this worker.
pub(super) fn load(frozen: Option<&str>) -> Option<Value> {
    let path = frozen
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("COWBOY_SESSION_EXTENSIONS"))?;
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

/// `bindings_admitted`: the launch carried an MCP plan `mcp_launch` froze
/// and verified; only then do the descriptor's bindings run. A missing
/// descriptor still leaves the session Cowboy MCP.
pub(super) fn derive(
    extensions: Option<&Value>,
    bindings_admitted: bool,
    session_id: &str,
) -> SandboxExtensions {
    let empty = json!({});
    let extensions = extensions.unwrap_or(&empty);
    let plan = extensions
        .get("mcp")
        .filter(|plan| bindings_admitted && !plan.is_null());
    // The Cowboy CLI on the target: the binding plan's verified command, else
    // the one on the target's PATH, as Claude's and Codex's runtimes use.
    let command = plan
        .and_then(|plan| plan.get("command"))
        .and_then(absolute)
        .unwrap_or_else(|| "cowboy".to_owned());
    // An ordinary bound session always has Cowboy MCP; the target keeper
    // adds the session's call context to every process it starts.
    let mut servers = vec![ServerPlan {
        name: "cowboy".to_owned(),
        argv: vec![command.clone(), "mcp".to_owned()],
        required: false,
    }];
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
        servers.push(ServerPlan {
            name: binding_name,
            argv: vec![
                command.clone(),
                "mcp".to_owned(),
                "--binding".to_owned(),
                descriptor,
            ],
            required: true,
        });
    }
    let memory = MemoryBridge::from_extensions(extensions);
    let settings = extensions.get("memory").filter(|_| memory.is_some());
    let flag = |key: &str| {
        settings
            .and_then(|memory| memory.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let memory_tools = if flag("enable_tools") {
        settings
            .and_then(|memory| memory.get("tools"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(name)
            .take(32)
            .collect()
    } else {
        Vec::new()
    };
    let memory_server = settings
        .and_then(|memory| memory.get("plugin"))
        .and_then(|plugin| plugin.get("id"))
        .and_then(name)
        .unwrap_or_else(|| "matrix".to_owned());
    // Every installed profile's guidance (sent with the tool list, never in
    // the size-bounded descriptor); the proxy fills its placeholders.
    let guidance: Vec<String> = extensions
        .get("calls")
        .and_then(|calls| calls.get("profiles"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(8)
        .filter_map(|profile| profile.get("guidance").and_then(Value::as_str))
        .filter(|text| text.len() <= 4096)
        .map(str::to_owned)
        .collect();
    SandboxExtensions {
        summary: json!({
            "schema": 2,
            "session": session_id,
            "memory": settings.map(|_| json!({
                "enable_recall": flag("enable_recall"),
                "observe": flag("observe"),
                "tools": !memory_tools.is_empty(),
            })),
        }),
        plan: ExtensionPlan {
            servers,
            memory_tools: memory_tools.clone(),
            memory_server,
            guidance,
        },
        memory,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_learns_switches_and_guidance_but_never_the_relay_or_commands() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let extensions = json!({
            "schema": 1,
            "mcp": {"schema": 1, "command": "/nix/store/x/bin/cowboy", "bindings": [
                {"name": "suger", "descriptor": "/private/suger.json"},
                {"name": "../bad", "descriptor": "/private/bad.json"},
            ]},
            "calls": {"profiles": [{"guidance": "Prefer Cowboy MCP."}, {"guidance": "Two."}]},
            "memory": {"plugin": {"id": "matrix"}, "tools": ["memory_search", "Bad Name"],
                "enable_tools": true, "enable_recall": true, "observe": false,
                "relay": {"endpoint": "http://127.0.0.1:4100", "token": "secret-token",
                    "state_dir": "/home/ubuntu/private"}},
        });
        let derived = derive(Some(&extensions), true, "sess-1");
        assert_eq!(
            derived.plan.servers,
            vec![
                ServerPlan {
                    name: "cowboy".into(),
                    argv: vec!["/nix/store/x/bin/cowboy".into(), "mcp".into()],
                    required: false,
                },
                ServerPlan {
                    name: "suger".into(),
                    argv: vec![
                        "/nix/store/x/bin/cowboy".into(),
                        "mcp".into(),
                        "--binding".into(),
                        "/private/suger.json".into()
                    ],
                    required: true,
                },
            ]
        );
        assert_eq!(derived.plan.memory_tools, vec!["memory_search".to_owned()]);
        assert!(derived.memory.is_some());
        let text = derived.summary.to_string();
        for secret in [
            "secret-token",
            "4100",
            "/home/ubuntu/private",
            "/nix/store",
            "/private/",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
        assert_eq!(derived.summary["session"], "sess-1");
        assert_eq!(derived.summary["memory"]["enable_recall"], true);
        assert_eq!(derived.summary["memory"]["observe"], false);
        assert_eq!(derived.plan.guidance.len(), 2);
        assert!(derived.summary.get("calls").is_none());
        // Without an admitted plan the descriptor's bindings never run.
        let unplanned = derive(Some(&extensions), false, "sess-1");
        assert_eq!(unplanned.plan.servers.len(), 1);
        assert_eq!(
            unplanned.plan.servers[0].argv,
            vec!["cowboy".to_owned(), "mcp".to_owned()]
        );
    }

    #[test]
    fn a_session_without_bindings_or_memory_still_has_cowboy_mcp() {
        let derived = derive(None, false, "s");
        assert_eq!(derived.plan.servers.len(), 1);
        assert_eq!(
            derived.plan.servers[0].argv,
            vec!["cowboy".to_owned(), "mcp".to_owned()]
        );
        assert!(derived.plan.memory_tools.is_empty());
        assert!(derived.memory.is_none());
        assert!(derived.summary["memory"].is_null());
    }
}

//! Provider registry.
//!
//! Transport is uniformly ACP (design §2), so a provider's only per-agent
//! difference at this layer is *how to launch its ACP adapter over stdio* plus
//! a couple of capability flags. Adding a provider = add a [`LaunchSpec`] here;
//! the generic ACP backend in [`crate::acp`] does the rest.

use std::collections::HashMap;

/// How to spawn one provider's ACP adapter as a subprocess.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// Stable provider id, e.g. `"claude-code"`.
    pub id: &'static str,
    /// Executable to run.
    pub command: String,
    /// Arguments passed to the executable.
    pub args: Vec<String>,
    /// Whether the agent can resume a prior session via `session/load`
    /// (design §7 restart recovery). Informational for now.
    pub resume: bool,
}

/// Built-in providers. claude-code and codex first (design build order).
///
/// - `claude-code`: the `@agentclientprotocol/claude-agent-acp` adapter (the
///   renamed `@zed-industries/claude-code-acp`), run via `npx`. Speaks ACP over
///   NDJSON on stdio. Requires Claude auth in the environment (e.g.
///   `ANTHROPIC_API_KEY` or a prior `claude` login).
/// - `codex`: the `@zed-industries/codex-acp` adapter, run via `npx`. Wraps the
///   Codex CLI. Requires Codex auth (ChatGPT subscription login in `~/.codex`,
///   or `CODEX_API_KEY` / `OPENAI_API_KEY`).
#[must_use]
pub fn builtin() -> HashMap<&'static str, LaunchSpec> {
    let mut m = HashMap::new();
    m.insert(
        "claude-code",
        LaunchSpec {
            id: "claude-code",
            command: "npx".into(),
            args: vec!["-y".into(), "@agentclientprotocol/claude-agent-acp".into()],
            resume: true,
        },
    );
    m.insert(
        "codex",
        LaunchSpec {
            id: "codex",
            command: "npx".into(),
            args: vec!["-y".into(), "@zed-industries/codex-acp".into()],
            resume: false,
        },
    );
    m
}

/// Look up a built-in provider's launch spec by id.
#[must_use]
pub fn lookup(id: &str) -> Option<LaunchSpec> {
    builtin().remove(id)
}

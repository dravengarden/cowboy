//! Provider registry.
//!
//! Transport is uniformly ACP (design §2), so a provider's only per-agent
//! difference at this layer is *how to launch its ACP adapter over stdio* plus
//! a couple of capability flags. Adding a provider = add a [`LaunchSpec`] here;
//! the generic ACP backend in [`crate::acp`] does the rest.

use std::collections::HashMap;

// Per-provider specifics beyond launching. Today each holds its L1 confirm-detect
// (the volatile, often-changing turn-end markers — design §B), sharing the
// portable stop-reason rule in `confirm`.
pub mod claude_code;
pub mod codex;
pub mod confirm;
pub mod gemini;

/// How to spawn one provider's ACP adapter as a subprocess.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// Stable provider id, e.g. `"claude-code"`.
    pub id: &'static str,
    /// Executable to run.
    pub command: String,
    /// Arguments passed to the executable.
    pub args: Vec<String>,
}

const CODEX_FULL_ACCESS_ARGS: &[&str] = &[
    "-c",
    "approval_policy=\"never\"",
    "-c",
    "sandbox_mode=\"danger-full-access\"",
];

// Note: whether an agent can resume via `session/load` (design §7) is read at
// runtime from its `initialize` response (`agent_capabilities.load_session` —
// see `crate::acp::agent_main`), which is authoritative, so it isn't duplicated
// as a static flag here.

/// Built-in providers. claude-code and codex first (design build order).
///
/// - `claude-code`: the `@agentclientprotocol/claude-agent-acp` adapter (the
///   renamed `@zed-industries/claude-code-acp`), run via `npx`. Speaks ACP over
///   NDJSON on stdio. Requires Claude auth in the environment (e.g.
///   `ANTHROPIC_API_KEY` or a prior `claude` login).
/// - `codex`: the `@zed-industries/codex-acp` adapter, run via `npx`. Wraps the
///   Codex CLI. Requires Codex auth (`ChatGPT` subscription login in `~/.codex`,
///   or `CODEX_API_KEY` / `OPENAI_API_KEY`).
/// - `gemini`: the Gemini CLI's own ACP mode — `@google/gemini-cli --acp`, run via
///   `npx` (the CLI is the adapter; no separate package). Requires Gemini auth (a
///   prior `gemini` OAuth login in `~/.gemini`, or `GEMINI_API_KEY`).
#[must_use]
pub fn builtin() -> HashMap<&'static str, LaunchSpec> {
    let mut m = HashMap::new();
    m.insert(
        "claude-code",
        spec(
            "claude-code",
            "npx",
            &["-y", "@agentclientprotocol/claude-agent-acp"],
        ),
    );
    m.insert(
        "codex",
        spec_with_custom_default_args(
            "codex",
            "npx",
            &concat_slices(&["-y", "@zed-industries/codex-acp"], CODEX_FULL_ACCESS_ARGS),
            CODEX_FULL_ACCESS_ARGS,
        ),
    );
    m.insert(
        "gemini",
        // The Gemini CLI IS the ACP adapter (`--acp` starts ACP mode); there's no
        // separate npm package like the others.
        spec("gemini", "npx", &["-y", "@google/gemini-cli", "--acp"]),
    );
    m
}

/// Build a provider's launch spec, letting the deployment OVERRIDE how the ACP
/// adapter is launched via env — `COWBOY_ACP_<ID>_CMD` (+ optional
/// whitespace-split `COWBOY_ACP_<ID>_ARGS`), where `<ID>` is the upper-cased id
/// with `-`→`_` (e.g. `COWBOY_ACP_CLAUDE_CODE_CMD`).
///
/// Why: the default `npx -y <pkg>` cold-installs the adapter into the shared
/// `~/.npm/_npx` cache on EVERY session start. Concurrent starts race npm's
/// atomic rename (ENOTEMPTY → the adapter exits 217 → the session crashes), an
/// interrupted install leaves stale staging dirs that poison every later start,
/// and each start pays a registry round-trip. Pointing this at a PRE-INSTALLED
/// adapter binary (the hawk `services/cowboy` module, matching the host's
/// bootstrap-wrapper convention for the CLIs) removes `npx` from the hot path
/// entirely — no install-at-spawn, no race, no poison, no network dependency.
/// Unset ⇒ the npx default. A provider may still add adapter-specific default
/// flags that are independent from the npx wrapper itself.
fn spec(id: &'static str, default_cmd: &str, default_args: &[&str]) -> LaunchSpec {
    spec_with_custom_default_args(id, default_cmd, default_args, &[])
}

fn concat_slices(left: &[&'static str], right: &[&'static str]) -> Vec<&'static str> {
    left.iter().chain(right).copied().collect()
}

fn spec_with_custom_default_args(
    id: &'static str,
    default_cmd: &str,
    default_args: &[&str],
    custom_default_args: &[&str],
) -> LaunchSpec {
    let key = id.to_uppercase().replace('-', "_");
    let arg_override = std::env::var(format!("COWBOY_ACP_{key}_ARGS"))
        .ok()
        .map(|s| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>());
    match std::env::var(format!("COWBOY_ACP_{key}_CMD")) {
        // A custom command replaces npx: the npx-specific prefix (`-y <pkg>`)
        // does NOT carry over. Provider-specific args may still apply, e.g.
        // Codex's default full-access config for a pre-installed adapter.
        Ok(command) => LaunchSpec {
            id,
            command,
            args: arg_override.unwrap_or_else(|| {
                custom_default_args
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
            }),
        },
        // Default command (npx): `_ARGS` may still override the pinned adapter args.
        Err(_) => LaunchSpec {
            id,
            command: default_cmd.to_owned(),
            args: arg_override
                .unwrap_or_else(|| default_args.iter().map(|s| (*s).to_owned()).collect()),
        },
    }
}

/// Look up a built-in provider's launch spec by id.
#[must_use]
pub fn lookup(id: &str) -> Option<LaunchSpec> {
    builtin().remove(id)
}

#[cfg(test)]
mod tests {
    // Defaults AND the env override, in ONE test: the override sets a
    // process-global env var, so keeping both assertions in a single (serial)
    // function avoids racing a separate defaults test running in parallel.
    #[test]
    fn defaults_and_env_override() {
        // Hermetic: clear any AMBIENT override first. In production the daemon
        // sets these (services/cowboy), and a cowboy-spawned test process inherits
        // them — without this the "default" assertions below see the deployed
        // /opt/npm-global paths instead of npx.
        for k in [
            "COWBOY_ACP_CLAUDE_CODE_CMD",
            "COWBOY_ACP_CLAUDE_CODE_ARGS",
            "COWBOY_ACP_CODEX_CMD",
            "COWBOY_ACP_CODEX_ARGS",
            "COWBOY_ACP_GEMINI_CMD",
            "COWBOY_ACP_GEMINI_ARGS",
        ] {
            std::env::remove_var(k);
        }

        // Default (no env): npx + the pinned adapter args; unknown id → None.
        let claude = super::lookup("claude-code").expect("claude-code registered");
        assert_eq!(claude.command, "npx");
        assert_eq!(claude.args, ["-y", "@agentclientprotocol/claude-agent-acp"]);
        let codex = super::lookup("codex").expect("codex registered");
        assert_eq!(codex.command, "npx");
        assert_eq!(
            codex.args,
            [
                "-y",
                "@zed-industries/codex-acp",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "sandbox_mode=\"danger-full-access\"",
            ]
        );
        assert_eq!(
            super::lookup("gemini").map(|s| s.command),
            Some("npx".to_owned())
        );
        assert!(super::lookup("nope").is_none());

        // Override just _CMD: npx-specific args are dropped, while Codex keeps
        // its provider-specific full-access config for the pre-installed binary.
        std::env::set_var("COWBOY_ACP_CODEX_CMD", "/opt/npm-global/bin/codex-acp");
        let codex = super::lookup("codex").unwrap();
        assert_eq!(codex.command, "/opt/npm-global/bin/codex-acp");
        assert_eq!(
            codex.args,
            [
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "sandbox_mode=\"danger-full-access\"",
            ]
        );
        std::env::remove_var("COWBOY_ACP_CODEX_CMD");

        // Other custom commands still drop the npx-specific default args.
        std::env::set_var(
            "COWBOY_ACP_CLAUDE_CODE_CMD",
            "/opt/npm-global/bin/claude-agent-acp",
        );
        let o = super::lookup("claude-code").unwrap();
        assert_eq!(o.command, "/opt/npm-global/bin/claude-agent-acp");
        assert!(
            o.args.is_empty(),
            "custom command drops the npx default args"
        );

        // _ARGS overrides independently (e.g. gemini's `--acp`).
        std::env::set_var("COWBOY_ACP_CLAUDE_CODE_ARGS", "--acp --foo");
        assert_eq!(
            super::lookup("claude-code").unwrap().args,
            ["--acp", "--foo"]
        );

        std::env::remove_var("COWBOY_ACP_CLAUDE_CODE_CMD");
        std::env::remove_var("COWBOY_ACP_CLAUDE_CODE_ARGS");
        // Back to the default once unset.
        assert_eq!(super::lookup("claude-code").unwrap().command, "npx");
    }
}

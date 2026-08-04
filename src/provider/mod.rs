//! Provider registry.
//!
//! Transport is uniformly ACP (design §2), so a provider's only per-agent
//! difference at this layer is *how to launch its ACP adapter over stdio* plus
//! a couple of capability flags. Adding a provider = add a [`LaunchSpec`] here;
//! the generic ACP backend in [`crate::acp`] does the rest.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// Environment additions scoped to this adapter subprocess.
    pub env: HashMap<String, String>,
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
//
// TODO(acp-side-conversation): expose Codex `/side` / `/btw` only after the
// official codex-acp adapter advertises a structured side-conversation
// capability over ACP. Do not send the TUI-only slash command as a normal
// prompt and do not emulate it with Cowboy's queue or a visible session: those
// alternatives either disturb the active task or change the feature's
// ephemeral, transcript-isolated semantics. See docs/architecture/04-providers.md.

/// Built-in providers. claude-code and codex first (design build order).
///
/// - `claude-code`: the `@agentclientprotocol/claude-agent-acp` adapter (the
///   renamed `@zed-industries/claude-code-acp`), run via `npx`. Speaks ACP over
///   NDJSON on stdio. Requires Claude auth in the environment (e.g.
///   `ANTHROPIC_API_KEY` or a prior `claude` login).
/// - `codex`: the `@agentclientprotocol/codex-acp` adapter, run via `npx`.
///   Built on Codex App Server. Requires Codex auth (`ChatGPT` subscription
///   login in `~/.codex`, or `CODEX_API_KEY` / `OPENAI_API_KEY`).
/// - `gemini`: the Gemini CLI's own ACP mode — `@google/gemini-cli --acp`, run via
///   `npx` (the CLI is the adapter; no separate package). Requires Gemini auth (a
///   `GEMINI_API_KEY`, Vertex AI, or a Code Assist Standard/Enterprise Google
///   Login with `GOOGLE_CLOUD_PROJECT`. Consumer Google Login is retired and
///   belongs to Antigravity, which does not currently expose Cowboy's ACP
///   session transport.
#[must_use]
pub fn builtin() -> HashMap<&'static str, LaunchSpec> {
    builtin_with_env(|key| std::env::var(key).ok())
}

fn builtin_with_env(get_env: impl Fn(&str) -> Option<String>) -> HashMap<&'static str, LaunchSpec> {
    let mut m = HashMap::new();
    m.insert(
        "claude-code",
        spec(
            "claude-code",
            "npx",
            &["-y", "@agentclientprotocol/claude-agent-acp"],
            &get_env,
        ),
    );
    m.insert(
        "codex",
        spec_with_custom_default_args(
            "codex",
            "npx",
            &concat_slices(
                &["-y", "@agentclientprotocol/codex-acp"],
                CODEX_FULL_ACCESS_ARGS,
            ),
            CODEX_FULL_ACCESS_ARGS,
            &get_env,
        ),
    );
    let mut deepseek = spec_with_custom_default_args(
        "codex-deepseek",
        "npx",
        &["-y", "@agentclientprotocol/codex-acp"],
        &[],
        &get_env,
    );
    // Reuse the installed Codex ACP adapter when the host already supplies it.
    // The inference endpoint itself remains a separate, independently deployed
    // process; only this worker-local configuration points Codex at it.
    if get_env("COWBOY_ACP_CODEX_DEEPSEEK_CMD").is_none()
        && let Some(command) = get_env("COWBOY_ACP_CODEX_CMD")
    {
        deepseek.command = command;
        if get_env("COWBOY_ACP_CODEX_DEEPSEEK_ARGS").is_none() {
            deepseek.args.clear();
        }
    }
    deepseek
        .env
        .insert("MODEL_PROVIDER".to_owned(), "deepseek-local".to_owned());
    deepseek.env.insert(
        "CODEX_CONFIG".to_owned(),
        serde_json::json!({
            "model": "deepseek-v4-flash",
            "model_provider": "deepseek-local",
            "model_reasoning_effort": "high",
            "model_catalog_json": "/etc/codex-deepseek/codex-models.json",
            "model_providers": {
                "deepseek-local": {
                    "name": "Local DeepSeek Responses gateway",
                    "base_url": "http://127.0.0.1:8088/v1",
                    "wire_api": "responses",
                    "request_max_retries": 1,
                    "stream_max_retries": 0,
                    "stream_idle_timeout_ms": 600000
                }
            }
        })
        .to_string(),
    );
    m.insert("codex-deepseek", deepseek);
    m.insert(
        "gemini",
        // The Gemini CLI IS the ACP adapter (`--acp` starts ACP mode); there's no
        // separate npm package like the others.
        spec(
            "gemini",
            "npx",
            &["-y", "@google/gemini-cli", "--acp"],
            &get_env,
        ),
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
fn spec(
    id: &'static str,
    default_cmd: &str,
    default_args: &[&str],
    get_env: &impl Fn(&str) -> Option<String>,
) -> LaunchSpec {
    spec_with_custom_default_args(id, default_cmd, default_args, &[], get_env)
}

fn concat_slices(left: &[&'static str], right: &[&'static str]) -> Vec<&'static str> {
    left.iter().chain(right).copied().collect()
}

fn spec_with_custom_default_args(
    id: &'static str,
    default_cmd: &str,
    default_args: &[&str],
    custom_default_args: &[&str],
    get_env: &impl Fn(&str) -> Option<String>,
) -> LaunchSpec {
    let key = id.to_uppercase().replace('-', "_");
    let arg_override = get_env(&format!("COWBOY_ACP_{key}_ARGS"))
        .map(|s| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>());
    match get_env(&format!("COWBOY_ACP_{key}_CMD")) {
        // A custom command replaces npx: the npx-specific prefix (`-y <pkg>`)
        // does NOT carry over. Provider-specific args may still apply, e.g.
        // Codex's default full-access config for a pre-installed adapter.
        Some(command) => LaunchSpec {
            id,
            command,
            args: arg_override.unwrap_or_else(|| {
                custom_default_args
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
            }),
            env: HashMap::new(),
        },
        // Default command (npx): `_ARGS` may still override the pinned adapter args.
        None => LaunchSpec {
            id,
            command: default_cmd.to_owned(),
            args: arg_override
                .unwrap_or_else(|| default_args.iter().map(|s| (*s).to_owned()).collect()),
            env: HashMap::new(),
        },
    }
}

/// Look up a built-in provider's launch spec by id.
#[must_use]
pub fn lookup(id: &str) -> Option<LaunchSpec> {
    let mut spec = builtin().remove(id)?;
    if id == "codex-deepseek" {
        match prepare_codex_deepseek_home() {
            Ok(home) => {
                spec.env
                    .insert("CODEX_HOME".to_owned(), home.display().to_string());
            }
            Err(error) => {
                tracing::warn!(%error, "failed to prepare isolated Codex DeepSeek home");
                return None;
            }
        }
    }
    Some(spec)
}

/// Whether a Cowboy provider uses the Codex ACP/runtime semantics.
#[must_use]
pub fn is_codex(id: &str) -> bool {
    matches!(id, "codex" | "codex-deepseek")
}

fn prepare_codex_deepseek_home() -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _, symlink};

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/draven"));
    let source = home.join(".codex");
    let target = home.join(".local/state/cowboy/codex-deepseek-home");
    std::fs::create_dir_all(&target)?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))?;

    let base = match std::fs::read_to_string(source.join("config.toml")) {
        Ok(config) => config,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let config = render_codex_deepseek_config(&base);
    static NEXT_CONFIG_WRITE: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_CONFIG_WRITE.fetch_add(1, Ordering::Relaxed);
    let temporary = target.join(format!(".config.toml.{}.{sequence}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(config.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&temporary, target.join("config.toml"))?;

    for entry in [
        "AGENTS.md",
        "skills",
        "plugins",
        "rules",
        "memories",
        "memories_extensions",
    ] {
        let link = target.join(entry);
        if !link.exists() && !link.is_symlink() {
            let _ = symlink(source.join(entry), link);
        }
    }
    Ok(target)
}

fn render_codex_deepseek_config(base: &str) -> String {
    let mut filtered = String::new();
    let mut in_root = true;
    let mut skip_managed_provider = false;
    for line in base.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_root = false;
            skip_managed_provider = trimmed == "[model_providers.deepseek-local]"
                || trimmed.starts_with("[model_providers.deepseek-local.");
            if skip_managed_provider {
                continue;
            }
        } else if skip_managed_provider {
            continue;
        }
        let key = line.split_once('=').map(|(key, _)| key.trim());
        if in_root
            && matches!(
                key,
                Some("model" | "model_provider" | "model_reasoning_effort" | "model_catalog_json")
            )
        {
            continue;
        }
        filtered.push_str(line);
        filtered.push('\n');
    }
    format!(
        "model = \"deepseek-v4-flash\"\n\
         model_provider = \"deepseek-local\"\n\
         model_reasoning_effort = \"high\"\n\
         model_catalog_json = \"/etc/codex-deepseek/codex-models.json\"\n\n\
         {filtered}\n\
         [model_providers.deepseek-local]\n\
         name = \"Local DeepSeek Responses gateway\"\n\
         base_url = \"http://127.0.0.1:8088/v1\"\n\
         wire_api = \"responses\"\n\
         request_max_retries = 1\n\
         stream_max_retries = 0\n\
         stream_idle_timeout_ms = 600000\n"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn lookup_with(overrides: &[(&str, &str)], id: &str) -> Option<super::LaunchSpec> {
        let overrides: HashMap<_, _> = overrides.iter().copied().collect();
        super::builtin_with_env(|key| overrides.get(key).map(|value| (*value).to_owned()))
            .remove(id)
    }

    #[test]
    fn defaults_and_env_override() {
        // Default (no env): npx + the pinned adapter args; unknown id → None.
        let claude = lookup_with(&[], "claude-code").expect("claude-code registered");
        assert_eq!(claude.command, "npx");
        assert_eq!(claude.args, ["-y", "@agentclientprotocol/claude-agent-acp"]);
        let codex = lookup_with(&[], "codex").expect("codex registered");
        assert_eq!(codex.command, "npx");
        assert_eq!(
            codex.args,
            [
                "-y",
                "@agentclientprotocol/codex-acp",
                "-c",
                "approval_policy=\"never\"",
                "-c",
                "sandbox_mode=\"danger-full-access\"",
            ]
        );
        assert_eq!(
            lookup_with(&[], "gemini").map(|s| s.command),
            Some("npx".to_owned())
        );
        assert!(lookup_with(&[], "nope").is_none());

        let deepseek = lookup_with(
            &[("COWBOY_ACP_CODEX_CMD", "/opt/npm-global/bin/codex-acp")],
            "codex-deepseek",
        )
        .expect("codex-deepseek registered");
        assert_eq!(deepseek.command, "/opt/npm-global/bin/codex-acp");
        assert!(deepseek.args.is_empty());
        assert_eq!(
            deepseek.env.get("MODEL_PROVIDER").map(String::as_str),
            Some("deepseek-local")
        );
        let config: serde_json::Value =
            serde_json::from_str(deepseek.env.get("CODEX_CONFIG").expect("Codex config"))
                .expect("valid Codex config JSON");
        assert_eq!(config["model"], "deepseek-v4-flash");

        let deepseek_with_args = lookup_with(
            &[
                ("COWBOY_ACP_CODEX_CMD", "/opt/npm-global/bin/codex-acp"),
                ("COWBOY_ACP_CODEX_DEEPSEEK_ARGS", "--one --two"),
            ],
            "codex-deepseek",
        )
        .expect("codex-deepseek registered");
        assert_eq!(deepseek_with_args.args, ["--one", "--two"]);

        // Override just _CMD: npx-specific args are dropped, while Codex keeps
        // its provider-specific full-access config for the pre-installed binary.
        let codex = lookup_with(
            &[("COWBOY_ACP_CODEX_CMD", "/opt/npm-global/bin/codex-acp")],
            "codex",
        )
        .unwrap();
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
        // Other custom commands still drop the npx-specific default args.
        let o = lookup_with(
            &[(
                "COWBOY_ACP_CLAUDE_CODE_CMD",
                "/opt/npm-global/bin/claude-agent-acp",
            )],
            "claude-code",
        )
        .unwrap();
        assert_eq!(o.command, "/opt/npm-global/bin/claude-agent-acp");
        assert!(
            o.args.is_empty(),
            "custom command drops the npx default args"
        );

        // _ARGS overrides independently (e.g. gemini's `--acp`).
        assert_eq!(
            lookup_with(
                &[
                    (
                        "COWBOY_ACP_CLAUDE_CODE_CMD",
                        "/opt/npm-global/bin/claude-agent-acp",
                    ),
                    ("COWBOY_ACP_CLAUDE_CODE_ARGS", "--acp --foo"),
                ],
                "claude-code",
            )
            .unwrap()
            .args,
            ["--acp", "--foo"]
        );
    }

    #[test]
    fn deepseek_overlay_preserves_base_config_without_base_model_defaults() {
        let rendered = super::render_codex_deepseek_config(
            "approval_policy = \"never\"\nmodel = \"gpt\"\n\n\
             [model_providers.deepseek-local]\nbase_url = \"stale\"\n\n\
             [features]\nmemories = true\n",
        );
        assert!(rendered.starts_with("model = \"deepseek-v4-flash\""));
        assert!(!rendered.contains("model = \"gpt\""));
        assert!(rendered.contains("approval_policy = \"never\""));
        assert!(rendered.contains("[features]\nmemories = true"));
        assert!(rendered.contains("[model_providers.deepseek-local]"));
        assert_eq!(
            rendered.matches("[model_providers.deepseek-local]").count(),
            1
        );
        assert!(!rendered.contains("base_url = \"stale\""));
    }
}

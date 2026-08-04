//! Provider registry.
//!
//! Transport is uniformly ACP (design §2), so a provider's only per-agent
//! difference at this layer is *how to launch its ACP adapter over stdio* plus
//! a couple of capability flags. Adding a provider = add a [`LaunchSpec`] here;
//! the generic ACP backend in [`crate::acp`] does the rest.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::provider_catalog::{CODEX_DEEPSEEK_CATALOG, available_codex_deepseek_catalog};

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
    /// Inherited variables that must not cross this provider boundary.
    pub remove_env: Vec<&'static str>,
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
    deepseek.remove_env = vec![
        "CODEX_ACCESS_TOKEN",
        "CODEX_API_KEY",
        "CODEX_AUTH",
        "CODEX_AUTHAPI_BASE_URL",
        "CODEX_CLOUD_TASKS_BASE_URL",
        "CODEX_CONFIG",
        "CODEX_CONNECTORS_TOKEN",
        "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
        "CODEX_REVOKE_TOKEN_URL_OVERRIDE",
        "CODEX_URL",
        "OPENAI_API_KEY",
        "OPENAI_API_BASE",
        "OPENAI_BASE_URL",
        "OPENAI_ORG_ID",
        "OPENAI_ORGANIZATION",
        "OPENAI_PROJECT_ID",
        "CHATGPT_BASE_URL",
        "DEEPSEEK_API_KEY",
        "DEEPSEEK_API_KEY_FILE",
    ];
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
            remove_env: Vec::new(),
        },
        // Default command (npx): `_ARGS` may still override the pinned adapter args.
        None => LaunchSpec {
            id,
            command: default_cmd.to_owned(),
            args: arg_override
                .unwrap_or_else(|| default_args.iter().map(|s| (*s).to_owned()).collect()),
            env: HashMap::new(),
            remove_env: Vec::new(),
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
    let user_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/draven"));
    prepare_codex_deepseek_home_at(&user_home)
}

fn prepare_codex_deepseek_home_at(user_home: &Path) -> std::io::Result<PathBuf> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let target = user_home.join(".local/state/cowboy/providers/codex-deepseek/codex-home");
    std::fs::create_dir_all(&target)?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))?;

    let catalog =
        available_codex_deepseek_catalog().unwrap_or_else(|| PathBuf::from(CODEX_DEEPSEEK_CATALOG));
    let config = render_codex_deepseek_config(&catalog);
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
    Ok(target)
}

fn render_codex_deepseek_config(catalog: &Path) -> String {
    format!(
        "model = \"deepseek-v4-flash\"\n\
     model_provider = \"deepseek-local\"\n\
     model_reasoning_effort = \"high\"\n\
     model_catalog_json = \"{}\"\n\
     approval_policy = \"never\"\n\
     sandbox_mode = \"danger-full-access\"\n\n\
     [model_providers.deepseek-local]\n\
     name = \"Isolated DeepSeek Responses gateway\"\n\
     base_url = \"http://127.0.0.1:8088/v1\"\n\
     wire_api = \"responses\"\n\
     requires_openai_auth = false\n\
     request_max_retries = 1\n\
     stream_max_retries = 0\n\
     stream_idle_timeout_ms = 600000\n",
        catalog.display()
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
        assert!(!deepseek.env.contains_key("CODEX_CONFIG"));
        assert!(deepseek.remove_env.contains(&"CODEX_ACCESS_TOKEN"));
        assert!(deepseek.remove_env.contains(&"CODEX_AUTH"));
        assert!(deepseek.remove_env.contains(&"OPENAI_API_KEY"));
        assert!(deepseek.remove_env.contains(&"OPENAI_ORGANIZATION"));
        assert!(deepseek.remove_env.contains(&"CODEX_CONFIG"));
        assert!(deepseek.remove_env.contains(&"DEEPSEEK_API_KEY"));

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
    fn deepseek_config_is_self_contained() {
        let rendered = super::render_codex_deepseek_config(std::path::Path::new(
            super::CODEX_DEEPSEEK_CATALOG,
        ));
        assert!(rendered.starts_with("model = \"deepseek-v4-flash\""));
        assert!(rendered.contains("approval_policy = \"never\""));
        assert!(rendered.contains("[model_providers.deepseek-local]"));
        assert!(rendered.contains("requires_openai_auth = false"));
        assert!(rendered.contains("/nix/var/nix/profiles/columbus-components/codex-deepseek/"));
        assert!(!rendered.contains("api.openai.com"));
    }

    #[test]
    fn deepseek_home_never_reads_or_links_openai_codex_state() {
        use std::os::unix::fs::PermissionsExt as _;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_TEST_HOME: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "cowboy-codex-deepseek-isolation-{}-{}",
            std::process::id(),
            NEXT_TEST_HOME.fetch_add(1, Ordering::Relaxed)
        ));
        let openai_home = root.join(".codex");
        std::fs::create_dir_all(&openai_home).expect("create OpenAI Codex home");
        let sentinel =
            "model = \"gpt-secret-sentinel\"\n[mcp_servers.private]\ncommand = \"secret\"\n";
        std::fs::write(openai_home.join("config.toml"), sentinel).expect("write OpenAI config");
        std::fs::write(openai_home.join("auth.json"), "openai-auth-sentinel")
            .expect("write OpenAI auth");

        let isolated = super::prepare_codex_deepseek_home_at(&root).expect("prepare DeepSeek home");
        let config =
            std::fs::read_to_string(isolated.join("config.toml")).expect("read DeepSeek config");
        assert!(!config.contains("gpt-secret-sentinel"));
        assert!(!config.contains("mcp_servers.private"));
        assert!(!config.contains("openai-auth-sentinel"));
        assert_eq!(
            std::fs::read_to_string(openai_home.join("config.toml")).unwrap(),
            sentinel
        );
        assert_eq!(
            std::fs::read_to_string(openai_home.join("auth.json")).unwrap(),
            "openai-auth-sentinel"
        );
        assert_eq!(
            std::fs::metadata(&isolated).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(isolated.join("config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let entries = std::fs::read_dir(&isolated)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(entries, [std::ffi::OsString::from("config.toml")]);

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }
}

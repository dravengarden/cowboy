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
    /// Inherited variable prefixes removed before provider-owned values are
    /// applied. This closes over newly added upstream variables instead of
    /// relying on a permanently complete hand-written name list.
    pub remove_env_prefixes: Vec<&'static str>,
}

impl LaunchSpec {
    #[must_use]
    pub fn removes_inherited_env(&self, key: &str) -> bool {
        self.remove_env.contains(&key)
            || self
                .remove_env_prefixes
                .iter()
                .any(|prefix| key.starts_with(prefix))
    }
}

// A compacted thread can retain a large carried prefix. Counting that immutable
// prefix again leaves almost no headroom and can make Codex compact after every
// tool call; only post-compaction growth should trigger the next auto-compact.
const CODEX_RUNTIME_ARGS: &[&str] = &[
    "-c",
    "approval_policy=\"never\"",
    "-c",
    "sandbox_mode=\"danger-full-access\"",
    "-c",
    "model_auto_compact_token_limit_scope=\"body_after_prefix\"",
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
    let claude_deepseek_shell = crate::claude_shell::resolve(&get_env);
    builtin_with_env_and_shell(get_env, claude_deepseek_shell)
}

fn builtin_with_env_and_shell(
    get_env: impl Fn(&str) -> Option<String>,
    claude_deepseek_shell: Option<String>,
) -> HashMap<&'static str, LaunchSpec> {
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
    let mut claude_deepseek = spec(
        "claude-deepseek",
        "npx",
        &["-y", "@agentclientprotocol/claude-agent-acp"],
        &get_env,
    );
    // Reuse the adapter executable, and the non-secret setup linked into the
    // provider config dir. Model routing, credentials, history, and the rest of
    // the Claude runtime state remain provider-owned.
    if get_env("COWBOY_ACP_CLAUDE_DEEPSEEK_CMD").is_none()
        && let Some(command) = get_env("COWBOY_ACP_CLAUDE_CODE_CMD")
    {
        claude_deepseek.command = command;
        if get_env("COWBOY_ACP_CLAUDE_DEEPSEEK_ARGS").is_none() {
            claude_deepseek.args.clear();
        }
    }
    claude_deepseek.env.extend([
        (
            "ANTHROPIC_BASE_URL".to_owned(),
            "http://127.0.0.1:61138".to_owned(),
        ),
        (
            "ANTHROPIC_AUTH_TOKEN".to_owned(),
            "cowboy-local-credential-boundary".to_owned(),
        ),
        (
            "ANTHROPIC_MODEL".to_owned(),
            "deepseek-v4-flash[1m]".to_owned(),
        ),
        (
            "ANTHROPIC_DEFAULT_OPUS_MODEL".to_owned(),
            "deepseek-v4-pro[1m]".to_owned(),
        ),
        (
            "ANTHROPIC_DEFAULT_SONNET_MODEL".to_owned(),
            "deepseek-v4-flash[1m]".to_owned(),
        ),
        (
            "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_owned(),
            "deepseek-v4-flash".to_owned(),
        ),
        (
            "CLAUDE_CODE_SUBAGENT_MODEL".to_owned(),
            "deepseek-v4-flash".to_owned(),
        ),
        (
            "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST".to_owned(),
            "cowboy-claude-deepseek".to_owned(),
        ),
        (
            "CLAUDE_CODE_ENABLE_FINE_GRAINED_TOOL_STREAMING".to_owned(),
            "1".to_owned(),
        ),
        (
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(),
            "1".to_owned(),
        ),
        (
            "CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK".to_owned(),
            "1".to_owned(),
        ),
        (
            "CLAUDE_CODE_DISABLE_OFFICIAL_MARKETPLACE_AUTOINSTALL".to_owned(),
            "1".to_owned(),
        ),
        (
            "CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL".to_owned(),
            "1".to_owned(),
        ),
        ("DISABLE_LOGIN_COMMAND".to_owned(), "1".to_owned()),
        ("DISABLE_LOGOUT_COMMAND".to_owned(), "1".to_owned()),
        ("DISABLE_UPGRADE_COMMAND".to_owned(), "1".to_owned()),
        ("ENABLE_CLAUDEAI_MCP_SERVERS".to_owned(), "false".to_owned()),
    ]);
    if let Some(shell) = claude_deepseek_shell {
        // `CLAUDE_CODE_SHELL` is the authoritative override. Also set `SHELL`
        // for subprocesses and older Claude Code releases that consult it.
        claude_deepseek
            .env
            .insert("CLAUDE_CODE_SHELL".to_owned(), shell.clone());
        claude_deepseek.env.insert("SHELL".to_owned(), shell);
    }
    claude_deepseek.remove_env_prefixes = vec!["ANTHROPIC_", "CLAUDE_", "DEEPSEEK_"];
    claude_deepseek.remove_env = vec![
        "API_TIMEOUT_MS",
        "COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL",
        "DISABLE_PROMPT_CACHING",
        "DISABLE_PROMPT_CACHING_HAIKU",
        "DISABLE_PROMPT_CACHING_OPUS",
        "DISABLE_PROMPT_CACHING_SONNET",
        "ENABLE_TOOL_SEARCH",
        "ENABLE_CLAUDEAI_MCP_SERVERS",
        "MAX_THINKING_TOKENS",
        "MCP_TIMEOUT",
        "MCP_TOOL_TIMEOUT",
    ];
    m.insert("claude-deepseek", claude_deepseek);
    m.insert(
        "codex",
        spec_with_custom_default_args(
            "codex",
            "npx",
            &concat_slices(
                &["-y", "@agentclientprotocol/codex-acp"],
                CODEX_RUNTIME_ARGS,
            ),
            CODEX_RUNTIME_ARGS,
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
    let mut reasonix_deepseek = spec_with_custom_default_args(
        "reasonix-deepseek",
        "reasonix",
        &["acp"],
        &["acp"],
        &get_env,
    );
    reasonix_deepseek.remove_env_prefixes = vec!["ANTHROPIC_", "CLAUDE_", "DEEPSEEK_", "OPENAI_"];
    reasonix_deepseek.remove_env = vec!["CODEX_API_KEY", "CODEX_ACCESS_TOKEN", "CODEX_AUTH"];
    m.insert("reasonix-deepseek", reasonix_deepseek);
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
            remove_env_prefixes: Vec::new(),
        },
        // Default command (npx): `_ARGS` may still override the pinned adapter args.
        None => LaunchSpec {
            id,
            command: default_cmd.to_owned(),
            args: arg_override
                .unwrap_or_else(|| default_args.iter().map(|s| (*s).to_owned()).collect()),
            env: HashMap::new(),
            remove_env: Vec::new(),
            remove_env_prefixes: Vec::new(),
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
    if id == "claude-deepseek" {
        if !crate::claude_shell::available() {
            tracing::warn!("Claude DeepSeek requires an executable absolute bash or zsh path");
            return None;
        }
        match prepare_claude_deepseek_config_dir() {
            Ok(config_dir) => {
                spec.env.insert(
                    "CLAUDE_CONFIG_DIR".to_owned(),
                    config_dir.display().to_string(),
                );
            }
            Err(error) => {
                tracing::warn!(%error, "failed to prepare isolated Claude DeepSeek config");
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

/// Whether a Cowboy provider uses Claude Code ACP/runtime semantics.
#[must_use]
pub fn is_claude(id: &str) -> bool {
    matches!(id, "claude-code" | "claude-deepseek")
}

/// Entries a DeepSeek provider shares with the runtime's ordinary home.
///
/// A DeepSeek provider differs from its ordinary counterpart in exactly two
/// ways: which endpoint it talks to, and whose credential it presents. Anything
/// else the user set up should behave the same, so machine-wide guidance,
/// skills, and installed plugins are shared rather than re-created.
///
/// This is an allowlist on purpose. A denylist would silently start leaking the
/// day the runtime adds a new file that holds a secret.
const CODEX_SHARED_ENTRIES: &[&str] = &["AGENTS.md", "skills", "plugins"];
const CLAUDE_SHARED_ENTRIES: &[&str] = &["CLAUDE.md", "skills", "plugins"];

/// Codex resolves a configured marketplace against a snapshot under this path.
/// Only the snapshot directory is shared: the sibling lock and sync files stay
/// per-home so two Codex processes never contend over one lock.
const CODEX_SHARED_TMP_ENTRIES: &[&str] = &["marketplaces"];

/// `config.toml` tables that carry setup rather than secrets.
///
/// `mcp_servers` is deliberately absent: an MCP entry can hold a token in its
/// command, arguments, or headers.
const CODEX_SHARED_CONFIG_TABLES: &[&str] = &["marketplaces", "plugins", "hooks"];

/// `settings.json` keys that decide which plugins Claude Code loads.
///
/// The rest of that file stays private: it also carries model selection,
/// permissions, and MCP entries that the provider must own or must not see.
const CLAUDE_SHARED_SETTINGS_KEYS: &[&str] = &["enabledPlugins", "extraKnownMarketplaces"];

/// Link one shared entry into an isolated home, leaving real files untouched.
///
/// A missing source is not an error: the ordinary home may simply not have that
/// entry yet. An existing real file at the destination is left alone, because
/// provider-owned state must never be replaced by shared state.
///
/// A directory the runtime pre-created is the common case rather than the
/// exception: Codex writes `skills/.system` into every home it opens, so
/// refusing the whole directory would mean the user's skills never arrive.
/// Share its entries instead, which keeps the provider's own scaffolding.
fn link_shared_entry(isolated: &Path, ordinary: &Path, name: &str) -> std::io::Result<()> {
    let source = ordinary.join(name);
    if !source.exists() {
        return Ok(());
    }
    let destination = isolated.join(name);
    match std::fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() => std::fs::remove_file(&destination)?,
        Ok(metadata) if metadata.is_dir() && source.is_dir() => {
            return link_shared_children(&destination, &source);
        }
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::os::unix::fs::symlink(&source, &destination)
}

/// Link each child of a shared directory, never shadowing provider-owned state.
fn link_shared_children(destination: &Path, source: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let child = destination.join(entry.file_name());
        match std::fs::symlink_metadata(&child) {
            Ok(metadata) if metadata.file_type().is_symlink() => std::fs::remove_file(&child)?,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::os::unix::fs::symlink(entry.path(), &child)?;
    }
    Ok(())
}

/// Copy the allowlisted `config.toml` tables from the ordinary Codex home.
///
/// Codex needs both the marketplace/plugin tables and the shared snapshot
/// directory before it reports a plugin as installed; neither alone is enough.
fn shared_codex_config_tables(ordinary_home: &Path) -> String {
    let Ok(existing) = std::fs::read_to_string(ordinary_home.join("config.toml")) else {
        return String::new();
    };
    let mut copied = String::new();
    let mut keeping = false;
    for line in existing.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            let header = trimmed.trim_start_matches('[');
            keeping = CODEX_SHARED_CONFIG_TABLES.iter().any(|table| {
                header
                    .strip_prefix(table)
                    .is_some_and(|rest| rest.starts_with(['.', ']']))
            });
        }
        if keeping {
            copied.push_str(line);
            copied.push('\n');
        }
    }
    copied
}

/// Write the provider-owned `settings.json` that enables the shared plugins.
///
/// Claude Code keeps plugin enablement in `settings.json`, so linking
/// `plugins/` alone leaves every plugin installed but unloaded. Only the
/// enablement keys are copied; the file is regenerated on each launch so the
/// two runtimes cannot drift apart.
fn write_claude_shared_settings(isolated: &Path, ordinary: &Path) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let Ok(existing) = std::fs::read_to_string(ordinary.join("settings.json")) else {
        return Ok(());
    };
    let Ok(serde_json::Value::Object(settings)) =
        serde_json::from_str::<serde_json::Value>(&existing)
    else {
        return Ok(());
    };
    let shared: serde_json::Map<_, _> = CLAUDE_SHARED_SETTINGS_KEYS
        .iter()
        .filter_map(|key| {
            settings
                .get(*key)
                .map(|value| ((*key).to_owned(), value.clone()))
        })
        .collect();
    if shared.is_empty() {
        return Ok(());
    }
    let rendered = serde_json::to_string_pretty(&serde_json::Value::Object(shared))
        .map_err(std::io::Error::other)?;
    let temporary = isolated.join(format!(".settings.json.{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(rendered.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(&temporary, isolated.join("settings.json"))
}

fn prepare_claude_deepseek_config_dir() -> std::io::Result<PathBuf> {
    let user_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/draven"));
    prepare_claude_deepseek_config_dir_at(&user_home)
}

fn prepare_claude_deepseek_config_dir_at(user_home: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    let mut target = user_home.to_path_buf();
    for component in [
        ".local",
        "state",
        "cowboy",
        "providers",
        "claude-deepseek",
        "claude-config",
    ] {
        target.push(component);
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Claude DeepSeek config boundary must contain only real directories",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::create_dir(&target) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let metadata = std::fs::symlink_metadata(&target)?;
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "Claude DeepSeek config boundary must contain only real directories",
                            ));
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))?;

    let ordinary = user_home.join(".claude");
    for entry in CLAUDE_SHARED_ENTRIES {
        link_shared_entry(&target, &ordinary, entry)?;
    }
    write_claude_shared_settings(&target, &ordinary)?;
    Ok(target)
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

    let ordinary = user_home.join(".codex");
    for entry in CODEX_SHARED_ENTRIES {
        link_shared_entry(&target, &ordinary, entry)?;
    }
    let shared_tmp = target.join(".tmp");
    std::fs::create_dir_all(&shared_tmp)?;
    for entry in CODEX_SHARED_TMP_ENTRIES {
        link_shared_entry(&shared_tmp, &ordinary.join(".tmp"), entry)?;
    }

    let catalog =
        available_codex_deepseek_catalog().unwrap_or_else(|| PathBuf::from(CODEX_DEEPSEEK_CATALOG));
    let shared_tables = shared_codex_config_tables(&ordinary);
    let config = if shared_tables.is_empty() {
        render_codex_deepseek_config(&catalog)
    } else {
        format!(
            "{}\n{shared_tables}",
            render_codex_deepseek_config(&catalog)
        )
    };
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
     model_auto_compact_token_limit_scope = \"body_after_prefix\"\n\n\
     [model_providers.deepseek-local]\n\
     name = \"Isolated DeepSeek Responses gateway\"\n\
     base_url = \"http://127.0.0.1:61137/v1\"\n\
     wire_api = \"responses\"\n\
     requires_openai_auth = false\n\
     request_max_retries = 1\n\
     stream_max_retries = 0\n\
     stream_idle_timeout_ms = 600000\n\n\
     [features]\n\
     memories = true\n\n\
     [memories]\n\
     disable_on_external_context = true\n\
     extract_model = \"deepseek-v4-flash\"\n\
     consolidation_model = \"deepseek-v4-flash\"\n\
     min_rate_limit_remaining_percent = 0\n",
        catalog.display()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn lookup_with(overrides: &[(&str, &str)], id: &str) -> Option<super::LaunchSpec> {
        let overrides: HashMap<_, _> = overrides.iter().copied().collect();
        super::builtin_with_env_and_shell(
            |key| overrides.get(key).map(|value| (*value).to_owned()),
            Some("/test/bin/bash".to_owned()),
        )
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
                "-c",
                "model_auto_compact_token_limit_scope=\"body_after_prefix\"",
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

        let reasonix = lookup_with(
            &[(
                "COWBOY_ACP_REASONIX_DEEPSEEK_CMD",
                "/run/current-system/sw/bin/reasonix",
            )],
            "reasonix-deepseek",
        )
        .expect("reasonix-deepseek registered");
        assert_eq!(reasonix.command, "/run/current-system/sw/bin/reasonix");
        assert_eq!(reasonix.args, ["acp"]);
        for secret in ["DEEPSEEK_API_KEY", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"] {
            assert!(reasonix.removes_inherited_env(secret));
        }

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
                "-c",
                "model_auto_compact_token_limit_scope=\"body_after_prefix\"",
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

        let claude_deepseek = lookup_with(
            &[(
                "COWBOY_ACP_CLAUDE_CODE_CMD",
                "/opt/npm-global/bin/claude-agent-acp",
            )],
            "claude-deepseek",
        )
        .expect("claude-deepseek registered");
        assert_eq!(
            claude_deepseek.command,
            "/opt/npm-global/bin/claude-agent-acp"
        );
        assert!(claude_deepseek.args.is_empty());
        assert_eq!(
            claude_deepseek
                .env
                .get("ANTHROPIC_BASE_URL")
                .map(String::as_str),
            Some("http://127.0.0.1:61138")
        );
        assert_eq!(
            claude_deepseek
                .env
                .get("ANTHROPIC_MODEL")
                .map(String::as_str),
            Some("deepseek-v4-flash[1m]")
        );
        assert_eq!(
            claude_deepseek
                .env
                .get("CLAUDE_CODE_SUBAGENT_MODEL")
                .map(String::as_str),
            Some("deepseek-v4-flash")
        );
        assert!(!claude_deepseek.env.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
        assert_eq!(
            claude_deepseek
                .env
                .get("ANTHROPIC_AUTH_TOKEN")
                .map(String::as_str),
            Some("cowboy-local-credential-boundary")
        );
        assert_eq!(
            claude_deepseek
                .env
                .get("CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            claude_deepseek
                .env
                .get("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST")
                .map(String::as_str),
            Some("cowboy-claude-deepseek")
        );
        let shell = claude_deepseek
            .env
            .get("CLAUDE_CODE_SHELL")
            .expect("Claude Code shell detected");
        assert_eq!(shell, "/test/bin/bash");
        let shell = std::path::Path::new(shell);
        assert!(shell.is_absolute());
        assert!(matches!(
            shell.file_name().and_then(std::ffi::OsStr::to_str),
            Some("bash" | "zsh")
        ));
        assert_eq!(
            claude_deepseek.env.get("SHELL").map(String::as_str),
            shell.to_str()
        );
        assert!(claude_deepseek.removes_inherited_env("COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL"));
        assert!(
            !claude_deepseek
                .env
                .contains_key("ANTHROPIC_SMALL_FAST_MODEL")
        );
        for inherited in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_SHELL",
            "CLAUDE_CODE_USE_BEDROCK",
            "DEEPSEEK_API_KEY",
            "DISABLE_PROMPT_CACHING",
            "MAX_THINKING_TOKENS",
        ] {
            assert!(
                claude_deepseek.removes_inherited_env(inherited),
                "inherited {inherited} crossed the provider boundary"
            );
        }
        for preserved in ["HOME", "PATH", "SSH_AUTH_SOCK", "HTTP_PROXY"] {
            assert!(!claude_deepseek.removes_inherited_env(preserved));
        }

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
        assert!(rendered.contains("model_auto_compact_token_limit_scope = \"body_after_prefix\""));
        assert!(rendered.contains("[model_providers.deepseek-local]"));
        assert!(rendered.contains("requires_openai_auth = false"));
        assert!(rendered.contains("[features]\nmemories = true"));
        assert!(rendered.contains("[memories]\ndisable_on_external_context = true"));
        assert!(rendered.contains("extract_model = \"deepseek-v4-flash\""));
        assert!(rendered.contains("consolidation_model = \"deepseek-v4-flash\""));
        assert!(rendered.contains("min_rate_limit_remaining_percent = 0"));
        assert!(rendered.contains("/nix/var/nix/profiles/columbus-components/codex-deepseek/"));
        assert!(!rendered.contains("api.openai.com"));
    }

    fn isolation_test_root(prefix: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_TEST_HOME: AtomicU64 = AtomicU64::new(1);
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT_TEST_HOME.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn deepseek_home_never_reads_or_links_openai_codex_state() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = isolation_test_root("cowboy-codex-deepseek-isolation");
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
        // Credential-bearing and conversation state is never copied or linked.
        for leaked in ["auth.json", "history.jsonl", "sessions", "memories"] {
            assert!(
                !isolated.join(leaked).exists(),
                "isolated Codex home must not expose {leaked}"
            );
        }

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[test]
    fn codex_deepseek_home_shares_guidance_skills_and_plugins() {
        let root = isolation_test_root("cowboy-codex-deepseek-sharing");
        let openai_home = root.join(".codex");
        std::fs::create_dir_all(openai_home.join("skills/omega")).expect("create skills");
        std::fs::create_dir_all(openai_home.join("plugins/cache")).expect("create plugins");
        std::fs::create_dir_all(openai_home.join(".tmp/marketplaces/columbus"))
            .expect("create marketplace snapshot");
        std::fs::write(openai_home.join("AGENTS.md"), "machine guidance").expect("write guidance");
        std::fs::write(
            openai_home.join("config.toml"),
            "model = \"gpt-secret-sentinel\"\n\
             [mcp_servers.private]\ncommand = \"secret\"\n\
             [marketplaces.columbus]\nsource = \"git@example.invalid:c.git\"\n\
             [plugins.\"columbus-harness@columbus\"]\nenabled = true\n\
             [hooks.state]\ntrusted = \"sha256:abc\"\n",
        )
        .expect("write OpenAI config");

        let isolated = super::prepare_codex_deepseek_home_at(&root).expect("prepare DeepSeek home");

        assert_eq!(
            std::fs::read_to_string(isolated.join("AGENTS.md")).unwrap(),
            "machine guidance"
        );
        assert!(isolated.join("skills/omega").is_dir());
        assert!(isolated.join("plugins/cache").is_dir());
        assert!(isolated.join(".tmp/marketplaces/columbus").is_dir());

        let config = std::fs::read_to_string(isolated.join("config.toml")).expect("read config");
        assert!(config.contains("[marketplaces.columbus]"));
        assert!(config.contains("[plugins.\"columbus-harness@columbus\"]"));
        assert!(config.contains("[hooks.state]"));
        // The provider still owns model selection, and MCP entries can hold tokens.
        assert!(config.contains("model = \"deepseek-v4-flash\""));
        assert!(!config.contains("gpt-secret-sentinel"));
        assert!(!config.contains("mcp_servers.private"));

        // The lock and sync files beside the snapshot stay per-home.
        assert!(!isolated.join(".tmp/plugins.sync.lock").exists());

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[test]
    fn codex_deepseek_home_preserves_provider_owned_entries() {
        let root = isolation_test_root("cowboy-codex-deepseek-preserve");
        let openai_home = root.join(".codex");
        std::fs::create_dir_all(openai_home.join("skills")).expect("create ordinary skills");
        let isolated_home =
            root.join(".local/state/cowboy/providers/codex-deepseek/codex-home/skills");
        std::fs::create_dir_all(&isolated_home).expect("create provider-owned skills");
        std::fs::write(isolated_home.join("owned.md"), "provider owned").expect("write owned");

        let isolated = super::prepare_codex_deepseek_home_at(&root).expect("prepare DeepSeek home");

        // A real provider-owned entry is never replaced by a shared link.
        assert!(isolated.join("skills/owned.md").is_file());
        assert!(
            !std::fs::symlink_metadata(isolated.join("skills"))
                .unwrap()
                .file_type()
                .is_symlink()
        );

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[test]
    fn codex_deepseek_home_shares_skills_beside_the_runtime_scaffolding() {
        // Codex writes skills/.system into every home it opens, so the shared
        // directory is always pre-created; refusing it outright shared nothing.
        let root = isolation_test_root("cowboy-codex-deepseek-skills");
        let openai_home = root.join(".codex");
        std::fs::create_dir_all(openai_home.join("skills/omega")).expect("create shared skill");
        std::fs::create_dir_all(openai_home.join("skills/.system"))
            .expect("create ordinary system");
        let scaffolding =
            root.join(".local/state/cowboy/providers/codex-deepseek/codex-home/skills/.system");
        std::fs::create_dir_all(&scaffolding).expect("create provider scaffolding");
        std::fs::write(scaffolding.join("marker"), "provider owned").expect("write marker");

        let isolated = super::prepare_codex_deepseek_home_at(&root).expect("prepare DeepSeek home");

        // The user's skill arrives even though the directory already existed.
        assert!(isolated.join("skills/omega").is_dir());
        assert!(
            std::fs::symlink_metadata(isolated.join("skills/omega"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // The runtime's own scaffolding is kept, not shadowed by the shared one.
        assert_eq!(
            std::fs::read_to_string(isolated.join("skills/.system/marker")).unwrap(),
            "provider owned"
        );
        assert!(
            !std::fs::symlink_metadata(isolated.join("skills/.system"))
                .unwrap()
                .file_type()
                .is_symlink()
        );

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[test]
    fn claude_deepseek_config_never_reads_or_links_standard_claude_state() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = isolation_test_root("cowboy-claude-deepseek-isolation");
        let standard = root.join(".claude");
        std::fs::create_dir_all(&standard).expect("create standard Claude config");
        std::fs::write(
            standard.join("settings.json"),
            r#"{"model":"claude-secret-sentinel","mcpServers":{"private":{}}}"#,
        )
        .expect("write standard settings");
        std::fs::write(standard.join(".credentials.json"), "claude-auth-sentinel")
            .expect("write standard auth");
        std::fs::write(root.join(".claude.json"), "claude-instance-sentinel")
            .expect("write standard instance metadata");

        let isolated = super::prepare_claude_deepseek_config_dir_at(&root)
            .expect("prepare isolated Claude config");
        assert_eq!(
            std::fs::metadata(&isolated).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // Settings and credentials are never linked; only the allowlist is.
        for leaked in ["settings.json", ".credentials.json", "projects", "history"] {
            assert!(
                !isolated.join(leaked).exists(),
                "isolated Claude config must not expose {leaked}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(standard.join("settings.json")).unwrap(),
            r#"{"model":"claude-secret-sentinel","mcpServers":{"private":{}}}"#
        );
        assert_eq!(
            std::fs::read_to_string(standard.join(".credentials.json")).unwrap(),
            "claude-auth-sentinel"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(".claude.json")).unwrap(),
            "claude-instance-sentinel"
        );

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[test]
    fn claude_deepseek_config_shares_guidance_skills_and_plugins() {
        let root = isolation_test_root("cowboy-claude-deepseek-sharing");
        let standard = root.join(".claude");
        std::fs::create_dir_all(standard.join("skills/omega")).expect("create skills");
        std::fs::create_dir_all(standard.join("plugins/cache")).expect("create plugins");
        std::fs::write(standard.join("CLAUDE.md"), "machine guidance").expect("write guidance");
        std::fs::write(standard.join(".credentials.json"), "claude-auth-sentinel")
            .expect("write standard auth");
        std::fs::write(
            standard.join("settings.json"),
            r#"{"model":"claude-secret-sentinel",
                "mcpServers":{"private":{"command":"secret"}},
                "permissions":{"allow":["WebSearch"]},
                "enabledPlugins":{"columbus-harness@columbus":true},
                "extraKnownMarketplaces":{"columbus":{"source":{"repo":"c"}}}}"#,
        )
        .expect("write standard settings");

        let isolated = super::prepare_claude_deepseek_config_dir_at(&root)
            .expect("prepare isolated Claude config");

        assert_eq!(
            std::fs::read_to_string(isolated.join("CLAUDE.md")).unwrap(),
            "machine guidance"
        );
        assert!(isolated.join("skills/omega").is_dir());
        assert!(isolated.join("plugins/cache").is_dir());
        assert!(!isolated.join(".credentials.json").exists());

        // Claude keeps plugin enablement in settings.json, so linking plugins/
        // alone leaves every plugin installed but unloaded.
        let settings =
            std::fs::read_to_string(isolated.join("settings.json")).expect("read shared settings");
        assert!(settings.contains("columbus-harness@columbus"));
        assert!(settings.contains("extraKnownMarketplaces"));
        assert!(!settings.contains("claude-secret-sentinel"));
        assert!(!settings.contains("mcpServers"));
        assert!(!settings.contains("permissions"));

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[test]
    fn claude_deepseek_settings_are_absent_without_shared_keys() {
        let root = isolation_test_root("cowboy-claude-deepseek-no-settings");
        let standard = root.join(".claude");
        std::fs::create_dir_all(&standard).expect("create standard Claude config");
        std::fs::write(standard.join("settings.json"), r#"{"theme":"dark"}"#)
            .expect("write standard settings");

        let isolated = super::prepare_claude_deepseek_config_dir_at(&root)
            .expect("prepare isolated Claude config");

        // Nothing to share means no provider-owned file is invented.
        assert!(!isolated.join("settings.json").exists());

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[cfg(unix)]
    #[test]
    fn claude_deepseek_config_rejects_symlink_boundaries_at_every_depth() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_TEST_HOME: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "cowboy-claude-deepseek-symlink-{}-{}",
            std::process::id(),
            NEXT_TEST_HOME.fetch_add(1, Ordering::Relaxed)
        ));
        for relative in [
            ".local/state/cowboy/providers/claude-deepseek",
            ".local/state/cowboy/providers/claude-deepseek/claude-config",
        ] {
            let case = root.join(relative.replace('/', "-"));
            let boundary = case.join(relative);
            std::fs::create_dir_all(boundary.parent().unwrap()).unwrap();
            let outside = case.join("ordinary-claude-state");
            std::fs::create_dir_all(&outside).unwrap();
            symlink(&outside, &boundary).unwrap();

            let error = super::prepare_claude_deepseek_config_dir_at(&case).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        }

        std::fs::remove_dir_all(&root).expect("remove symlink test home");
    }

    #[test]
    fn claude_deepseek_config_creation_is_safe_under_concurrent_first_launches() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let root = std::env::temp_dir().join(format!(
            "cowboy-claude-deepseek-concurrent-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).expect("create concurrent test home");

        let barrier = Arc::new(Barrier::new(16));
        let workers = (0..16)
            .map(|_| {
                let root = root.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    super::prepare_claude_deepseek_config_dir_at(&root)
                })
            })
            .collect::<Vec<_>>();

        let mut expected = None;
        for worker in workers {
            let path = worker
                .join()
                .expect("config creation thread panicked")
                .expect("concurrent config creation failed");
            assert_eq!(expected.get_or_insert_with(|| path.clone()), &path);
        }
        std::fs::remove_dir_all(&root).expect("remove concurrent test home");
    }
}

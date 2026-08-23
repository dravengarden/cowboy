//! Provider process launch boundary.
//!
//! Installed sessions resolve their exact package and generation-local command
//! from Machine-owned state. The in-tree table is a bounded compatibility path
//! for package-less local or pre-schema sessions; it is not Provider discovery,
//! publication, or installation authority. A conforming signed Provider can be
//! added without adding an ID branch here.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use cowboy_provider_sdk::{
    AuthComponent, PlatformPayload, PrivateComponentKind, RuntimeBinding, RuntimeContract,
    RuntimeSidecar, RuntimeSidecarTransport, RuntimeValue,
};

use crate::provider_catalog::{CODEX_DEEPSEEK_CATALOG, available_codex_deepseek_catalog};

pub(crate) use crate::provider_behavior::legacy_behavior;

pub(crate) const DEEPSEEK_SESSION_ID_ENV: &str = "COWBOY_DEEPSEEK_SESSION_ID";

/// How to spawn one provider's ACP adapter as a subprocess.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// Stable provider id, e.g. `"claude-code"`.
    pub id: String,
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
    /// Owned isolation rules supplied by a signed Provider package.
    pub package_remove_env: std::collections::BTreeSet<String>,
    pub package_remove_env_prefixes: std::collections::BTreeSet<String>,
}

/// Prepared exact Provider process tree. Session sidecars use `kill_on_drop`
/// and are held here for the complete worker lifetime.
pub(crate) struct PreparedLaunch {
    pub spec: LaunchSpec,
    pub sidecars: Vec<tokio::process::Child>,
}

impl LaunchSpec {
    #[must_use]
    pub fn removes_inherited_env(&self, key: &str) -> bool {
        self.remove_env.contains(&key)
            || self.package_remove_env.contains(key)
            || self
                .remove_env_prefixes
                .iter()
                .any(|prefix| key.starts_with(prefix))
            || self
                .package_remove_env_prefixes
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

// Grok Build is itself an ACP agent. Keep every Cowboy session in its own
// process instead of joining the CLI's optional shared leader, leave component
// updates to Cowboy Machine, and match Cowboy's unrestricted agent posture.
// DeepSeek's Anthropic-compatible 1M lane counts the requested completion
// against the same context budget as the prompt. Claude Code otherwise waits
// until roughly the end of the advertised window before compacting, after
// DeepSeek has already rejected the request. The default user-visible 830K
// budget therefore compacts at the explicitly safer 819.2K boundary.
const CLAUDE_DEEPSEEK_AUTO_COMPACT_WINDOW: &str = "819200";
const CLAUDE_DEEPSEEK_MAX_OUTPUT_TOKENS: &str = "128000";
const CODEX_DEEPSEEK_CONTEXT_WINDOW: &str = "680000";
const CODEX_DEEPSEEK_AUTO_COMPACT_TOKEN_LIMIT: &str = "646000";

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

/// Legacy package-less launch recipes. Claude Code and Codex remain first for
/// deterministic compatibility-test ordering.
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
/// - `grok`: the official Grok Build CLI's own ACP stdio agent —
///   `@xai-official/grok agent stdio`. Requires a prior `grok login` or
///   `XAI_API_KEY`.
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
    let session_context_window = get_env(crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV)
        .and_then(|value| value.parse::<u64>().ok());
    let session_auto_compact_token_limit =
        get_env(crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV)
            .and_then(|value| value.parse::<u64>().ok());
    let session_budget_values = session_context_window
        .zip(session_auto_compact_token_limit)
        .filter(|(window, compact)| *window > 0 && *compact > 0 && *compact <= *window);
    let claude_session_budget = session_budget_values.and_then(|(window, compact)| {
        crate::deepseek_context::from_launch_values(
            &cowboy_provider_sdk::ConfigurationBehavior::AnthropicGatewayV1,
            window,
            compact,
        )
    });
    let codex_session_budget = session_budget_values.and_then(|(window, compact)| {
        crate::deepseek_context::from_launch_values(
            &cowboy_provider_sdk::ConfigurationBehavior::OpenaiGatewayV1,
            window,
            compact,
        )
    });
    let claude_executable =
        get_env("COWBOY_ACP_CLAUDE_CODE_EXECUTABLE").filter(|value| !value.trim().is_empty());
    let mut claude = spec(
        "claude-code",
        "npx",
        &["-y", "@agentclientprotocol/claude-agent-acp"],
        &get_env,
    );
    if let Some(executable) = &claude_executable {
        claude
            .env
            .insert("CLAUDE_CODE_EXECUTABLE".to_owned(), executable.clone());
    }
    m.insert("claude-code", claude);
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
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW".to_owned(),
            claude_session_budget.map_or_else(
                || CLAUDE_DEEPSEEK_AUTO_COMPACT_WINDOW.to_owned(),
                |budget| budget.auto_compact_token_limit.to_string(),
            ),
        ),
        (
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS".to_owned(),
            CLAUDE_DEEPSEEK_MAX_OUTPUT_TOKENS.to_owned(),
        ),
        // DeepSeek's strongest reasoning posture is the default for this
        // isolated lane. The ACP effort picker remains available, so users
        // can still choose `default` or `high` for a particular session.
        ("CLAUDE_CODE_EFFORT_LEVEL".to_owned(), "max".to_owned()),
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
    // Keep Claude Code's native non-streaming fallback enabled. DeepSeek can
    // occasionally return HTTP 200 and then close SSE before a content block;
    // the CLI's fallback repairs that empty attempt within the same native turn.
    if let Some(shell) = claude_deepseek_shell {
        // `CLAUDE_CODE_SHELL` is the authoritative override. Also set `SHELL`
        // for subprocesses and older Claude Code releases that consult it.
        claude_deepseek
            .env
            .insert("CLAUDE_CODE_SHELL".to_owned(), shell.clone());
        claude_deepseek.env.insert("SHELL".to_owned(), shell);
    }
    if let Some(executable) = claude_executable {
        // Applied after the inherited CLAUDE_* scrub, so the isolated provider
        // uses the host-selected CLI without inheriting ordinary Claude state.
        claude_deepseek
            .env
            .insert("CLAUDE_CODE_EXECUTABLE".to_owned(), executable);
    }
    claude_deepseek.remove_env_prefixes = vec!["ANTHROPIC_", "CLAUDE_", "DEEPSEEK_"];
    claude_deepseek.remove_env = vec![
        "API_TIMEOUT_MS",
        "COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL",
        crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV,
        crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
        "DISABLE_AUTO_COMPACT",
        "DISABLE_COMPACT",
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
    if let Some(budget) = codex_session_budget {
        deepseek.args.extend([
            "-c".to_owned(),
            format!("model_context_window={}", budget.context_window),
            "-c".to_owned(),
            format!(
                "model_auto_compact_token_limit={}",
                budget.auto_compact_token_limit
            ),
        ]);
    }
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
        crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV,
        crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
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
    m.insert(
        "grok",
        spec_with_custom_default_args(
            "grok",
            "npx",
            &concat_slices(&["-y", "@xai-official/grok"], crate::grok::RUNTIME_ARGS),
            crate::grok::RUNTIME_ARGS,
            &get_env,
        ),
    );
    m
}

/// Build a provider's launch spec, letting the deployment OVERRIDE how the ACP
/// adapter is launched via env — `COWBOY_ACP_<ID>_CMD` (+ optional
/// shell-quoted `COWBOY_ACP_<ID>_ARGS`), where `<ID>` is the upper-cased id
/// with `-`→`_` (e.g. `COWBOY_ACP_CLAUDE_CODE_CMD`).
///
/// Why: the default `npx -y <pkg>` cold-installs the adapter into the shared
/// `~/.npm/_npx` cache on EVERY session start. Concurrent starts race npm's
/// atomic rename (ENOTEMPTY → the adapter exits 217 → the session crashes), an
/// interrupted install leaves stale staging dirs that poison every later start,
/// and each start pays a registry round-trip. Pointing this at a PRE-INSTALLED
/// adapter binary supplied by the host removes `npx` from the hot path
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
    let arg_override = get_env(&format!("COWBOY_ACP_{key}_ARGS")).map(|args| {
        shell_words::split(&args)
            .unwrap_or_else(|_| args.split_whitespace().map(str::to_owned).collect())
    });
    match get_env(&format!("COWBOY_ACP_{key}_CMD")) {
        // A custom command replaces npx: the npx-specific prefix (`-y <pkg>`)
        // does NOT carry over. Provider-specific args may still apply, e.g.
        // Codex's default full-access config for a pre-installed adapter.
        Some(command) => LaunchSpec {
            id: id.to_owned(),
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
            package_remove_env: std::collections::BTreeSet::new(),
            package_remove_env_prefixes: std::collections::BTreeSet::new(),
        },
        // Default command (npx): `_ARGS` may still override the pinned adapter args.
        None => LaunchSpec {
            id: id.to_owned(),
            command: default_cmd.to_owned(),
            args: arg_override
                .unwrap_or_else(|| default_args.iter().map(|s| (*s).to_owned()).collect()),
            env: HashMap::new(),
            remove_env: Vec::new(),
            remove_env_prefixes: Vec::new(),
            package_remove_env: std::collections::BTreeSet::new(),
            package_remove_env_prefixes: std::collections::BTreeSet::new(),
        },
    }
}

/// Resolve the exact installed-package launch, or the legacy local fallback
/// when no package path is present in this worker.
#[must_use]
pub fn lookup(id: &str) -> Option<LaunchSpec> {
    if std::env::var_os("COWBOY_PROVIDER_PACKAGE_PATH").is_some() {
        // Exact packages may require async sidecar readiness. The detached
        // worker must use `prepare`; never degrade a corrupt package into the
        // legacy table or start it without its signed process tree.
        return None;
    }
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

/// Resolve and prepare the complete process tree for one detached worker.
/// Package-less sessions retain the bounded legacy launch path; exact signed
/// packages additionally get linked components and session-owned sidecars.
pub(crate) async fn prepare(id: &str) -> Result<PreparedLaunch> {
    if std::env::var_os("COWBOY_PROVIDER_PACKAGE_PATH").is_none() {
        let spec = lookup(id).with_context(|| format!("unknown provider {id:?}"))?;
        return Ok(PreparedLaunch {
            spec,
            sidecars: Vec::new(),
        });
    }
    prepare_package_launch(id).await
}

/// Controller-side placeholder for an immutable Provider generation that will
/// be resolved and launched by the selected Machine. No executable or secret
/// crosses this boundary.
#[must_use]
pub fn remote_generation(id: &str) -> Option<LaunchSpec> {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    valid.then(|| LaunchSpec {
        id: id.to_owned(),
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        remove_env: Vec::new(),
        remove_env_prefixes: Vec::new(),
        package_remove_env: std::collections::BTreeSet::new(),
        package_remove_env_prefixes: std::collections::BTreeSet::new(),
    })
}

/// Resolve a signed, Machine-validated Provider generation supplied to this
/// worker and start every declared sidecar before the ACP adapter. All links
/// are closed SDK values; no shell interpolation or Provider code is involved.
async fn prepare_package_launch(id: &str) -> Result<PreparedLaunch> {
    let package_path = PathBuf::from(
        std::env::var_os("COWBOY_PROVIDER_PACKAGE_PATH")
            .context("exact Provider worker has no package path")?,
    );
    let bytes = std::fs::read(&package_path)
        .with_context(|| format!("reading Provider package {}", package_path.display()))?;
    let package = cowboy_provider_sdk::ProviderPackage::from_bytes(&bytes)
        .context("validating exact Provider process package")?;
    ensure!(
        package.manifest.id == id,
        "Provider package identity mismatch: expected {id:?}, got {:?}",
        package.manifest.id
    );
    let os = match std::env::consts::OS {
        "linux" => cowboy_provider_sdk::OperatingSystem::Linux,
        "macos" => cowboy_provider_sdk::OperatingSystem::Macos,
        value => bail!("unsupported Provider worker operating system {value:?}"),
    };
    let architecture = match std::env::consts::ARCH {
        "x86_64" => cowboy_provider_sdk::Architecture::X86_64,
        "aarch64" => cowboy_provider_sdk::Architecture::Aarch64,
        value => bail!("unsupported Provider worker architecture {value:?}"),
    };
    let payload = package
        .manifest
        .runtime
        .platforms
        .iter()
        .find(|payload| payload.os == os && payload.architecture == architecture)
        .context("Provider package does not support this worker platform")?;
    let commands = verified_component_commands(&package_path, payload)?;
    let command = commands
        .get(&payload.launch_command)
        .context("Provider launch command is absent from the Machine binding")?
        .display()
        .to_string();
    if let Ok(machine_entrypoint) = std::env::var("COWBOY_PROVIDER_ENTRYPOINT") {
        ensure!(
            Path::new(&machine_entrypoint) == Path::new(&command),
            "Machine Provider entrypoint disagrees with its component binding"
        );
    }

    let mut sidecars = Vec::new();
    let mut sidecar_urls = BTreeMap::new();
    for sidecar in &package.manifest.runtime.sidecars {
        let (child, url) = start_sidecar(sidecar, &package.manifest.runtime, payload, &commands)
            .await
            .with_context(|| format!("starting Provider sidecar {:?}", sidecar.id))?;
        sidecars.push(child);
        sidecar_urls.insert(sidecar.id.clone(), url);
    }

    let mut environment = HashMap::new();
    for (name, value) in &package.manifest.runtime.environment {
        environment.insert(
            name.clone(),
            resolve_runtime_value(value, payload, &commands, &sidecar_urls)?,
        );
    }
    let sidecar_auth: BTreeSet<_> = package
        .manifest
        .runtime
        .sidecars
        .iter()
        .flat_map(|sidecar| sidecar.auth_environment.iter().cloned())
        .collect();
    environment.extend(projected_auth_environment(
        &package.manifest.authentication.environment_projection,
        &sidecar_auth,
        |name| std::env::var(name).ok(),
    ));
    let arguments = package
        .manifest
        .runtime
        .arguments
        .iter()
        .map(|value| resolve_runtime_value(value, payload, &commands, &sidecar_urls))
        .collect::<Result<Vec<_>>>()?;
    Ok(PreparedLaunch {
        spec: LaunchSpec {
            id: package.manifest.id,
            command,
            args: arguments,
            env: environment,
            remove_env: Vec::new(),
            remove_env_prefixes: Vec::new(),
            package_remove_env: package.manifest.runtime.remove_environment,
            package_remove_env_prefixes: package.manifest.runtime.remove_environment_prefixes,
        },
        sidecars,
    })
}

fn projected_auth_environment(
    projection: &BTreeMap<String, String>,
    sidecar_auth: &BTreeSet<String>,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> HashMap<String, String> {
    projection
        .keys()
        .filter(|name| !sidecar_auth.contains(*name))
        .filter_map(|name| lookup(name).map(|value| (name.clone(), value)))
        .collect()
}

fn verified_component_commands(
    package_path: &Path,
    payload: &PlatformPayload,
) -> Result<BTreeMap<String, PathBuf>> {
    let raw = std::env::var(crate::provider_behavior::COMPONENT_COMMANDS_ENV)
        .context("Machine did not bind Provider component commands")?;
    let supplied: BTreeMap<String, String> =
        serde_json::from_str(&raw).context("decoding Machine Provider component commands")?;
    ensure!(
        supplied.len() == payload.private_components.len(),
        "Machine Provider component command set is incomplete"
    );
    let content = package_path
        .parent()
        .context("Provider package has no generation content directory")?
        .canonicalize()
        .context("resolving Provider generation content directory")?;
    let mut commands = BTreeMap::new();
    for component in &payload.private_components {
        let path = PathBuf::from(
            supplied
                .get(&component.command)
                .with_context(|| format!("Machine did not bind {:?}", component.command))?,
        );
        ensure!(
            path.is_absolute(),
            "Provider component command is not absolute"
        );
        let path = path
            .canonicalize()
            .with_context(|| format!("resolving Provider component {}", path.display()))?;
        ensure!(
            path.starts_with(&content) && path.is_file(),
            "Provider component command escaped its exact generation"
        );
        commands.insert(component.command.clone(), path);
    }
    ensure!(
        supplied
            .keys()
            .all(|command| commands.contains_key(command)),
        "Machine bound an undeclared Provider component command"
    );
    Ok(commands)
}

fn component_command<'a>(
    component: &AuthComponent,
    payload: &PlatformPayload,
    commands: &'a BTreeMap<String, PathBuf>,
) -> Result<&'a Path> {
    let command = payload
        .private_components
        .iter()
        .find(|candidate| candidate.kind == component.kind && candidate.slot == component.slot)
        .context("Provider runtime references an unavailable component")?
        .command
        .as_str();
    commands
        .get(command)
        .map(PathBuf::as_path)
        .context("Provider runtime component command is not bound")
}

fn resolve_runtime_value(
    value: &RuntimeValue,
    payload: &PlatformPayload,
    commands: &BTreeMap<String, PathBuf>,
    sidecars: &BTreeMap<String, String>,
) -> Result<String> {
    let (prefix, value, suffix) = match value {
        RuntimeValue::Literal(value) => return Ok(value.clone()),
        RuntimeValue::Binding(RuntimeBinding::ComponentCommand {
            component,
            prefix,
            suffix,
        }) => (
            prefix,
            component_command(component, payload, commands)?
                .display()
                .to_string(),
            suffix,
        ),
        RuntimeValue::Binding(RuntimeBinding::SidecarUrl {
            sidecar,
            prefix,
            suffix,
        }) => (
            prefix,
            sidecars
                .get(sidecar)
                .with_context(|| format!("Provider sidecar {sidecar:?} is not ready"))?
                .clone(),
            suffix,
        ),
    };
    Ok(format!("{prefix}{value}{suffix}"))
}

async fn start_sidecar(
    sidecar: &RuntimeSidecar,
    runtime: &RuntimeContract,
    payload: &PlatformPayload,
    commands: &BTreeMap<String, PathBuf>,
) -> Result<(tokio::process::Child, String)> {
    ensure!(
        sidecar.component.kind == PrivateComponentKind::ProviderGateway,
        "Provider sidecar component is not a gateway"
    );
    let executable = component_command(&sidecar.component, payload, commands)?;
    let RuntimeSidecarTransport::LoopbackHttpV1 {
        listen_argument,
        health_path,
        timeout_ms,
    } = &sidecar.transport;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .context("allocating Provider sidecar loopback port")?;
    let address = listener.local_addr()?;
    drop(listener);
    let base_url = format!("http://{address}");
    let health_url = format!("{base_url}{health_path}");

    let mut command = tokio::process::Command::new(executable);
    command
        .args(&sidecar.arguments)
        .arg(listen_argument)
        .arg(address.to_string())
        .current_dir(
            executable
                .parent()
                .context("Provider sidecar command has no parent directory")?,
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    for (name, _) in std::env::vars_os() {
        let Some(name) = name.to_str() else {
            continue;
        };
        if runtime.remove_environment.contains(name)
            || runtime
                .remove_environment_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
        {
            command.env_remove(name);
        }
    }
    command.envs(&sidecar.environment);
    for name in &sidecar.auth_environment {
        command.env(
            name,
            std::env::var(name)
                .with_context(|| format!("Provider sidecar auth projection {name:?} is missing"))?,
        );
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawning Provider sidecar {}", executable.display()))?;
    let deadline = Instant::now() + Duration::from_millis(*timeout_ms);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(250))
        .build()?;
    loop {
        if let Some(status) = child.try_wait().context("polling Provider sidecar")? {
            bail!("Provider sidecar exited before readiness: {status}");
        }
        if client
            .get(&health_url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok((child, base_url));
        }
        if Instant::now() >= deadline {
            let _ = child.kill().await;
            bail!("Provider sidecar readiness timed out");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Read the exact package selected by the Machine for this worker. The package
/// has already passed the Machine trust boundary; parsing it again keeps all
/// worker-side behavior dispatch bound to the signed generation rather than to
/// a user-visible Provider id.
fn process_package(id: &str) -> Option<cowboy_provider_sdk::ProviderPackage> {
    let path = std::env::var_os("COWBOY_PROVIDER_PACKAGE_PATH")?;
    let bytes = std::fs::read(path).ok()?;
    let package = cowboy_provider_sdk::ProviderPackage::from_bytes(&bytes).ok()?;
    if package.manifest.id != id {
        tracing::warn!(expected = id, actual = %package.manifest.id, "Provider package identity mismatch");
        return None;
    }
    Some(package)
}

/// Display identity from the exact signed process package. Package-less legacy
/// bridges deliberately fall back to the opaque id instead of a core name table.
#[must_use]
pub(crate) fn display_name(id: &str) -> String {
    process_package(id)
        .map(|package| package.manifest.display.name)
        .unwrap_or_else(|| id.to_owned())
}

/// Closed host behavior selected by the signed Provider package. Embedded
/// sources are consulted only for package-less sessions created before schema v1.
#[must_use]
pub fn behavior(id: &str) -> cowboy_provider_sdk::ProviderBehaviorContract {
    if std::env::var_os("COWBOY_PROVIDER_PACKAGE_PATH").is_some() {
        return process_package(id)
            .map(|package| package.manifest.runtime.behavior)
            .unwrap_or_else(|| legacy_behavior(""));
    }
    legacy_behavior(id)
}

/// Whether a Provider selected ACP config-option full-access mediation.
#[must_use]
pub fn uses_config_full_access(id: &str) -> bool {
    behavior(id).permission == cowboy_provider_sdk::PermissionBehavior::AcpConfigFullAccessV1
}

/// Whether a Provider selected ACP's bypass-permissions session mode.
#[must_use]
pub fn uses_bypass_permissions_session_mode(id: &str) -> bool {
    behavior(id).permission
        == cowboy_provider_sdk::PermissionBehavior::AcpSessionModeBypassPermissionsV1
}

/// Whether a Provider selected stable preset metadata for ACP sessions.
#[must_use]
pub fn uses_stable_preset_system_prompt(id: &str) -> bool {
    behavior(id).session == cowboy_provider_sdk::SessionBehavior::StablePresetSystemPromptV1
}

/// Whether a Provider selected the versioned xAI ACP extension interface.
#[must_use]
pub fn uses_xai_session_extensions(id: &str) -> bool {
    behavior(id).session == cowboy_provider_sdk::SessionBehavior::XaiSessionV1
}

/// Whether a Provider selected ACP's yolo session-mode interface.
#[must_use]
pub fn uses_yolo_session_mode(id: &str) -> bool {
    behavior(id).permission == cowboy_provider_sdk::PermissionBehavior::AcpSessionModeYoloV1
}

#[must_use]
pub fn keeps_worker_alive_for_behavior(
    behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
    detail: &str,
) -> bool {
    behavior
        .matching_error_rule(detail)
        .is_some_and(|rule| rule.keep_worker_alive)
}

#[must_use]
pub fn should_retry_without_visible_update(
    behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
    detail: &str,
    visible_update: bool,
    retries: usize,
) -> bool {
    !visible_update
        && retries == 0
        && behavior
            .matching_error_rule(detail)
            .is_some_and(|rule| rule.retry_once_without_visible_update)
}

#[must_use]
pub fn user_facing_startup_error(
    behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
    detail: &str,
) -> Option<String> {
    behavior
        .matching_error_rule(detail)
        .and_then(|rule| rule.user_detail.clone())
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

/// Write the provider-owned `settings.json` with its context safety invariant
/// and the shared plugin enablement keys.
///
/// Claude Code keeps plugin enablement in `settings.json`, so linking
/// `plugins/` alone leaves every plugin installed but unloaded. Auto-compaction
/// is provider-owned: disabling it lets DeepSeek reject a long thread before
/// Claude's default 1M threshold. The file is regenerated on each launch so
/// ordinary Claude settings cannot weaken the isolated lane.
fn write_claude_deepseek_settings(isolated: &Path, ordinary: &Path) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut provider_settings = serde_json::Map::new();
    provider_settings.insert(
        "autoCompactEnabled".to_owned(),
        serde_json::Value::Bool(true),
    );
    if let Ok(existing) = std::fs::read_to_string(ordinary.join("settings.json"))
        && let Ok(serde_json::Value::Object(settings)) =
            serde_json::from_str::<serde_json::Value>(&existing)
    {
        provider_settings.extend(CLAUDE_SHARED_SETTINGS_KEYS.iter().filter_map(|key| {
            settings
                .get(*key)
                .map(|value| ((*key).to_owned(), value.clone()))
        }));
    }
    let rendered = serde_json::to_string_pretty(&serde_json::Value::Object(provider_settings))
        .map_err(std::io::Error::other)?;
    static NEXT_SETTINGS_WRITE: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_SETTINGS_WRITE.fetch_add(1, Ordering::Relaxed);
    let temporary = isolated.join(format!(".settings.json.{}.{sequence}", std::process::id()));
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
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is not set"))?;
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
    write_claude_deepseek_settings(&target, &ordinary)?;
    Ok(target)
}

fn prepare_codex_deepseek_home() -> std::io::Result<PathBuf> {
    let user_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is not set"))?;
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
     model_reasoning_effort = \"max\"\n\
     model_catalog_json = \"{}\"\n\
     approval_policy = \"never\"\n\
     sandbox_mode = \"danger-full-access\"\n\n\
     model_context_window = {CODEX_DEEPSEEK_CONTEXT_WINDOW}\n\
     model_auto_compact_token_limit = {CODEX_DEEPSEEK_AUTO_COMPACT_TOKEN_LIMIT}\n\
     model_auto_compact_token_limit_scope = \"body_after_prefix\"\n\n\
     [model_providers.deepseek-local]\n\
     name = \"Isolated DeepSeek Responses gateway\"\n\
     base_url = \"http://127.0.0.1:61137/v1\"\n\
     wire_api = \"responses\"\n\
     requires_openai_auth = false\n\
     env_http_headers = {{ \"X-Cowboy-Session-Id\" = \"{DEEPSEEK_SESSION_ID_ENV}\", \"X-Cowboy-Cache-Protection\" = \"{cache_policy_env}\" }}\n\
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
        catalog.display(),
        cache_policy_env = crate::deepseek_cache::SESSION_POLICY_ENV,
    )
}

#[cfg(test)]
mod tests {
    use crate::deepseek_context;
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    fn lookup_with(overrides: &[(&str, &str)], id: &str) -> Option<super::LaunchSpec> {
        let overrides: HashMap<_, _> = overrides.iter().copied().collect();
        super::builtin_with_env_and_shell(
            |key| overrides.get(key).map(|value| (*value).to_owned()),
            Some("/test/bin/bash".to_owned()),
        )
        .remove(id)
    }

    #[test]
    fn auth_environment_accepts_alternative_file_only_credentials() {
        let projection = BTreeMap::from([("XAI_API_KEY".to_owned(), "api_key".to_owned())]);

        let file_only = super::projected_auth_environment(&projection, &BTreeSet::new(), |_| None);
        assert!(file_only.is_empty());

        let api_key = super::projected_auth_environment(&projection, &BTreeSet::new(), |name| {
            (name == "XAI_API_KEY").then(|| "projected-key".to_owned())
        });
        assert_eq!(
            api_key.get("XAI_API_KEY").map(String::as_str),
            Some("projected-key")
        );

        let sidecar_owned = super::projected_auth_environment(
            &projection,
            &BTreeSet::from(["XAI_API_KEY".to_owned()]),
            |_| Some("must-not-leak".to_owned()),
        );
        assert!(sidecar_owned.is_empty());
    }

    #[test]
    fn defaults_and_env_override() {
        // Default (no env): npx + the pinned adapter args; unknown id → None.
        let claude = lookup_with(&[], "claude-code").expect("claude-code registered");
        assert_eq!(claude.command, "npx");
        assert_eq!(claude.args, ["-y", "@agentclientprotocol/claude-agent-acp"]);
        let claude_deepseek =
            lookup_with(&[], "claude-deepseek").expect("claude-deepseek registered");
        assert_eq!(
            claude_deepseek
                .env
                .get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
                .map(String::as_str),
            Some("819200")
        );
        assert_eq!(
            claude_deepseek
                .env
                .get("CLAUDE_CODE_MAX_OUTPUT_TOKENS")
                .map(String::as_str),
            Some("128000")
        );
        assert!(claude_deepseek.remove_env.contains(&"DISABLE_AUTO_COMPACT"));
        assert!(claude_deepseek.remove_env.contains(&"DISABLE_COMPACT"));
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
        let grok = lookup_with(&[], "grok").expect("grok registered");
        assert_eq!(grok.command, "npx");
        assert_eq!(
            grok.args,
            [
                "-y",
                "@xai-official/grok",
                "--no-auto-update",
                "--experimental-memory",
                "--rules",
                crate::grok::PROJECT_RULES_BOOTSTRAP,
                "agent",
                "--always-approve",
                "--no-leader",
                "stdio",
            ]
        );
        let pinned_grok = lookup_with(
            &[
                ("COWBOY_ACP_GROK_CMD", "/opt/npm-global/bin/grok"),
                (
                    "COWBOY_ACP_GROK_ARGS",
                    "--no-auto-update --experimental-memory --rules 'Read and follow the closest AGENTS.md project instructions before taking any action.' agent --always-approve --no-leader stdio",
                ),
            ],
            "grok",
        )
        .expect("pinned grok command");
        assert_eq!(
            pinned_grok.args,
            [
                "--no-auto-update",
                "--experimental-memory",
                "--rules",
                crate::grok::PROJECT_RULES_BOOTSTRAP,
                "agent",
                "--always-approve",
                "--no-leader",
                "stdio",
            ]
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

        let claude_budget = lookup_with(
            &[
                (deepseek_context::SESSION_CONTEXT_WINDOW_ENV, "256000"),
                (
                    deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
                    "256000",
                ),
            ],
            "claude-deepseek",
        )
        .expect("claude-deepseek registered");
        assert_eq!(
            claude_budget
                .env
                .get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
                .map(String::as_str),
            Some("256000")
        );
        assert!(
            claude_budget
                .remove_env
                .contains(&deepseek_context::SESSION_CONTEXT_WINDOW_ENV)
        );

        let codex_budget = lookup_with(
            &[
                (deepseek_context::SESSION_CONTEXT_WINDOW_ENV, "830000"),
                (
                    deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
                    "788500",
                ),
            ],
            "codex-deepseek",
        )
        .expect("codex-deepseek registered");
        assert_eq!(
            codex_budget.args,
            [
                "-y",
                "@agentclientprotocol/codex-acp",
                "-c",
                "model_context_window=830000",
                "-c",
                "model_auto_compact_token_limit=788500",
            ]
        );
        assert!(
            codex_budget
                .remove_env
                .contains(&deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV)
        );

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
        let grok = lookup_with(
            &[("COWBOY_ACP_GROK_CMD", "/opt/npm-global/bin/grok")],
            "grok",
        )
        .expect("custom grok command");
        assert_eq!(grok.command, "/opt/npm-global/bin/grok");
        assert_eq!(
            grok.args,
            [
                "--no-auto-update",
                "--experimental-memory",
                "--rules",
                crate::grok::PROJECT_RULES_BOOTSTRAP,
                "agent",
                "--always-approve",
                "--no-leader",
                "stdio",
            ]
        );
        // Other custom commands still drop the npx-specific default args.
        let o = lookup_with(
            &[
                (
                    "COWBOY_ACP_CLAUDE_CODE_CMD",
                    "/opt/npm-global/bin/claude-agent-acp",
                ),
                (
                    "COWBOY_ACP_CLAUDE_CODE_EXECUTABLE",
                    "/opt/npm-global/bin/claude",
                ),
            ],
            "claude-code",
        )
        .unwrap();
        assert_eq!(o.command, "/opt/npm-global/bin/claude-agent-acp");
        assert!(
            o.args.is_empty(),
            "custom command drops the npx default args"
        );
        assert_eq!(
            o.env.get("CLAUDE_CODE_EXECUTABLE").map(String::as_str),
            Some("/opt/npm-global/bin/claude")
        );

        let claude_deepseek = lookup_with(
            &[
                (
                    "COWBOY_ACP_CLAUDE_CODE_CMD",
                    "/opt/npm-global/bin/claude-agent-acp",
                ),
                (
                    "COWBOY_ACP_CLAUDE_CODE_EXECUTABLE",
                    "/opt/npm-global/bin/claude",
                ),
            ],
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
                .get("CLAUDE_CODE_EXECUTABLE")
                .map(String::as_str),
            Some("/opt/npm-global/bin/claude")
        );
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
        assert_eq!(
            claude_deepseek
                .env
                .get("CLAUDE_CODE_EFFORT_LEVEL")
                .map(String::as_str),
            Some("max")
        );
        assert_eq!(
            claude_deepseek
                .env
                .get("ANTHROPIC_AUTH_TOKEN")
                .map(String::as_str),
            Some("cowboy-local-credential-boundary")
        );
        assert!(
            !claude_deepseek
                .env
                .contains_key("CLAUDE_CODE_DISABLE_NONSTREAMING_FALLBACK")
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
        assert!(rendered.contains("model_reasoning_effort = \"max\""));
        assert!(rendered.contains("approval_policy = \"never\""));
        assert!(rendered.contains("model_context_window = 680000"));
        assert!(rendered.contains("model_auto_compact_token_limit = 646000"));
        assert!(rendered.contains("model_auto_compact_token_limit_scope = \"body_after_prefix\""));
        assert!(rendered.contains("[model_providers.deepseek-local]"));
        assert!(rendered.contains("requires_openai_auth = false"));
        assert!(rendered.contains("\"X-Cowboy-Session-Id\" = \"COWBOY_DEEPSEEK_SESSION_ID\""));
        assert!(
            rendered
                .contains("\"X-Cowboy-Cache-Protection\" = \"COWBOY_DEEPSEEK_CACHE_PROTECTION\"")
        );
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
        // Credentials and mutable state are never linked; provider-owned
        // settings contain only the enforced safety key and allowlisted shared
        // entries.
        for leaked in [".credentials.json", "projects", "history"] {
            assert!(
                !isolated.join(leaked).exists(),
                "isolated Claude config must not expose {leaked}"
            );
        }
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(isolated.join("settings.json"))
                .expect("read provider settings"),
        )
        .expect("parse provider settings");
        assert_eq!(settings, serde_json::json!({"autoCompactEnabled": true}));
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
        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(isolated.join("settings.json")).expect("read shared settings"),
        )
        .expect("parse shared settings");
        assert_eq!(settings["autoCompactEnabled"], true);
        assert!(
            settings["enabledPlugins"]
                .get("columbus-harness@columbus")
                .is_some()
        );
        assert!(settings.get("extraKnownMarketplaces").is_some());
        assert!(settings.get("model").is_none());
        assert!(settings.get("mcpServers").is_none());
        assert!(settings.get("permissions").is_none());

        std::fs::remove_dir_all(&root).expect("remove isolated test home");
    }

    #[test]
    fn claude_deepseek_settings_keep_context_safety_without_shared_keys() {
        let root = isolation_test_root("cowboy-claude-deepseek-no-settings");
        let standard = root.join(".claude");
        std::fs::create_dir_all(&standard).expect("create standard Claude config");
        std::fs::write(standard.join("settings.json"), r#"{"theme":"dark"}"#)
            .expect("write standard settings");

        let isolated = super::prepare_claude_deepseek_config_dir_at(&root)
            .expect("prepare isolated Claude config");

        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(isolated.join("settings.json"))
                .expect("read provider settings"),
        )
        .expect("parse provider settings");
        assert_eq!(settings, serde_json::json!({"autoCompactEnabled": true}));

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

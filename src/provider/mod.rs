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

// Grok Build is itself an ACP agent. Keep every Cowboy session in its own
// process instead of joining the CLI's optional shared leader, leave component
// updates to Cowboy Machine, and match Cowboy's unrestricted agent posture.
// DeepSeek window and token budgets live in the plugin payloads: Claude Code's
// Anthropic-compatible 1M lane counts the requested completion against the
// same context budget as the prompt, so the default user-visible 830K budget
// compacts at the safer 819.2K boundary owned by claude-deepseek.

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
    builtin_with_env_and_shell(get_env, None)
}

fn builtin_with_env_and_shell(
    get_env: impl Fn(&str) -> Option<String>,
    isolated_shell: Option<String>,
) -> HashMap<&'static str, LaunchSpec> {
    let session_context_window = get_env(crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV)
        .and_then(|value| value.parse::<u64>().ok());
    let session_auto_compact_token_limit =
        get_env(crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV)
            .and_then(|value| value.parse::<u64>().ok());
    let session_budget_values = session_context_window
        .zip(session_auto_compact_token_limit)
        .filter(|(window, compact)| *window > 0 && *compact > 0 && *compact <= *window);
    let mut m = HashMap::new();
    for plugin_id in crate::plugin_runtime_args::launch_plugin_ids() {
        let extra = crate::plugin_runtime_args::cli_arguments(plugin_id);
        let npx = crate::plugin_runtime_args::npx_prefix(plugin_id);
        let mut default_args: Vec<&str> = npx.into_iter().collect();
        default_args.extend_from_slice(extra);
        let mut spec =
            spec_with_custom_default_args(plugin_id, "npx", &default_args, extra, &get_env);
        let configuration = crate::provider_behavior::legacy_behavior(plugin_id).configuration;
        if crate::plugin_runtime_args::isolated_home_env(plugin_id).is_some()
            && let Some((window, compact)) = session_budget_values
            && let Some(budget) =
                crate::deepseek_context::from_launch_values(&configuration, window, compact)
        {
            apply_openai_session_budget_args(&mut spec, &configuration, budget);
        }
        share_occupancy_command(plugin_id, &mut spec, &get_env);
        if crate::plugin_runtime_args::isolated_home_env(plugin_id).is_some() {
            apply_isolated_launch(plugin_id, &mut spec, &get_env, isolated_shell.as_deref());
            if let Some((window, compact)) = session_budget_values
                && let Some(budget) =
                    crate::deepseek_context::from_launch_values(&configuration, window, compact)
            {
                apply_anthropic_session_budget_env(&mut spec, &configuration, budget);
            }
        }
        apply_cli_executable(plugin_id, &mut spec, &get_env);
        m.insert(plugin_id, spec);
    }
    m
}

fn share_occupancy_command(
    plugin_id: &str,
    spec: &mut LaunchSpec,
    get_env: &impl Fn(&str) -> Option<String>,
) {
    let Some(slot) = crate::plugin_runtime_args::adapter_slot_for_provider(plugin_id) else {
        return;
    };
    let Some(primary) = crate::plugin_runtime_args::provider_for_adapter_slot(slot) else {
        return;
    };
    if primary == plugin_id
        || get_env(&crate::plugin_runtime_args::acp_env_key(plugin_id, "CMD")).is_some()
    {
        return;
    }
    let Some(command) = get_env(&crate::plugin_runtime_args::acp_env_key(primary, "CMD")) else {
        return;
    };
    spec.command = command;
    if get_env(&crate::plugin_runtime_args::acp_env_key(plugin_id, "ARGS")).is_none() {
        spec.args.clear();
    }
}

fn apply_cli_executable(
    plugin_id: &str,
    spec: &mut LaunchSpec,
    get_env: &impl Fn(&str) -> Option<String>,
) {
    let Some(slot) = crate::plugin_runtime_args::adapter_slot_for_provider(plugin_id) else {
        return;
    };
    let Some(primary) = crate::plugin_runtime_args::provider_for_adapter_slot(slot) else {
        return;
    };
    let Some(env_key) = crate::plugin_runtime_args::cli_executable_env(primary) else {
        return;
    };
    let Some(path) = get_env(&crate::plugin_runtime_args::acp_env_key(
        primary,
        "EXECUTABLE",
    ))
    .filter(|value| !value.trim().is_empty()) else {
        return;
    };
    spec.env.insert(env_key.to_owned(), path);
}

fn apply_isolated_launch(
    plugin_id: &str,
    spec: &mut LaunchSpec,
    get_env: &impl Fn(&str) -> Option<String>,
    isolated_shell: Option<&str>,
) {
    spec.env.extend(
        crate::plugin_runtime_args::environment(plugin_id)
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
    );
    if let Some(origin) = crate::plugin_runtime_args::loopback_origin_for(plugin_id)
        && let Some(env_key) = crate::plugin_runtime_args::loopback_env(plugin_id)
    {
        spec.env.insert(env_key.to_owned(), origin.to_owned());
    }
    spec.remove_env_prefixes =
        crate::plugin_runtime_args::remove_environment_prefixes(plugin_id).to_vec();
    let mut remove_env = crate::plugin_runtime_args::remove_environment(plugin_id).to_vec();
    if let Some(key) = crate::plugin_runtime_args::isolated_shell_acp_key(plugin_id) {
        remove_env.push(key);
    }
    remove_env.extend([
        crate::deepseek_context::SESSION_CONTEXT_WINDOW_ENV,
        crate::deepseek_context::SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
    ]);
    spec.remove_env = remove_env;
    if crate::plugin_runtime_args::isolated_shell(plugin_id) {
        let shell = isolated_shell
            .map(str::to_owned)
            .or_else(|| crate::claude_shell::resolve(plugin_id, get_env));
        if let Some(shell) = shell {
            for key in crate::plugin_runtime_args::isolated_shell_env(plugin_id) {
                spec.env.insert((*key).to_owned(), shell.clone());
            }
        }
    }
}

fn apply_openai_session_budget_args(
    spec: &mut LaunchSpec,
    configuration: &cowboy_provider_sdk::ConfigurationBehavior,
    budget: crate::deepseek_context::ContextBudget,
) {
    if !matches!(
        configuration,
        cowboy_provider_sdk::ConfigurationBehavior::OpenaiGatewayV1
    ) {
        return;
    }
    spec.args.extend([
        "-c".to_owned(),
        format!("model_context_window={}", budget.context_window),
        "-c".to_owned(),
        format!(
            "model_auto_compact_token_limit={}",
            budget.auto_compact_token_limit
        ),
    ]);
}

fn apply_anthropic_session_budget_env(
    spec: &mut LaunchSpec,
    configuration: &cowboy_provider_sdk::ConfigurationBehavior,
    budget: crate::deepseek_context::ContextBudget,
) {
    if !matches!(
        configuration,
        cowboy_provider_sdk::ConfigurationBehavior::AnthropicGatewayV1
    ) {
        return;
    }
    spec.env.insert(
        "CLAUDE_CODE_AUTO_COMPACT_WINDOW".to_owned(),
        budget.auto_compact_token_limit.to_string(),
    );
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
fn spec_with_custom_default_args(
    id: &'static str,
    default_cmd: &str,
    default_args: &[&str],
    custom_default_args: &[&str],
    get_env: &impl Fn(&str) -> Option<String>,
) -> LaunchSpec {
    let arg_override = get_env(&crate::plugin_runtime_args::acp_env_key(id, "ARGS")).map(|args| {
        shell_words::split(&args)
            .unwrap_or_else(|_| args.split_whitespace().map(str::to_owned).collect())
    });
    match get_env(&crate::plugin_runtime_args::acp_env_key(id, "CMD")) {
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
    apply_host_isolation(id, &mut spec)?;
    Some(spec)
}

fn apply_host_isolation(id: &str, spec: &mut LaunchSpec) -> Option<()> {
    if crate::plugin_runtime_args::isolated_shell(id) && !crate::claude_shell::available(id) {
        tracing::warn!(
            plugin_id = id,
            "isolated provider requires an executable absolute bash or zsh path"
        );
        return None;
    }
    let Some(env_key) = crate::plugin_runtime_args::isolated_home_env(id) else {
        return Some(());
    };
    let user_home = match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home),
        None => {
            tracing::warn!(plugin_id = id, "HOME is not set");
            return None;
        }
    };
    match prepare_isolated_home(id, env_key, &user_home) {
        Ok(home) => {
            spec.env
                .insert(env_key.to_owned(), home.display().to_string());
            Some(())
        }
        Err(error) => {
            tracing::warn!(
                %error,
                plugin_id = id,
                env_key,
                "failed to prepare isolated provider home"
            );
            None
        }
    }
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
    let package =
        parse_process_package(&bytes).context("validating exact Provider process package")?;
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
    let package = parse_process_package(&bytes).ok()?;
    if package.manifest.id != id {
        tracing::warn!(expected = id, actual = %package.manifest.id, "Provider package identity mismatch");
        return None;
    }
    Some(package)
}

fn parse_process_package(bytes: &[u8]) -> Result<cowboy_provider_sdk::ProviderPackage> {
    // The Machine already authenticated and pinned this immutable package.
    // Preserve the package's structural and host-contract validation while
    // allowing a package authored by an older compatible SDK to keep running.
    cowboy_provider_sdk::ProviderPackage::from_historical_bytes(bytes)
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

#[must_use]
pub fn provider_auth_required_detail(provider_id: &str, session_can_rebind: bool) -> String {
    let name = display_name(provider_id);
    let prefix = crate::provider_behavior::PROVIDER_AUTH_REQUIRED_PREFIX;
    if session_can_rebind {
        format!(
            "{prefix}{name} credentials expired or were rejected. Reconnect {name} in Settings > Providers, then retry this session."
        )
    } else {
        format!(
            "{prefix}{name} credentials expired or were rejected. Reconnect the same {name} account in Settings > Providers, then retry this session. Cowboy will reload its credentials without replacing the established session; switching accounts requires a new session."
        )
    }
}

/// Write only plugin-owned `settings.json` seed values. Ordinary Claude state
/// is never opened, copied, or linked across this boundary.
fn write_isolated_json_settings(plugin_id: &str, isolated: &Path) -> std::io::Result<()> {
    let provider_settings: serde_json::Map<String, serde_json::Value> =
        crate::plugin_runtime_args::isolated_home_settings(plugin_id)
            .into_iter()
            .flat_map(|settings| settings.iter())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
    let mut rendered = serde_json::to_string_pretty(&serde_json::Value::Object(provider_settings))
        .map_err(std::io::Error::other)?;
    rendered.push('\n');
    write_private_atomic(&isolated.join("settings.json"), rendered.as_bytes())
}

fn prepare_isolated_home(
    plugin_id: &str,
    env_key: &str,
    user_home: &Path,
) -> std::io::Result<PathBuf> {
    match env_key {
        "CODEX_HOME" => prepare_codex_home_at(user_home, plugin_id),
        "CLAUDE_CONFIG_DIR" => prepare_claude_config_dir_at(user_home, plugin_id),
        _ => prepare_generic_isolated_home_at(user_home, plugin_id),
    }
}

fn is_isolated_path_slug(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
}

fn prepare_isolated_provider_dir(
    user_home: &Path,
    plugin_id: &str,
    leaf: &str,
) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;

    if !is_isolated_path_slug(plugin_id) || !is_isolated_path_slug(leaf) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "isolated provider path is invalid",
        ));
    }
    let mut target = user_home.to_path_buf();
    for component in [".local", "state", "cowboy", "providers", plugin_id, leaf] {
        target.push(component);
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "isolated provider boundary must contain only real directories",
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
                                "isolated provider boundary must contain only real directories",
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
    Ok(target)
}

fn prepare_generic_isolated_home_at(user_home: &Path, plugin_id: &str) -> std::io::Result<PathBuf> {
    prepare_isolated_provider_dir(user_home, plugin_id, "home")
}

fn prepare_claude_config_dir_at(user_home: &Path, plugin_id: &str) -> std::io::Result<PathBuf> {
    let target = prepare_isolated_provider_dir(user_home, plugin_id, "claude-config")?;
    write_isolated_json_settings(plugin_id, &target)?;
    Ok(target)
}

fn prepare_codex_home_at(user_home: &Path, plugin_id: &str) -> std::io::Result<PathBuf> {
    let target = prepare_isolated_provider_dir(user_home, plugin_id, "codex-home")?;

    let catalog =
        PathBuf::from(crate::plugin_runtime_args::loopback_catalog(plugin_id).unwrap_or(""));
    let config = render_isolated_codex_config(plugin_id, &catalog);
    write_private_atomic(&target.join("config.toml"), config.as_bytes())?;
    Ok(target)
}

fn write_private_atomic(destination: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    static NEXT_PRIVATE_WRITE: AtomicU64 = AtomicU64::new(1);
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "private destination has no parent",
        )
    })?;
    let name = destination
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("state");
    let temporary = loop {
        let sequence = NEXT_PRIVATE_WRITE.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.{}.{}", std::process::id(), sequence));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let (temporary_path, mut file) = temporary;
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    drop(file);
    if let Err(error) = std::fs::rename(&temporary_path, destination) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(())
}

fn render_isolated_codex_config(plugin_id: &str, catalog: &Path) -> String {
    // Package-less fallback still pins the loopback gateway because the plugin
    // payload's base_url is a sidecar_url binding, not a literal.
    crate::plugin_runtime_args::isolated_config_toml(
        plugin_id,
        catalog,
        crate::plugin_runtime_args::loopback_origin_for(plugin_id).unwrap_or(""),
    )
}

#[cfg(test)]
mod tests {
    use crate::deepseek_context;
    use cowboy_provider_sdk::{StandardProviderSource, build_package, contract_fingerprint};
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
    fn worker_accepts_machine_validated_historical_provider_package() {
        let source: StandardProviderSource =
            serde_json::from_str(include_str!("../../plugins/codex/provider.json")).unwrap();
        let mut package = build_package(source.compile().unwrap()).unwrap();
        package.manifest.sdk_version = "2.4.0".to_owned();
        package.contract_fingerprint = contract_fingerprint(&package.manifest).unwrap();
        let bytes = serde_json::to_vec(&package).unwrap();

        assert!(cowboy_provider_sdk::ProviderPackage::from_bytes(&bytes).is_err());
        assert_eq!(
            super::parse_process_package(&bytes)
                .unwrap()
                .manifest
                .sdk_version,
            "2.4.0"
        );
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
        let mut codex_npx_args = vec!["-y", "@agentclientprotocol/codex-acp"];
        codex_npx_args.extend(
            crate::plugin_runtime_args::cli_arguments("codex")
                .iter()
                .copied(),
        );
        assert_eq!(codex.args, codex_npx_args);
        assert_eq!(
            lookup_with(&[], "gemini").map(|s| s.command),
            Some("npx".to_owned())
        );
        let grok = lookup_with(&[], "grok").expect("grok registered");
        assert_eq!(grok.command, "npx");
        let mut grok_npx_args = vec!["-y", "@xai-official/grok"];
        grok_npx_args.extend(
            crate::plugin_runtime_args::cli_arguments("grok")
                .iter()
                .copied(),
        );
        assert_eq!(grok.args, grok_npx_args);
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
            crate::plugin_runtime_args::cli_arguments("grok")
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
        assert!(deepseek.removes_inherited_env("CODEX_ACCESS_TOKEN"));
        assert!(deepseek.removes_inherited_env("CODEX_AUTH"));
        assert!(deepseek.removes_inherited_env("OPENAI_API_KEY"));
        assert!(deepseek.removes_inherited_env("OPENAI_ORGANIZATION"));
        assert!(deepseek.removes_inherited_env("CODEX_CONFIG"));
        assert!(deepseek.removes_inherited_env("DEEPSEEK_API_KEY"));

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
            crate::plugin_runtime_args::cli_arguments("codex")
        );
        let grok = lookup_with(
            &[("COWBOY_ACP_GROK_CMD", "/opt/npm-global/bin/grok")],
            "grok",
        )
        .expect("custom grok command");
        assert_eq!(grok.command, "/opt/npm-global/bin/grok");
        assert_eq!(grok.args, crate::plugin_runtime_args::cli_arguments("grok"));
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
            crate::plugin_runtime_args::loopback_origin_for("claude-deepseek")
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
            "ENABLE_CLAUDEAI_MCP_SERVERS",
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
        let rendered = super::render_isolated_codex_config(
            "codex-deepseek",
            std::path::Path::new(
                crate::plugin_runtime_args::loopback_catalog("codex-deepseek").unwrap(),
            ),
        );
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
    fn isolated_home_dispatch_uses_host_env_key() {
        let root = isolation_test_root("cowboy-isolated-home-dispatch");
        std::fs::create_dir_all(&root).expect("create isolated dispatch home");
        let codex = super::prepare_isolated_home("codex-deepseek", "CODEX_HOME", &root)
            .expect("prepare Codex home");
        assert_eq!(
            codex,
            root.join(".local/state/cowboy/providers/codex-deepseek/codex-home")
        );
        let claude = super::prepare_isolated_home("claude-deepseek", "CLAUDE_CONFIG_DIR", &root)
            .expect("prepare Claude config");
        assert_eq!(
            claude,
            root.join(".local/state/cowboy/providers/claude-deepseek/claude-config")
        );
        let generic = super::prepare_isolated_home("future-cli", "CUSTOM_HOME", &root)
            .expect("prepare generic home");
        assert_eq!(
            generic,
            root.join(".local/state/cowboy/providers/future-cli/home")
        );
        std::fs::remove_dir_all(&root).expect("remove isolated dispatch home");
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

        let isolated =
            super::prepare_codex_home_at(&root, "codex-deepseek").expect("prepare DeepSeek home");
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
    fn codex_deepseek_home_does_not_read_or_link_standard_plugins_and_skills() {
        let root = isolation_test_root("cowboy-codex-deepseek-no-sharing");
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

        let isolated =
            super::prepare_codex_home_at(&root, "codex-deepseek").expect("prepare DeepSeek home");

        assert!(!isolated.join("AGENTS.md").exists());
        assert!(!isolated.join("skills").exists());
        assert!(!isolated.join("plugins").exists());
        assert!(!isolated.join(".tmp/marketplaces").exists());

        let config = std::fs::read_to_string(isolated.join("config.toml")).expect("read config");
        assert!(!config.contains("[marketplaces.columbus]"));
        assert!(!config.contains("[plugins.\"columbus-harness@columbus\"]"));
        assert!(!config.contains("[hooks.state]"));
        assert!(config.contains("model = \"deepseek-v4-flash\""));
        assert!(!config.contains("gpt-secret-sentinel"));
        assert!(!config.contains("mcp_servers.private"));

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

        let isolated =
            super::prepare_codex_home_at(&root, "codex-deepseek").expect("prepare DeepSeek home");

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
    fn codex_deepseek_home_keeps_provider_scaffolding_without_standard_skills() {
        let root = isolation_test_root("cowboy-codex-deepseek-private-skills");
        let openai_home = root.join(".codex");
        std::fs::create_dir_all(openai_home.join("skills/omega")).expect("create shared skill");
        std::fs::create_dir_all(openai_home.join("skills/.system"))
            .expect("create ordinary system");
        let scaffolding =
            root.join(".local/state/cowboy/providers/codex-deepseek/codex-home/skills/.system");
        std::fs::create_dir_all(&scaffolding).expect("create provider scaffolding");
        std::fs::write(scaffolding.join("marker"), "provider owned").expect("write marker");

        let isolated =
            super::prepare_codex_home_at(&root, "codex-deepseek").expect("prepare DeepSeek home");

        assert!(!isolated.join("skills/omega").exists());
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

        let isolated = super::prepare_claude_config_dir_at(&root, "claude-deepseek")
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
    fn claude_deepseek_config_does_not_read_or_link_standard_plugins_and_skills() {
        let root = isolation_test_root("cowboy-claude-deepseek-no-sharing");
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

        let isolated = super::prepare_claude_config_dir_at(&root, "claude-deepseek")
            .expect("prepare isolated Claude config");

        assert!(!isolated.join("CLAUDE.md").exists());
        assert!(!isolated.join("skills").exists());
        assert!(!isolated.join("plugins").exists());
        assert!(!isolated.join(".credentials.json").exists());

        let settings: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(isolated.join("settings.json"))
                .expect("read provider settings"),
        )
        .expect("parse shared settings");
        assert_eq!(settings["autoCompactEnabled"], true);
        assert!(settings.get("enabledPlugins").is_none());
        assert!(settings.get("extraKnownMarketplaces").is_none());
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

        let isolated = super::prepare_claude_config_dir_at(&root, "claude-deepseek")
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
    fn isolated_homes_reject_symlink_boundaries_for_every_convention() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_TEST_HOME: AtomicU64 = AtomicU64::new(1);
        let root = std::env::temp_dir().join(format!(
            "cowboy-isolated-home-symlink-{}-{}",
            std::process::id(),
            NEXT_TEST_HOME.fetch_add(1, Ordering::Relaxed)
        ));
        for (plugin_id, env_key, leaf) in [
            ("codex-deepseek", "CODEX_HOME", "codex-home"),
            ("claude-deepseek", "CLAUDE_CONFIG_DIR", "claude-config"),
            ("future-cli", "CUSTOM_HOME", "home"),
        ] {
            for relative in [
                format!(".local/state/cowboy/providers/{plugin_id}"),
                format!(".local/state/cowboy/providers/{plugin_id}/{leaf}"),
            ] {
                let case = root.join(format!("{plugin_id}-{}", relative.replace('/', "-")));
                let boundary = case.join(&relative);
                std::fs::create_dir_all(boundary.parent().unwrap()).unwrap();
                let outside = case.join("outside");
                std::fs::create_dir_all(&outside).unwrap();
                symlink(&outside, &boundary).unwrap();

                let error = super::prepare_isolated_home(plugin_id, env_key, &case).unwrap_err();
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
            }
        }

        std::fs::remove_dir_all(&root).expect("remove symlink test home");
    }

    #[cfg(unix)]
    #[test]
    fn isolated_config_replaces_a_destination_symlink_without_following_it() {
        use std::os::unix::fs::symlink;

        let root = isolation_test_root("cowboy-isolated-config-symlink");
        std::fs::create_dir_all(&root).expect("create test home");
        let isolated = super::prepare_codex_home_at(&root, "codex-deepseek")
            .expect("prepare isolated Codex home");
        let outside = root.join("outside-config.toml");
        std::fs::write(&outside, "ordinary sentinel").expect("write outside config");
        std::fs::remove_file(isolated.join("config.toml")).expect("remove generated config");
        symlink(&outside, isolated.join("config.toml")).expect("link hostile destination");

        super::prepare_codex_home_at(&root, "codex-deepseek")
            .expect("regenerate isolated Codex home");

        assert_eq!(
            std::fs::read_to_string(&outside).unwrap(),
            "ordinary sentinel"
        );
        assert!(
            !std::fs::symlink_metadata(isolated.join("config.toml"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_dir_all(&root).expect("remove destination symlink test home");
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
                    super::prepare_claude_config_dir_at(&root, "claude-deepseek")
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

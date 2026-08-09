//! CLI for the stable Machine host.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use clap::{Parser, ValueEnum};
use futures::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Message;

use crate::machine_auth::MachineIdentity;
use crate::machine_broker::{MachineBrokerArgs, SpawnMode};
use crate::machine_components::ComponentStore;
use crate::machine_protocol::{
    AuthState, ComponentId, ComponentInventory, ComponentKind, ComponentState, ComponentUpdate,
    ConnectionMode, MACHINE_PROTOCOL_VERSION, MIN_MACHINE_PROTOCOL_VERSION, MachineCapacity,
    MachineCommand, MachineEvent, MachineFrame, MachineHello, MachineWorkspace, Platform,
};

struct LoginSession {
    cancel: tokio::sync::watch::Sender<bool>,
    input: tokio::sync::mpsc::UnboundedSender<String>,
}

type LoginSessions = Arc<parking_lot::Mutex<std::collections::HashMap<String, LoginSession>>>;

struct LoginIo {
    cancel: tokio::sync::watch::Receiver<bool>,
    input: tokio::sync::mpsc::UnboundedReceiver<String>,
    sessions: LoginSessions,
}

struct ControllerConfig {
    controller_url: String,
    machine_id: String,
    display_name: String,
    identity: MachineIdentity,
    runtime_socket: PathBuf,
    workspaces: Arc<WorkspaceConfig>,
    components: Arc<ComponentStore>,
    zed_adapter_socket: Option<PathBuf>,
    code_adapter_socket: Option<PathBuf>,
    worktree_root: PathBuf,
    capacity: MachineCapacity,
    local: bool,
    provider_usage: crate::provider_usage_spool::ProviderUsageSpool,
}

const DEFAULT_WORKSPACE_CONFIG: &str = "/etc/cowboy-machine/workspaces.json";

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceSnapshot {
    revision: Option<String>,
    workspaces: Vec<MachineWorkspace>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceFile {
    version: u16,
    revision: String,
    workspaces: Vec<String>,
}

struct WorkspaceConfig {
    path: PathBuf,
    fallback: Vec<String>,
    updates: tokio::sync::watch::Sender<WorkspaceSnapshot>,
}

impl WorkspaceConfig {
    fn new(path: PathBuf, fallback: Vec<String>) -> anyhow::Result<Self> {
        let snapshot = load_workspace_snapshot(&path, &fallback)?;
        let (updates, _) = tokio::sync::watch::channel(snapshot);
        Ok(Self {
            path,
            fallback,
            updates,
        })
    }

    fn snapshot(&self) -> WorkspaceSnapshot {
        self.updates.borrow().clone()
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<WorkspaceSnapshot> {
        self.updates.subscribe()
    }

    fn reload(&self) -> anyhow::Result<WorkspaceSnapshot> {
        let snapshot = load_workspace_snapshot(&self.path, &self.fallback)?;
        self.updates.send_if_modified(|current| {
            if *current == snapshot {
                false
            } else {
                current.clone_from(&snapshot);
                true
            }
        });
        Ok(snapshot)
    }
}

// The Machine and controller multiplex runtime traffic, adapter responses, and
// heartbeats over one full-duplex WebSocket. A large frame in each direction
// can otherwise leave both peers awaiting socket write capacity while neither
// returns to its read branch. Treat a stalled write as a dead connection so
// controller_loop reconnects and reattaches the surviving workers.
const MACHINE_FRAME_SEND_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSpawnMode {
    Direct,
    SystemdUser,
}

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
    // Keep this stable path for detached workers that survive Machine upgrades.
    #[arg(long, default_value = "/run/user/1000/cowboy/cowboy-machine.sock")]
    socket: PathBuf,
    /// Additional older broker endpoints accepted during a bounded migration.
    /// Newly launched workers always receive `--socket` instead.
    #[arg(long = "compat-socket")]
    compatibility_sockets: Vec<PathBuf>,
    #[arg(long, default_value = "cowboy-acp-worker")]
    worker_command: PathBuf,
    /// Optional bootstrap generation. Production leaves this empty and lets
    /// the active Cowboy controller declare the desired generation + exact
    /// worker executable over IPC.
    #[arg(long, default_value = "")]
    desired_generation: String,
    #[arg(long, value_enum, default_value_t = CliSpawnMode::Direct)]
    spawn_mode: CliSpawnMode,
    // ACP permits one 60 s initialize retry, then independently bounds session
    // establishment and startup configuration at 60 s each. Stay above that
    // 240 s worst case so the Machine host never preempts the phase-aware error.
    #[arg(long, default_value_t = 255)]
    worker_ready_timeout_seconds: u64,
    /// Cowboy controller base URL.
    #[arg(
        long,
        env = "COWBOY_MACHINE_CONTROLLER_URL",
        required_unless_present = "provider_usage_status"
    )]
    controller_url: Option<String>,
    #[arg(long, env = "COWBOY_MACHINE_ID", default_value = "local")]
    machine_id: String,
    #[arg(long, env = "COWBOY_MACHINE_DISPLAY_NAME")]
    display_name: Option<String>,
    #[arg(
        long,
        env = "COWBOY_MACHINE_STATE_DIR",
        default_value = ".cowboy-machine"
    )]
    state_dir: PathBuf,
    /// Print the durable provider-usage outbox status as JSON and exit. This
    /// opens the SQLite spool read-only and does not contact the controller.
    #[arg(long, default_value_t = false)]
    provider_usage_status: bool,
    /// Machine-local ingestion socket for provider gateways.
    #[arg(
        long,
        env = "COWBOY_MACHINE_PROVIDER_USAGE_SOCKET",
        default_value = "/run/cowboy/provider-usage.sock"
    )]
    provider_usage_socket: PathBuf,
    /// One-time secret. Prefer the environment variable so it is absent from
    /// shell history; omit after the first successful enrollment.
    #[arg(long, env = "COWBOY_MACHINE_ENROLLMENT_TOKEN")]
    enrollment_token: Option<String>,
    /// Mode-0600 one-time token file. It is removed after enrollment.
    #[arg(long, env = "COWBOY_MACHINE_ENROLLMENT_TOKEN_FILE")]
    enrollment_token_file: Option<PathBuf>,
    /// Trusted remote launch root in `id=/absolute/path` form. Repeat for each
    /// workspace the controller may target.
    #[arg(
        long = "workspace",
        env = "COWBOY_MACHINE_WORKSPACES",
        value_delimiter = ','
    )]
    workspaces: Vec<String>,
    /// Declarative workspace contract. When absent, `--workspace` remains the
    /// compatibility fallback for non-Nix and older installations.
    #[arg(
        long,
        env = "COWBOY_MACHINE_WORKSPACE_CONFIG",
        default_value = DEFAULT_WORKSPACE_CONFIG
    )]
    workspace_config: PathBuf,
    /// Ed25519/OpenSSH public key allowed to sign managed component artifacts.
    #[arg(long, env = "COWBOY_MACHINE_ARTIFACT_PUBLIC_KEY")]
    artifact_public_key: Option<PathBuf>,
    /// Unix socket of the Cowboy-managed versioned Zed adapter payload.
    #[arg(long, env = "COWBOY_MACHINE_ZED_ADAPTER_SOCKET")]
    zed_adapter_socket: Option<PathBuf>,
    /// Unix socket of the Cowboy-managed filesystem/Git adapter payload.
    #[arg(long, env = "COWBOY_MACHINE_CODE_ADAPTER_SOCKET")]
    code_adapter_socket: Option<PathBuf>,
    /// Maximum detached ACP sessions accepted by this Machine.
    #[arg(long, env = "COWBOY_MACHINE_MAX_SESSIONS", default_value_t = 8)]
    max_sessions: u32,
    /// Keep existing sessions alive while refusing new placement.
    #[arg(long, env = "COWBOY_MACHINE_DRAINING", default_value_t = false)]
    draining: bool,
    /// Mark this authenticated Machine as colocated with the controller for
    /// display and scheduling preference only. Runtime traffic still uses the
    /// normal Machine WebSocket protocol.
    #[arg(long, env = "COWBOY_MACHINE_LOCAL", default_value_t = false)]
    local: bool,
}

#[derive(Serialize)]
struct EnrollmentRequest<'a> {
    token: &'a str,
    public_key: &'a str,
}

#[derive(Deserialize)]
struct EnrollmentResponse {
    machine_id: String,
    fingerprint: String,
}

/// Parse the stable local-mode CLI and run the existing detached ACP broker.
///
/// # Errors
/// Returns when broker startup or its listener fails.
pub async fn run(command_name: &'static str) -> anyhow::Result<()> {
    // Unlike the full Cowboy daemon, the small Machine binary does not start
    // SQLx first. Make the shared TLS provider explicit instead of depending
    // on another subsystem's initialization order.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args =
        Args::parse_from(std::iter::once(command_name.to_owned()).chain(std::env::args().skip(1)));
    if args.provider_usage_status {
        let status = crate::provider_usage_spool::ProviderUsageSpool::read_status(
            &args.state_dir.join("provider-usage.sqlite3"),
        )?;
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }
    let runtime_socket = args.socket.clone();
    let bootstrap_acp_generation = if args.desired_generation.is_empty() {
        env!("CARGO_PKG_VERSION").to_owned()
    } else {
        args.desired_generation.clone()
    };
    let components = Arc::new(ComponentStore::new(
        args.state_dir.join("components"),
        args.artifact_public_key.as_deref(),
        bootstrap_acp_generation.clone(),
    )?);
    let active_acp = components
        .active()?
        .into_iter()
        .find(|(component, _)| component.id.kind == ComponentKind::AcpRuntime);
    let desired_generation = active_acp.as_ref().map_or_else(
        || bootstrap_acp_generation,
        |(component, _)| component.generation.clone(),
    );
    let worker_command =
        active_acp.map_or_else(|| args.worker_command.clone(), |(_, executable)| executable);
    let worker_environment = managed_provider_environment(&components, &worker_command)?;
    let worktree_root = args.state_dir.join("worktrees");
    let broker = MachineBrokerArgs {
        socket: args.socket,
        compatibility_sockets: args.compatibility_sockets,
        worker_command,
        desired_generation,
        spawn_mode: match args.spawn_mode {
            CliSpawnMode::Direct => SpawnMode::Direct,
            CliSpawnMode::SystemdUser => SpawnMode::SystemdUser,
        },
        worker_environment,
        worktree_root: worktree_root.clone(),
        worker_ready_timeout: std::time::Duration::from_secs(args.worker_ready_timeout_seconds),
    };
    let controller_url = args
        .controller_url
        .context("--controller-url is required unless --provider-usage-status is used")?;
    validate_controller_url(&controller_url)?;
    let identity = MachineIdentity::load_or_create(&args.state_dir)?;
    let display_name = args.display_name.unwrap_or_else(default_display_name);
    let workspaces = Arc::new(WorkspaceConfig::new(
        args.workspace_config,
        args.workspaces,
    )?);
    let file_token = args
        .enrollment_token_file
        .as_ref()
        .filter(|path| path.exists())
        .map(std::fs::read_to_string)
        .transpose()
        .context("reading Machine enrollment token file")?;
    if let Some(token) = args
        .enrollment_token
        .as_deref()
        .or(file_token.as_deref().map(str::trim))
    {
        enroll(&controller_url, token, identity.public_key()).await?;
        if let Some(path) = args.enrollment_token_file.as_ref() {
            std::fs::remove_file(path).context("removing consumed enrollment token file")?;
        }
    }
    let code_adapter_socket = args.code_adapter_socket.clone();
    let zed_adapter_socket = args.zed_adapter_socket.clone();
    let provider_usage = crate::provider_usage_spool::ProviderUsageSpool::open(
        &args.state_dir.join("provider-usage.sqlite3"),
    )?;
    let controller = controller_loop(ControllerConfig {
        controller_url,
        machine_id: args.machine_id,
        display_name,
        identity,
        runtime_socket,
        workspaces: Arc::clone(&workspaces),
        components: Arc::clone(&components),
        zed_adapter_socket: zed_adapter_socket.clone(),
        code_adapter_socket: code_adapter_socket.clone(),
        worktree_root,
        capacity: MachineCapacity {
            max_sessions: args.max_sessions.max(1),
            draining: args.draining,
        },
        local: args.local,
        provider_usage: provider_usage.clone(),
    });
    let provider_usage_listener =
        crate::provider_usage_spool::serve(args.provider_usage_socket, provider_usage);
    let code_adapter = supervise_code_adapter(
        Arc::clone(&components),
        code_adapter_socket,
        Arc::clone(&workspaces),
    );
    let zed_adapter = supervise_zed_adapter(
        Arc::clone(&components),
        zed_adapter_socket,
        args.state_dir.join("zed"),
    );
    tokio::try_join!(
        crate::machine_broker::run(broker),
        controller,
        provider_usage_listener,
        code_adapter,
        zed_adapter
    )?;
    Ok(())
}

fn managed_provider_environment(
    components: &ComponentStore,
    worker_command: &Path,
) -> anyhow::Result<BTreeMap<String, String>> {
    let disabled = disabled_provider_slots();
    let active = components.active()?;
    let has = |kind, slots: &[&str]| {
        active.iter().any(|(component, _)| {
            component.id.kind == kind
                && slots.contains(&component.id.slot.as_str())
                && !disabled.iter().any(|slot| slot == &component.id.slot)
        })
    };
    let installed = |kind, slots: &[&str]| {
        active.iter().any(|(component, _)| {
            component.id.kind == kind && slots.contains(&component.id.slot.as_str())
        })
    };
    let claude_enabled = claude_runtime_enabled(&disabled);
    let mut environment = BTreeMap::new();
    for key in [
        "COWBOY_ACP_CODEX_CMD",
        "COWBOY_ACP_CLAUDE_CODE_CMD",
        "COWBOY_ACP_CLAUDE_CODE_EXECUTABLE",
        "COWBOY_ACP_GEMINI_CMD",
        "COWBOY_ACP_GEMINI_ARGS",
    ] {
        let slot = match key {
            "COWBOY_ACP_CLAUDE_CODE_CMD" | "COWBOY_ACP_CLAUDE_CODE_EXECUTABLE" => "claude",
            "COWBOY_ACP_GEMINI_CMD" | "COWBOY_ACP_GEMINI_ARGS" => "gemini",
            _ => "codex",
        };
        if if slot == "claude" {
            !claude_enabled
        } else {
            disabled.iter().any(|disabled| disabled == slot)
        } {
            continue;
        }
        if let Ok(value) = std::env::var(key)
            && !value.trim().is_empty()
        {
            environment.insert(key.to_owned(), value);
        }
    }
    if !disabled.iter().any(|slot| slot == "claude-deepseek")
        && let Some(shell) = crate::claude_shell::resolve(&|key| std::env::var(key).ok())
    {
        // Pin the same readiness-checked path into every detached worker. This
        // preserves a nonstandard inherited shell across the systemd boundary;
        // the Claude DeepSeek launch spec removes this control variable before
        // starting the adapter.
        environment.insert("COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL".to_owned(), shell);
    }
    if has(ComponentKind::ProviderAdapter, &["codex"]) {
        environment.insert(
            "COWBOY_ACP_CODEX_CMD".to_owned(),
            components.command_path("codex-acp").display().to_string(),
        );
    }
    // Keep npm ownership of codex-acp while correcting its App Server resume
    // request at the worker boundary. A signed standalone ACP runtime may omit
    // the sibling shim and own its complete provider contract instead.
    let codex_proxy = worker_command.with_file_name("cowboy-codex-app-server");
    if !disabled.iter().any(|slot| slot == "codex") && codex_proxy.is_file() {
        environment.insert("CODEX_PATH".to_owned(), codex_proxy.display().to_string());
    }
    if claude_enabled && installed(ComponentKind::ProviderAdapter, &["claude", "claude-code"]) {
        environment.insert(
            "COWBOY_ACP_CLAUDE_CODE_CMD".to_owned(),
            components
                .command_path("claude-agent-acp")
                .display()
                .to_string(),
        );
    }
    if claude_enabled {
        let managed_cli = installed(ComponentKind::ProviderCli, &["claude"])
            .then(|| components.command_path("claude"));
        let configured = environment
            .get("COWBOY_ACP_CLAUDE_CODE_EXECUTABLE")
            .map(PathBuf::from);
        let adapter_sibling = environment
            .get("COWBOY_ACP_CLAUDE_CODE_CMD")
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(|parent| parent.join("claude")))
            .filter(|path| path.is_file());
        let path_command = command_on_path("claude");
        if let Some(executable) = managed_cli
            .or(configured)
            .or(adapter_sibling)
            .or(path_command)
        {
            // claude-agent-acp otherwise prefers the SDK's pinned optional
            // binary, which can lag the Machine's updated provider CLI.
            environment.insert(
                "COWBOY_ACP_CLAUDE_CODE_EXECUTABLE".to_owned(),
                executable.display().to_string(),
            );
        }
    }
    if has(ComponentKind::ProviderCli, &["gemini"]) {
        environment.insert(
            "COWBOY_ACP_GEMINI_CMD".to_owned(),
            components.command_path("gemini").display().to_string(),
        );
        environment.insert("COWBOY_ACP_GEMINI_ARGS".to_owned(), "--acp".to_owned());
    }
    Ok(environment)
}

fn command_on_path(command: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(command))
        .find(|candidate| candidate.is_file())
}

fn disabled_provider_slots() -> Vec<String> {
    disabled_provider_slots_from(
        &std::env::var("COWBOY_MACHINE_DISABLED_PROVIDERS").unwrap_or_default(),
    )
}

fn disabled_provider_slots_from(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|slot| !slot.is_empty())
        .map(|slot| match slot {
            "claude-code" => "claude".to_owned(),
            other => other.to_owned(),
        })
        .collect()
}

fn claude_runtime_enabled(disabled: &[String]) -> bool {
    ["claude", "claude-deepseek"]
        .iter()
        .any(|slot| !disabled.iter().any(|disabled| disabled == slot))
}

async fn supervise_zed_adapter(
    components: Arc<ComponentStore>,
    socket: Option<PathBuf>,
    state_dir: PathBuf,
) -> anyhow::Result<()> {
    let Some(socket) = socket else {
        return std::future::pending().await;
    };
    loop {
        let active = components.active()?;
        let Some((_, adapter, server)) = selected_zed_pair(&active) else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let mut child = tokio::process::Command::new(&adapter)
            .arg("serve")
            .arg("--socket")
            .arg(&socket)
            .arg("--zed-server")
            .arg(&server)
            .arg("--state-dir")
            .arg(&state_dir)
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", adapter.display()))?;
        loop {
            tokio::select! {
                status = child.wait() => {
                    tracing::warn!(?status, "Zed adapter exited");
                    break;
                }
                () = tokio::time::sleep(Duration::from_secs(2)) => {
                    let active = components.active().unwrap_or_default();
                    let current = selected_zed_pair(&active);
                    if current.as_ref().map(|(_, adapter, server)| (adapter, server))
                        != Some((&adapter, &server))
                    {
                        child.kill().await?;
                        let _ = child.wait().await;
                        break;
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn selected_zed_pair(
    active: &[(crate::machine_protocol::DesiredComponent, PathBuf)],
) -> Option<(String, PathBuf, PathBuf)> {
    active.iter().rev().find_map(|(adapter, adapter_path)| {
        (adapter.id.kind == ComponentKind::ZedAdapter).then(|| {
            active
                .iter()
                .find(|(server, _)| {
                    server.id.kind == ComponentKind::ZedServer && server.id.slot == adapter.id.slot
                })
                .map(|(_, server_path)| {
                    (
                        adapter.id.slot.clone(),
                        adapter_path.clone(),
                        server_path.clone(),
                    )
                })
        })?
    })
}

async fn supervise_code_adapter(
    components: Arc<ComponentStore>,
    socket: Option<PathBuf>,
    workspaces: Arc<WorkspaceConfig>,
) -> anyhow::Result<()> {
    let Some(socket) = socket else {
        return std::future::pending().await;
    };
    let mut workspace_updates = workspaces.subscribe();
    loop {
        let command = components.command_path("cowboy-code-adapter");
        let Ok(executable) = command.canonicalize() else {
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        };
        let workspace_snapshot = workspaces.snapshot();
        let mut child =
            tokio::process::Command::new(&executable)
                .arg("--socket")
                .arg(&socket)
                .args(workspace_snapshot.workspaces.iter().flat_map(|workspace| {
                    ["--workspace".to_owned(), workspace.canonical_path.clone()]
                }))
                .kill_on_drop(true)
                .spawn()
                .with_context(|| format!("starting {}", executable.display()))?;
        loop {
            tokio::select! {
                status = child.wait() => {
                    tracing::warn!(?status, "Code adapter exited");
                    break;
                }
                () = tokio::time::sleep(Duration::from_secs(2)) => {
                    if command.canonicalize().ok().as_ref() != Some(&executable) {
                        child.kill().await?;
                        let _ = child.wait().await;
                        break;
                    }
                }
                changed = workspace_updates.changed() => {
                    changed.context("workspace configuration channel closed")?;
                    child.kill().await?;
                    let _ = child.wait().await;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn validate_controller_url(value: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(value).context("parsing Machine controller URL")?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        bail!("remote Machine controller must use https; http is allowed only on loopback");
    }
    Ok(())
}

fn default_display_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Cowboy Machine".to_owned())
}

async fn enroll(controller_url: &str, token: &str, public_key: &str) -> anyhow::Result<()> {
    let endpoint = format!(
        "{}/api/machine/enroll",
        controller_url.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(endpoint)
        .json(&EnrollmentRequest { token, public_key })
        .send()
        .await
        .context("sending Machine enrollment")?
        .error_for_status()
        .context("Machine enrollment rejected")?
        .json::<EnrollmentResponse>()
        .await
        .context("decoding Machine enrollment response")?;
    tracing::info!(
        machine = %response.machine_id,
        fingerprint = %response.fingerprint,
        "Machine identity enrolled"
    );
    Ok(())
}

async fn controller_loop(config: ControllerConfig) -> anyhow::Result<()> {
    let mut retry = Duration::from_secs(1);
    loop {
        match controller_connection(&config).await {
            Ok(()) => retry = Duration::from_secs(1),
            Err(error) => tracing::warn!(%error, "Machine controller disconnected"),
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(Duration::from_secs(30));
    }
}

async fn controller_connection(config: &ControllerConfig) -> anyhow::Result<()> {
    let workspace_snapshot = config.workspaces.reload()?;
    let mut endpoint =
        reqwest::Url::parse(&config.controller_url).context("parsing controller URL")?;
    endpoint
        .set_scheme(if endpoint.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .map_err(|()| anyhow::anyhow!("invalid controller URL scheme"))?;
    endpoint.set_path("/api/machine/connect");
    endpoint.set_query(None);
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint.as_str())
        .await
        .context("connecting Machine WebSocket")?;
    let challenge = receive_frame(&mut socket).await?;
    let MachineFrame::Challenge {
        challenge_id,
        nonce,
        expires_at_ms,
    } = challenge
    else {
        bail!("controller did not begin with a challenge");
    };
    if expires_at_ms < unix_ms() {
        bail!("controller challenge already expired");
    }
    let mut hello = MachineHello {
        machine_id: config.machine_id.clone(),
        display_name: config.display_name.clone(),
        platform: current_platform(),
        arch: std::env::consts::ARCH.to_owned(),
        connection_mode: if config.local {
            ConnectionMode::LocalUds
        } else {
            ConnectionMode::OutboundTls
        },
        min_protocol: MIN_MACHINE_PROTOCOL_VERSION,
        max_protocol: MACHINE_PROTOCOL_VERSION,
        min_runtime_protocol: crate::runtime_wire::MIN_PROTOCOL_VERSION,
        max_runtime_protocol: crate::runtime_wire::PROTOCOL_VERSION,
        host_build: env!("CARGO_PKG_VERSION").to_owned(),
        challenge_id: Some(challenge_id.clone()),
        challenge_signature: None,
        components: collect_inventory(&config.components, config.zed_adapter_socket.as_deref())
            .await,
        workspaces: workspace_snapshot.workspaces,
        workspace_revision: workspace_snapshot.revision,
        capacity: config.capacity.clone(),
    };
    let proof =
        crate::machine_protocol::challenge_proof_v1(&challenge_id, &nonce, expires_at_ms, &hello);
    hello.challenge_signature = Some(config.identity.sign(&proof)?);
    send_frame(&mut socket, &MachineFrame::Hello { hello }).await?;
    let MachineFrame::Welcome {
        protocol,
        heartbeat_interval_ms,
        desired_components,
        ..
    } = receive_frame(&mut socket).await?
    else {
        bail!("controller rejected Machine hello");
    };
    tracing::info!(machine = %config.machine_id, "Machine controller authenticated");
    if !desired_components.is_empty() {
        let active = config.components.active().unwrap_or_default();
        let restart_host = desired_components.iter().any(|component| {
            component_requires_host_restart(&component.id.kind)
                && !active.iter().any(|(current, _)| {
                    current.id == component.id
                        && current.digest.eq_ignore_ascii_case(&component.digest)
                })
        });
        let events = reconcile_components(
            "welcome".to_owned(),
            desired_components,
            Arc::clone(&config.components),
        )
        .await;
        for event in events {
            send_frame(&mut socket, &MachineFrame::Event { event }).await?;
        }
        if restart_host {
            tokio::time::sleep(Duration::from_millis(250)).await;
            std::process::exit(75);
        }
    }
    let runtime = UnixStream::connect(&config.runtime_socket)
        .await
        .with_context(|| {
            format!(
                "connecting Machine broker socket {}",
                config.runtime_socket.display()
            )
        })?;
    let (runtime_reader, mut runtime_writer) = runtime.into_split();
    let mut runtime_reader = crate::runtime_wire::FrameReader::new(runtime_reader);
    let mut heartbeat =
        tokio::time::interval(Duration::from_millis(heartbeat_interval_ms.max(1_000)));
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_command_tx, mut runtime_command_rx) = tokio::sync::mpsc::unbounded_channel();
    let login_sessions: LoginSessions = Arc::default();
    heartbeat.tick().await;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                send_frame(&mut socket, &MachineFrame::Heartbeat { sent_at_ms: unix_ms() }).await?;
                if protocol >= 2
                    && let Some(event) = config.provider_usage.pending_batch()?
                {
                    send_frame(&mut socket, &MachineFrame::Event { event }).await?;
                }
            }
            event = event_rx.recv() => {
                let Some(event) = event else { continue };
                send_frame(&mut socket, &MachineFrame::Event { event }).await?;
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Ok(()),
                    Some(Ok(Message::Ping(value))) => socket.send(Message::Pong(value)).await?,
                    Some(Ok(Message::Text(text))) => {
                        let frame: MachineFrame = serde_json::from_str(&text)
                            .context("parsing Machine controller frame")?;
                        match frame {
                            MachineFrame::Runtime { frame } => {
                                let workspace_snapshot = config.workspaces.snapshot();
                                if let Some(rejection) = reject_untrusted_workspace(
                                    &frame,
                                    &workspace_snapshot.workspaces,
                                    &config.worktree_root,
                                ) {
                                    send_frame(&mut socket, &MachineFrame::Runtime { frame: rejection }).await?;
                                    continue;
                                }
                                crate::runtime_wire::write_frame(&mut runtime_writer, &frame).await?;
                            }
                            MachineFrame::Command { command } => {
                                if let MachineCommand::ProviderUsageAck { producer_id, sequence } = &command {
                                    config.provider_usage.acknowledge(producer_id, *sequence)?;
                                    continue;
                                }
                                handle_machine_command(command, MachineCommandContext {
                                    events: event_tx.clone(),
                                    components: Arc::clone(&config.components),
                                    zed_adapter_socket: config.zed_adapter_socket.clone(),
                                    code_adapter_socket: config.code_adapter_socket.clone(),
                                    worktree_root: config.worktree_root.clone(),
                                    workspaces: Arc::clone(&config.workspaces),
                                    login_sessions: Arc::clone(&login_sessions),
                                    runtime_commands: runtime_command_tx.clone(),
                                });
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
            frame = runtime_reader.next() => {
                let Some(frame) = frame? else {
                    bail!("Machine broker runtime tunnel closed");
                };
                send_frame(&mut socket, &MachineFrame::Runtime { frame }).await?;
            }
            command = runtime_command_rx.recv() => {
                if let Some(command) = command {
                    crate::runtime_wire::write_frame(&mut runtime_writer, &command).await?;
                }
            }
        }
    }
}

async fn collect_inventory(
    store: &ComponentStore,
    zed_adapter_socket: Option<&std::path::Path>,
) -> Vec<ComponentInventory> {
    let disabled = disabled_provider_slots();
    let mut inventory = vec![ComponentInventory {
        id: ComponentId {
            kind: ComponentKind::MachineHost,
            slot: String::new(),
        },
        state: ComponentState::Active,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        generation: env!("CARGO_PKG_VERSION").to_owned(),
        digest: String::new(),
        rollback_generation: None,
        active_leases: 1,
        auth: None,
        detail: None,
        update: None,
    }];
    let mut managed = store.active().unwrap_or_default();
    managed.retain(|(component, _)| {
        !matches!(
            component.id.kind,
            ComponentKind::ProviderAdapter | ComponentKind::ProviderCli
        ) || !disabled.contains(&component.id.slot)
    });
    let has_managed_acp = managed
        .iter()
        .any(|(component, _)| component.id.kind == ComponentKind::AcpRuntime);
    inventory.extend(managed.into_iter().map(|(component, _)| {
        let rollback_generation = store.rollback_generation(&component);
        ComponentInventory {
            id: component.id,
            state: ComponentState::Active,
            version: component.version,
            generation: component.generation,
            digest: component.digest,
            rollback_generation,
            active_leases: 0,
            auth: None,
            detail: None,
            update: None,
        }
    }));
    if !has_managed_acp {
        inventory.push(bootstrap_acp_inventory(store.bootstrap_acp_generation()));
    }
    for (slot, command, version_args) in [
        ("codex", "codex", &["--version"][..]),
        ("claude", "claude", &["--version"][..]),
        ("gemini", "gemini", &["--version"][..]),
    ] {
        if disabled.iter().any(|disabled| disabled == slot) {
            continue;
        }
        let output = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::process::Command::new(command)
                .args(version_args)
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let (state, version, detail) = match output {
            Ok(Ok(output)) if output.status.success() => (
                ComponentState::Active,
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                None,
            ),
            Ok(Ok(output)) => (
                ComponentState::Failed,
                String::new(),
                Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
            ),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                (ComponentState::Missing, String::new(), None)
            }
            Ok(Err(error)) => (
                ComponentState::Failed,
                String::new(),
                Some(error.to_string()),
            ),
            Err(_) => (
                ComponentState::Failed,
                String::new(),
                Some("version probe timed out".to_owned()),
            ),
        };
        let kind = ComponentKind::ProviderCli;
        let (auth, auth_detail) = match slot {
            "codex" => (probe_exit_auth("codex", &["login", "status"]).await, None),
            "claude" => (
                probe_exit_auth("claude", &["auth", "status", "--json"]).await,
                None,
            ),
            "gemini" => {
                let probe = probe_gemini_auth();
                (probe.state, probe.detail)
            }
            _ => (AuthState::Unsupported, None),
        };
        if let Some(existing) = inventory
            .iter_mut()
            .find(|component| component.id.kind == kind && component.id.slot == slot)
        {
            existing.auth = Some(auth);
            existing.detail = auth_detail.or(existing.detail.take());
            continue;
        }
        inventory.push(ComponentInventory {
            id: ComponentId {
                kind,
                slot: slot.to_owned(),
            },
            state,
            version,
            generation: String::new(),
            digest: String::new(),
            rollback_generation: None,
            active_leases: 0,
            auth: Some(auth),
            detail: auth_detail.or(detail),
            update: None,
        });
    }
    inventory.push(probe_zed_inventory(zed_adapter_socket).await);
    for (slot, command) in [("codex", "codex-acp"), ("claude", "claude-agent-acp")] {
        if disabled.iter().any(|disabled| disabled == slot) {
            continue;
        }
        if inventory.iter().any(|component| {
            component.id.kind == ComponentKind::ProviderAdapter && component.id.slot == slot
        }) {
            continue;
        }
        let output = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::process::Command::new(command)
                .arg("--version")
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let (state, version, detail) = match output {
            Ok(Ok(output)) if output.status.success() => (
                ComponentState::Active,
                String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                Some("bootstrap adapter".to_owned()),
            ),
            Ok(Ok(output)) => (
                ComponentState::Failed,
                String::new(),
                Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
            ),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                (ComponentState::Missing, String::new(), None)
            }
            Ok(Err(error)) => (
                ComponentState::Failed,
                String::new(),
                Some(error.to_string()),
            ),
            Err(_) => (
                ComponentState::Failed,
                String::new(),
                Some("version probe timed out".to_owned()),
            ),
        };
        inventory.push(ComponentInventory {
            id: ComponentId {
                kind: ComponentKind::ProviderAdapter,
                slot: slot.to_owned(),
            },
            state,
            version,
            generation: String::new(),
            digest: String::new(),
            rollback_generation: None,
            active_leases: 0,
            auth: None,
            detail,
            update: None,
        });
    }
    if !disabled.iter().any(|slot| slot == "codex-deepseek") {
        let deepseek_ready = crate::provider_catalog::available_codex_deepseek_catalog().is_some()
            && tokio::time::timeout(
                Duration::from_millis(500),
                reqwest::Client::new()
                    .get("http://127.0.0.1:61137/healthz")
                    .send(),
            )
            .await
            .is_ok_and(|result| result.is_ok_and(|response| response.status().is_success()));
        inventory.push(ComponentInventory {
            id: ComponentId {
                kind: ComponentKind::ProviderAdapter,
                slot: "codex-deepseek".to_owned(),
            },
            state: if deepseek_ready {
                ComponentState::Active
            } else {
                ComponentState::Missing
            },
            version: String::new(),
            generation: String::new(),
            digest: String::new(),
            rollback_generation: None,
            active_leases: 0,
            auth: None,
            detail: Some("loopback Responses gateway".to_owned()),
            update: None,
        });
    }
    if !disabled.iter().any(|slot| slot == "claude-deepseek") {
        let gateway_ready = tokio::time::timeout(
            Duration::from_millis(500),
            reqwest::Client::new()
                .get("http://127.0.0.1:61138/healthz")
                .send(),
        )
        .await
        .is_ok_and(|result| result.is_ok_and(|response| response.status().is_success()));
        let shell_ready = crate::claude_shell::available();
        inventory.push(ComponentInventory {
            id: ComponentId {
                kind: ComponentKind::ProviderAdapter,
                slot: "claude-deepseek".to_owned(),
            },
            state: if gateway_ready && shell_ready {
                ComponentState::Active
            } else {
                ComponentState::Missing
            },
            version: String::new(),
            generation: String::new(),
            digest: String::new(),
            rollback_generation: None,
            active_leases: 0,
            auth: None,
            detail: Some(if shell_ready {
                "isolated loopback Anthropic Messages gateway".to_owned()
            } else {
                "Claude Code requires an executable absolute bash or zsh path".to_owned()
            }),
            update: None,
        });
    }
    apply_npm_release_status(&mut inventory).await;
    inventory
}

fn bootstrap_acp_inventory(generation: &str) -> ComponentInventory {
    ComponentInventory {
        id: ComponentId {
            kind: ComponentKind::AcpRuntime,
            slot: String::new(),
        },
        state: ComponentState::Active,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        generation: generation.to_owned(),
        digest: String::new(),
        rollback_generation: None,
        active_leases: 0,
        auth: None,
        detail: Some("bootstrap generation".to_owned()),
        update: None,
    }
}

const NPM_COMPONENTS: &[(ComponentKind, &str, &str)] = &[
    (ComponentKind::ProviderCli, "codex", "@openai/codex"),
    (
        ComponentKind::ProviderCli,
        "claude",
        "@anthropic-ai/claude-code",
    ),
    (ComponentKind::ProviderCli, "gemini", "@google/gemini-cli"),
    (
        ComponentKind::ProviderAdapter,
        "codex",
        "@agentclientprotocol/codex-acp",
    ),
    (
        ComponentKind::ProviderAdapter,
        "claude",
        "@agentclientprotocol/claude-agent-acp",
    ),
];

static NPM_UPDATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn npm_package_for_component(id: &ComponentId) -> Option<&'static str> {
    NPM_COMPONENTS
        .iter()
        .find(|(kind, slot, _)| &id.kind == kind && id.slot == *slot)
        .map(|(_, _, package)| *package)
}

async fn apply_npm_release_status(inventory: &mut [ComponentInventory]) {
    let (installed, outdated) = tokio::join!(
        npm_json(&["list", "--global", "--depth=0", "--json"]),
        npm_json(&["outdated", "--global", "--json"]),
    );
    let Some(installed) = installed else {
        return;
    };
    let Some(dependencies) = installed
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    let Some(outdated) = outdated.as_ref().and_then(serde_json::Value::as_object) else {
        // A successful install probe is not evidence that the registry check
        // succeeded. Keep freshness unknown rather than claiming everything is
        // current after an offline or timed-out `npm outdated`.
        return;
    };
    let checked_at_ms = unix_ms();
    for component in inventory {
        let Some((_, _, package)) = NPM_COMPONENTS
            .iter()
            .find(|(kind, slot, _)| &component.id.kind == kind && component.id.slot == *slot)
        else {
            continue;
        };
        let Some(current) = dependencies
            .get(*package)
            .and_then(|value| value.get("version"))
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let latest = outdated
            .get(*package)
            .and_then(|value| value.get("latest"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(current);
        component.update = Some(ComponentUpdate {
            latest_version: latest.to_owned(),
            available: current != latest,
            source: "npm registry".to_owned(),
            checked_at_ms,
            installable: true,
        });
    }
}

fn npm_update_is_confirmed_by_inventory(
    id: &ComponentId,
    inventory: &[ComponentInventory],
) -> bool {
    inventory.iter().any(|component| {
        component.id == *id
            && !component.version.is_empty()
            && component.update.as_ref().is_some_and(|update| {
                update.installable
                    && !update.available
                    && !update.latest_version.is_empty()
                    && update.source == "npm registry"
            })
    })
}

async fn update_npm_component(id: &ComponentId) -> anyhow::Result<()> {
    let package = npm_package_for_component(id).context("component has no npm update channel")?;
    let _guard = NPM_UPDATE_LOCK.lock().await;
    let status = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::process::Command::new("npm")
            .args([
                "install",
                "--global",
                "--no-audit",
                "--no-fund",
                &format!("{package}@latest"),
            ])
            .env("NO_UPDATE_NOTIFIER", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("npm update timed out")??;
    if status.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&status.stderr).trim().to_owned();
        bail!(if detail.is_empty() {
            "npm update failed".to_owned()
        } else {
            detail
        })
    }
}

async fn npm_json(args: &[&str]) -> Option<serde_json::Value> {
    let output = tokio::time::timeout(
        Duration::from_secs(8),
        tokio::process::Command::new("npm")
            .args(args)
            .env("NO_UPDATE_NOTIFIER", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    // `npm outdated` deliberately exits non-zero when updates exist, so its
    // JSON stdout is authoritative regardless of the process status.
    serde_json::from_slice(&output.stdout).ok()
}

async fn probe_zed_inventory(socket: Option<&std::path::Path>) -> ComponentInventory {
    let mut component = ComponentInventory {
        id: ComponentId {
            kind: ComponentKind::ZedServer,
            slot: "zed".to_owned(),
        },
        state: ComponentState::Missing,
        version: String::new(),
        generation: String::new(),
        digest: String::new(),
        rollback_generation: None,
        active_leases: 0,
        auth: None,
        detail: Some("Cowboy Zed adapter is not configured".to_owned()),
        update: None,
    };
    let Some(socket) = socket else {
        return component;
    };
    let probe = tokio::time::timeout(Duration::from_secs(3), async {
        let stream = UnixStream::connect(socket).await?;
        let (reader, mut writer) = stream.into_split();
        writer.write_all(b"{\"type\":\"health\"}\n").await?;
        let mut line = String::new();
        BufReader::new(reader).read_line(&mut line).await?;
        Ok::<_, anyhow::Error>(serde_json::from_str::<serde_json::Value>(&line)?)
    })
    .await;
    match probe {
        Ok(Ok(value))
            if value.get("type").and_then(serde_json::Value::as_str) == Some("health") =>
        {
            component.state = ComponentState::Active;
            component.version = value
                .get("zed_version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            component.detail = Some("Cowboy isolated Zed integration".to_owned());
        }
        Ok(Ok(_)) => {
            component.state = ComponentState::Failed;
            component.detail = Some("Zed adapter returned an invalid health response".to_owned());
        }
        Ok(Err(error)) => {
            component.state = ComponentState::Failed;
            component.detail = Some(error.to_string());
        }
        Err(_) => {
            component.state = ComponentState::Failed;
            component.detail = Some("Zed adapter health probe timed out".to_owned());
        }
    }
    component
}

async fn probe_exit_auth(command: &str, args: &[&str]) -> AuthState {
    match tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new(command)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    {
        Ok(Ok(output)) if output.status.success() => AuthState::SignedIn,
        Ok(Ok(_)) => AuthState::SignedOut,
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => AuthState::Unsupported,
        Ok(Err(_)) | Err(_) => AuthState::Error,
    }
}

const GEMINI_CONSUMER_LOGIN_RETIRED: &str = "Personal Google Login is no longer supported by Gemini CLI. Use Antigravity for personal, Google AI Pro, or Google AI Ultra accounts. Cowboy Gemini sessions require GEMINI_API_KEY or a Code Assist Standard/Enterprise Google Cloud project.";

#[derive(Debug, PartialEq, Eq)]
struct GeminiAuthProbe {
    state: AuthState,
    detail: Option<String>,
}

fn probe_gemini_auth() -> GeminiAuthProbe {
    let Some(home) = std::env::var_os("HOME") else {
        return GeminiAuthProbe {
            state: AuthState::SignedOut,
            detail: Some(
                "HOME is not configured, so Gemini credentials cannot be discovered.".to_owned(),
            ),
        };
    };
    let root = PathBuf::from(home).join(".gemini");
    let api_key = gemini_env_configured(&root, "GEMINI_API_KEY");
    let vertex_enabled = gemini_env_value(&root, "GOOGLE_GENAI_USE_VERTEXAI")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let vertex = vertex_enabled
        && (gemini_env_configured(&root, "GOOGLE_CLOUD_PROJECT")
            || gemini_env_configured(&root, "GOOGLE_API_KEY"));
    let selected = std::fs::read(root.join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value
                .pointer("/security/auth/selectedType")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    let gateway = std::env::var("GOOGLE_GEMINI_BASE_URL")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    let code_assist_project = gemini_env_configured(&root, "GOOGLE_CLOUD_PROJECT");
    gemini_auth_from_metadata(
        selected.as_deref(),
        root.join("oauth_creds.json").is_file(),
        api_key,
        vertex,
        gateway,
        code_assist_project,
    )
}

fn gemini_env_configured(root: &Path, key: &str) -> bool {
    gemini_env_value(root, key).is_some_and(|value| !value.trim().is_empty())
}

fn gemini_env_value(root: &Path, key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(key)
        && !value.trim().is_empty()
    {
        return Some(value);
    }
    let contents = std::fs::read_to_string(root.join(".env")).ok()?;
    gemini_env_value_from(&contents, key)
}

fn gemini_env_value_from(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (candidate, value) = line.split_once('=')?;
        (candidate.trim() == key).then(|| {
            value
                .trim()
                .trim_matches(|character| character == '\'' || character == '"')
                .to_owned()
        })
    })
}

fn gemini_auth_from_metadata(
    selected: Option<&str>,
    oauth_credentials: bool,
    api_key: bool,
    vertex: bool,
    gateway: bool,
    code_assist_project: bool,
) -> GeminiAuthProbe {
    match selected {
        Some("oauth-personal") if oauth_credentials && code_assist_project => GeminiAuthProbe {
            state: AuthState::SignedIn,
            detail: Some("Code Assist Standard/Enterprise Google Login".to_owned()),
        },
        Some("oauth-personal") if oauth_credentials => GeminiAuthProbe {
            state: AuthState::SignedOut,
            detail: Some(GEMINI_CONSUMER_LOGIN_RETIRED.to_owned()),
        },
        Some("gemini-api-key") if api_key => GeminiAuthProbe { state: AuthState::SignedIn, detail: Some("Gemini API key".to_owned()) },
        Some("vertex-ai" | "compute-default-credentials") if vertex => GeminiAuthProbe { state: AuthState::SignedIn, detail: Some("Vertex AI".to_owned()) },
        Some("gateway") if gateway => GeminiAuthProbe { state: AuthState::SignedIn, detail: Some("Gemini API gateway".to_owned()) },
        None if api_key => GeminiAuthProbe { state: AuthState::SignedIn, detail: Some("Gemini API key".to_owned()) },
        None if vertex => GeminiAuthProbe { state: AuthState::SignedIn, detail: Some("Vertex AI".to_owned()) },
        Some(_) | None => GeminiAuthProbe {
            state: AuthState::SignedOut,
            detail: Some("Configure GEMINI_API_KEY, Vertex AI, or a Code Assist Standard/Enterprise Google Cloud project.".to_owned()),
        },
    }
}

struct MachineCommandContext {
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    components: Arc<ComponentStore>,
    zed_adapter_socket: Option<PathBuf>,
    code_adapter_socket: Option<PathBuf>,
    worktree_root: PathBuf,
    workspaces: Arc<WorkspaceConfig>,
    login_sessions: LoginSessions,
    runtime_commands: tokio::sync::mpsc::UnboundedSender<crate::runtime_wire::Frame>,
}

fn handle_machine_command(command: MachineCommand, context: MachineCommandContext) {
    let MachineCommandContext {
        events,
        components,
        zed_adapter_socket,
        code_adapter_socket,
        worktree_root,
        workspaces,
        login_sessions,
        runtime_commands,
    } = context;
    match command {
        MachineCommand::RefreshInventory { request_id } => {
            tokio::spawn(async move {
                let components =
                    collect_inventory(&components, zed_adapter_socket.as_deref()).await;
                let workspace_result = workspaces.reload();
                if let Ok(snapshot) = &workspace_result {
                    let _ = events.send(MachineEvent::Inventory {
                        components,
                        workspaces: Some(snapshot.workspaces.clone()),
                        workspace_revision: snapshot.revision.clone(),
                        observed_at_ms: unix_ms(),
                    });
                }
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted: workspace_result.is_ok(),
                    detail: workspace_result
                        .err()
                        .map(|error| format!("workspace configuration reload failed: {error:#}")),
                });
            });
        }
        MachineCommand::BeginLogin {
            request_id,
            provider,
            auth_method,
        } => {
            let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
            let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
            login_sessions.lock().insert(
                request_id.clone(),
                LoginSession {
                    cancel: cancel_tx,
                    input: input_tx,
                },
            );
            tokio::spawn(run_login(
                request_id,
                provider,
                auth_method,
                events,
                components,
                zed_adapter_socket,
                LoginIo {
                    cancel: cancel_rx,
                    input: input_rx,
                    sessions: login_sessions,
                },
            ));
        }
        MachineCommand::AdapterRequest {
            request_id,
            adapter,
            payload,
        } => {
            tokio::spawn(run_adapter_request(
                request_id,
                adapter,
                payload,
                AdapterRequestContext {
                    zed_adapter_socket,
                    code_adapter_socket,
                    worktree_root,
                    workspaces: workspaces.snapshot().workspaces,
                    events,
                },
            ));
        }
        MachineCommand::Reconcile {
            request_id,
            components: desired,
        } => {
            tokio::spawn(async move {
                let active = components.active().unwrap_or_default();
                let restart_host = desired.iter().any(|component| {
                    component_requires_host_restart(&component.id.kind)
                        && !active.iter().any(|(current, _)| {
                            current.id == component.id
                                && current.digest.eq_ignore_ascii_case(&component.digest)
                        })
                });
                let result = reconcile_components(request_id, desired, components).await;
                let accepted = result.iter().any(|event| {
                    matches!(event, MachineEvent::CommandResult { accepted: true, .. })
                });
                for event in result {
                    let _ = events.send(event);
                }
                if restart_host && accepted {
                    // The service manager owns host replacement. Payload
                    // processes and state survive; exit only after the result
                    // has had a chance to reach the controller.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    std::process::exit(75);
                }
            });
        }
        MachineCommand::CancelLogin { request_id } => {
            let accepted = login_sessions
                .lock()
                .remove(&request_id)
                .is_some_and(|session| session.cancel.send(true).is_ok());
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted,
                detail: (!accepted).then_some("login request is not active".to_owned()),
            });
        }
        MachineCommand::SubmitLoginCode { request_id, code } => {
            let code = code.trim();
            let accepted = !code.is_empty()
                && login_sessions
                    .lock()
                    .get(&request_id)
                    .is_some_and(|session| session.input.send(code.to_owned()).is_ok());
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted,
                detail: (!accepted).then_some("login request is not accepting a code".to_owned()),
            });
        }
        MachineCommand::UpdateNpmComponent {
            request_id,
            component,
        } => {
            tokio::spawn(async move {
                let result = update_npm_component(&component).await;
                let inventory = collect_inventory(&components, zed_adapter_socket.as_deref()).await;
                let recovered =
                    result.is_err() && npm_update_is_confirmed_by_inventory(&component, &inventory);
                let accepted = result.is_ok() || recovered;
                if recovered && let Err(error) = &result {
                    tracing::warn!(
                        component = ?component,
                        error = %error,
                        "npm update reported an error but inventory confirms the latest release"
                    );
                }
                if accepted && let Some(provider) = provider_for_component(&component) {
                    let _ = runtime_commands.send(crate::runtime_wire::Frame::CoreCommand {
                        command: crate::runtime_wire::CoreCommand::RollProvider {
                            provider: provider.to_owned(),
                        },
                    });
                }
                let _ = events.send(MachineEvent::Inventory {
                    components: inventory,
                    workspaces: None,
                    workspace_revision: None,
                    observed_at_ms: unix_ms(),
                });
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted,
                    detail: match result {
                        Ok(()) => None,
                        Err(_) if recovered => Some(
                            "npm reported an error, but the installed component is up to date"
                                .to_owned(),
                        ),
                        Err(error) => Some(format!("{error:#}")),
                    },
                });
            });
        }
        MachineCommand::ProviderUsageAck { .. } => {
            // Consumed synchronously by the controller connection so the
            // durable spool advances before another batch is selected.
        }
    }
}

fn provider_for_component(id: &ComponentId) -> Option<&'static str> {
    match id.slot.as_str() {
        "codex" => Some("codex"),
        "claude" => Some("claude-code"),
        "gemini" => Some("gemini"),
        _ => None,
    }
}

fn component_requires_host_restart(kind: &ComponentKind) -> bool {
    matches!(
        kind,
        ComponentKind::MachineHost
            | ComponentKind::AcpRuntime
            | ComponentKind::ProviderAdapter
            | ComponentKind::ProviderCli
            | ComponentKind::ManagedNode
    )
}

struct AdapterRequestContext {
    zed_adapter_socket: Option<PathBuf>,
    code_adapter_socket: Option<PathBuf>,
    worktree_root: PathBuf,
    workspaces: Vec<MachineWorkspace>,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeepseekCacheStatusRequest {
    provider: String,
    session_id: String,
}

async fn run_adapter_request(
    request_id: String,
    adapter: String,
    payload: serde_json::Value,
    context: AdapterRequestContext,
) {
    let AdapterRequestContext {
        zed_adapter_socket,
        code_adapter_socket,
        worktree_root,
        workspaces,
        events,
    } = context;
    let result = async {
        if adapter == "deepseek-cache-status" {
            let request: DeepseekCacheStatusRequest = serde_json::from_value(payload)
                .context("decoding DeepSeek cache status request")?;
            if !crate::deepseek_cache::supported_provider(&request.provider)
                || request.session_id.is_empty()
                || request.session_id.len() > 256
                || !request
                    .session_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                bail!("invalid DeepSeek cache status request");
            }
            return crate::deepseek_cache::local_snapshot_status(
                &request.provider,
                &request.session_id,
            )
            .await;
        }
        if adapter == "workspace" {
            let request: crate::session_workspace::PrepareWorkspaceRequest =
                serde_json::from_value(payload)
                    .context("decoding workspace preparation request")?;
            validate_session_workspace_root(&request.root, &workspaces)?;
            return serde_json::to_value(
                crate::session_workspace::prepare(request, &worktree_root).await?,
            )
            .context("encoding prepared workspace");
        }
        let socket = match adapter.as_str() {
            "zed" => zed_adapter_socket.context("Zed adapter is not configured on this Machine")?,
            "code" => {
                code_adapter_socket.context("Code adapter is not configured on this Machine")?
            }
            _ => bail!("unknown Machine adapter {adapter:?}"),
        };
        if matches!(adapter.as_str(), "zed" | "code") {
            validate_adapter_workspace(&payload, &workspaces, &worktree_root)?;
        }
        let stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(&socket))
            .await
            .context("Zed adapter connect timed out")??;
        let (read, mut write) = stream.into_split();
        use tokio::io::AsyncWriteExt as _;
        write.write_all(payload.to_string().as_bytes()).await?;
        write.write_all(b"\n").await?;
        write.shutdown().await?;
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(35),
            tokio::io::BufReader::new(read).read_line(&mut line),
        )
        .await
        .context("Machine adapter response timed out")??;
        let response: serde_json::Value =
            serde_json::from_str(&line).context("decoding Machine adapter response")?;
        if response.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
            bail!(
                "{}",
                response
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("adapter rejected request")
            );
        }
        Ok(response.get("value").cloned().unwrap_or(response))
    }
    .await;
    let event = match result {
        Ok(payload) => MachineEvent::AdapterResponse {
            request_id,
            accepted: true,
            payload: Some(payload),
            detail: None,
        },
        Err(error) => MachineEvent::AdapterResponse {
            request_id,
            accepted: false,
            payload: None,
            detail: Some(format!("{error:#}")),
        },
    };
    let _ = events.send(event);
}

fn validate_session_workspace_root(
    value: &str,
    workspaces: &[MachineWorkspace],
) -> anyhow::Result<()> {
    let canonical = PathBuf::from(value)
        .canonicalize()
        .with_context(|| format!("canonicalizing session workspace {value}"))?;
    if !workspaces
        .iter()
        .any(|workspace| canonical == Path::new(&workspace.canonical_path))
    {
        bail!("session workspace is not an advertised Machine root");
    }
    Ok(())
}

fn validate_adapter_workspace(
    payload: &serde_json::Value,
    workspaces: &[MachineWorkspace],
    worktree_root: &Path,
) -> anyhow::Result<()> {
    for key in ["path", "worktree"] {
        let Some(value) = payload.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            continue;
        }
        let canonical = path
            .canonicalize()
            .with_context(|| format!("canonicalizing {value}"))?;
        if !workspace_path_allowed(&canonical, workspaces, worktree_root) {
            bail!("adapter path is outside the trusted Machine workspaces");
        }
    }
    Ok(())
}

async fn reconcile_components(
    request_id: String,
    desired: Vec<crate::machine_protocol::DesiredComponent>,
    store: Arc<ComponentStore>,
) -> Vec<MachineEvent> {
    let mut inventory = Vec::with_capacity(desired.len());
    for component in desired {
        match store.reconcile(component).await {
            Ok(component) => inventory.push(component),
            Err(error) => {
                return vec![MachineEvent::CommandResult {
                    request_id,
                    accepted: false,
                    detail: Some(format!("component reconciliation failed: {error:#}")),
                }];
            }
        }
    }
    vec![
        MachineEvent::Inventory {
            components: inventory,
            workspaces: None,
            workspace_revision: None,
            observed_at_ms: unix_ms(),
        },
        MachineEvent::CommandResult {
            request_id,
            accepted: true,
            detail: None,
        },
    ]
}

async fn run_login(
    request_id: String,
    provider: String,
    auth_method: Option<String>,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    components: Arc<ComponentStore>,
    zed_adapter_socket: Option<PathBuf>,
    mut login: LoginIo,
) {
    if provider == "gemini" && auth_method.as_deref() == Some("api_key") {
        run_gemini_api_key_login(
            request_id,
            provider,
            events,
            components,
            zed_adapter_socket,
            login,
        )
        .await;
        return;
    }
    let (command, args): (&str, &[&str]) = match provider.as_str() {
        "codex" => ("codex", &["login", "--device-auth"]),
        "claude" => ("claude", &["auth", "login"]),
        "gemini" => ("script", &[]),
        _ => {
            login.sessions.lock().remove(&request_id);
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted: false,
                detail: Some(format!(
                    "{provider} currently requires terminal-assisted login"
                )),
            });
            return;
        }
    };
    if provider == "gemini" && !gemini_code_assist_project_configured() {
        login.sessions.lock().remove(&request_id);
        let _ = events.send(MachineEvent::CommandResult {
            request_id,
            accepted: false,
            detail: Some(GEMINI_CONSUMER_LOGIN_RETIRED.to_owned()),
        });
        return;
    }
    if provider == "gemini"
        && let Err(error) = select_gemini_code_assist_oauth()
    {
        login.sessions.lock().remove(&request_id);
        let _ = events.send(MachineEvent::CommandResult {
            request_id,
            accepted: false,
            detail: Some(format!(
                "preparing Gemini Code Assist login failed: {error:#}"
            )),
        });
        return;
    }
    let mut process = tokio::process::Command::new(command);
    if provider == "gemini" {
        // Gemini deliberately rejects manual OAuth without a terminal. Give
        // only the login process a private PTY; Cowboy still owns its stdin and
        // parses the public authorization URL from stdout.
        process.args([
            "-q",
            "-e",
            "-f",
            "-c",
            "env NO_BROWSER=true TERM=dumb gemini",
            "/dev/null",
        ]);
    } else {
        process.args(args);
    }
    let mut child = match process
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            login.sessions.lock().remove(&request_id);
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted: false,
                detail: Some(error.to_string()),
            });
            return;
        }
    };
    let mut stdin = child.stdin.take().expect("piped stdin");
    let _ = events.send(MachineEvent::LoginState {
        request_id: request_id.clone(),
        provider: provider.clone(),
        state: AuthState::Pending,
        account_label: None,
        detail: Some("starting browser authorization".to_owned()),
    });
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let mut stdout = tokio::io::BufReader::new(stdout).lines();
    let mut stderr = tokio::io::BufReader::new(stderr).lines();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut verification_url = None;
    let mut user_code = None;
    let mut challenge_sent = false;
    let mut login_completed = false;
    loop {
        let line = tokio::select! {
            line = stdout.next_line(), if stdout_open => {
                if matches!(line, Ok(None)) { stdout_open = false; }
                line
            },
            line = stderr.next_line(), if stderr_open => {
                if matches!(line, Ok(None)) { stderr_open = false; }
                line
            },
            changed = login.cancel.changed() => {
                if changed.is_ok() && *login.cancel.borrow() {
                    let _ = child.kill().await;
                    login.sessions.lock().remove(&request_id);
                    let _ = events.send(MachineEvent::LoginState {
                        request_id,
                        provider,
                        state: AuthState::SignedOut,
                        account_label: None,
                        detail: Some("login cancelled".to_owned()),
                    });
                    return;
                }
                continue;
            },
            code = login.input.recv() => {
                if let Some(code) = code
                    && (stdin.write_all(code.as_bytes()).await.is_err()
                        || stdin.write_all(b"\n").await.is_err())
                {
                    let _ = child.kill().await;
                }
                continue;
            }
        };
        let Ok(Some(line)) = line else {
            if !stdout_open && !stderr_open {
                break;
            }
            continue;
        };
        if provider == "gemini"
            && challenge_sent
            && probe_gemini_auth().state == AuthState::SignedIn
        {
            login_completed = true;
            let _ = child.kill().await;
            break;
        }
        for word in line.split_whitespace() {
            let trimmed = word.trim_matches(|character: char| {
                matches!(character, '(' | ')' | '[' | ']' | ',' | ':' | ';')
            });
            if trimmed.starts_with("https://") {
                verification_url = Some(trimmed.to_owned());
            } else if trimmed.len() >= 6
                && trimmed
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
                && trimmed.contains('-')
            {
                user_code = Some(trimmed.to_owned());
            }
        }
        let challenge_ready = provider != "codex" || user_code.is_some();
        if !challenge_sent
            && challenge_ready
            && let Some(url) = verification_url.clone()
        {
            let _ = events.send(MachineEvent::LoginChallenge {
                request_id: request_id.clone(),
                provider: provider.clone(),
                verification_url: url,
                user_code: user_code.clone(),
                input_required: provider != "codex",
                input_label: None,
                secret_input: false,
                expires_at_ms: unix_ms().saturating_add(15 * 60 * 1_000),
            });
            challenge_sent = true;
        }
    }
    let status = child.wait().await;
    login.sessions.lock().remove(&request_id);
    let signed_in = login_completed || status.is_ok_and(|status| status.success());
    let _ = events.send(MachineEvent::LoginState {
        request_id: request_id.clone(),
        provider,
        state: if signed_in {
            AuthState::SignedIn
        } else {
            AuthState::Error
        },
        account_label: None,
        detail: (!signed_in).then_some("provider login did not complete".to_owned()),
    });
    if signed_in {
        let inventory = collect_inventory(&components, zed_adapter_socket.as_deref()).await;
        let _ = events.send(MachineEvent::Inventory {
            components: inventory,
            workspaces: None,
            workspace_revision: None,
            observed_at_ms: unix_ms(),
        });
    }
}

async fn run_gemini_api_key_login(
    request_id: String,
    provider: String,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    components: Arc<ComponentStore>,
    zed_adapter_socket: Option<PathBuf>,
    mut login: LoginIo,
) {
    let _ = events.send(MachineEvent::LoginState {
        request_id: request_id.clone(),
        provider: provider.clone(),
        state: AuthState::Pending,
        account_label: None,
        detail: Some("waiting for a Gemini API key".to_owned()),
    });
    let _ = events.send(MachineEvent::LoginChallenge {
        request_id: request_id.clone(),
        provider: provider.clone(),
        verification_url: "https://aistudio.google.com/apikey".to_owned(),
        user_code: None,
        input_required: true,
        input_label: Some("Gemini API key".to_owned()),
        secret_input: true,
        expires_at_ms: unix_ms().saturating_add(15 * 60 * 1_000),
    });
    let result = tokio::select! {
        changed = login.cancel.changed() => {
            if changed.is_ok() && *login.cancel.borrow() { Err(anyhow::anyhow!("login cancelled")) }
            else { Err(anyhow::anyhow!("login interrupted")) }
        },
        input = login.input.recv() => match input {
            Some(api_key) => store_gemini_api_key(&api_key),
            None => Err(anyhow::anyhow!("login input closed")),
        },
    };
    login.sessions.lock().remove(&request_id);
    let signed_in = result.is_ok();
    let _ = events.send(MachineEvent::LoginState {
        request_id: request_id.clone(),
        provider,
        state: if signed_in {
            AuthState::SignedIn
        } else {
            AuthState::Error
        },
        account_label: None,
        detail: result.err().map(|error| error.to_string()),
    });
    if signed_in {
        let inventory = collect_inventory(&components, zed_adapter_socket.as_deref()).await;
        let _ = events.send(MachineEvent::Inventory {
            components: inventory,
            workspaces: None,
            workspace_revision: None,
            observed_at_ms: unix_ms(),
        });
    }
}

fn store_gemini_api_key(api_key: &str) -> anyhow::Result<()> {
    use std::io::Write as _;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt as _;

    let api_key = api_key.trim();
    anyhow::ensure!(!api_key.is_empty(), "Gemini API key is empty");
    anyhow::ensure!(
        !api_key.contains(['\r', '\n']),
        "Gemini API key contains a line break"
    );
    let home = std::env::var_os("HOME").context("HOME is not configured")?;
    let root = PathBuf::from(home).join(".gemini");
    std::fs::create_dir_all(&root).context("creating Gemini state directory")?;
    let env_path = root.join(".env");
    let existing = std::fs::read_to_string(&env_path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| {
            let candidate = line.trim().strip_prefix("export ").unwrap_or(line.trim());
            !candidate.starts_with("GEMINI_API_KEY=")
        })
        .map(str::to_owned)
        .collect();
    lines.push(format!("GEMINI_API_KEY={api_key}"));
    let temporary = root.join(".env.cowboy-login");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).context("staging Gemini API key")?;
    writeln!(file, "{}", lines.join("\n")).context("writing Gemini API key")?;
    file.sync_all().context("syncing Gemini API key")?;
    std::fs::rename(&temporary, &env_path).context("activating Gemini API key")?;
    select_gemini_auth_type("gemini-api-key")
}

fn gemini_code_assist_project_configured() -> bool {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .is_some_and(|home| gemini_env_configured(&home.join(".gemini"), "GOOGLE_CLOUD_PROJECT"))
}

fn select_gemini_code_assist_oauth() -> anyhow::Result<()> {
    select_gemini_auth_type("oauth-personal")
}

fn select_gemini_auth_type(selected_type: &str) -> anyhow::Result<()> {
    let home = std::env::var_os("HOME").context("HOME is not configured")?;
    let root = PathBuf::from(home).join(".gemini");
    std::fs::create_dir_all(&root).context("creating Gemini state directory")?;
    let settings_path = root.join("settings.json");
    let mut settings = std::fs::read(&settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let object = settings
        .as_object_mut()
        .context("Gemini settings must be a JSON object")?;
    let security = object
        .entry("security")
        .or_insert_with(|| serde_json::json!({}));
    let security = security
        .as_object_mut()
        .context("Gemini security settings must be a JSON object")?;
    let auth = security
        .entry("auth")
        .or_insert_with(|| serde_json::json!({}));
    let auth = auth
        .as_object_mut()
        .context("Gemini auth settings must be a JSON object")?;
    auth.insert(
        "selectedType".to_owned(),
        serde_json::Value::String(selected_type.to_owned()),
    );
    let temporary = root.join("settings.json.cowboy-login");
    std::fs::write(&temporary, serde_json::to_vec_pretty(&settings)?)
        .context("writing staged Gemini settings")?;
    std::fs::rename(&temporary, &settings_path).context("activating Gemini OAuth settings")?;
    Ok(())
}

fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Linux
    }
}

fn load_workspace_snapshot(path: &Path, fallback: &[String]) -> anyhow::Result<WorkspaceSnapshot> {
    let file = match std::fs::read(path) {
        Ok(contents) => Some(
            serde_json::from_slice::<WorkspaceFile>(&contents)
                .with_context(|| format!("parsing workspace configuration {}", path.display()))?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading workspace configuration {}", path.display()));
        }
    };
    let (revision, values) = if let Some(file) = file {
        if file.version != 1 {
            bail!(
                "workspace configuration {} uses unsupported version {}",
                path.display(),
                file.version
            );
        }
        if file.revision.trim().is_empty() {
            bail!(
                "workspace configuration {} has an empty revision",
                path.display()
            );
        }
        (Some(file.revision), file.workspaces)
    } else {
        (None, fallback.to_vec())
    };
    Ok(WorkspaceSnapshot {
        revision,
        workspaces: parse_workspaces(&values)?,
    })
}

fn parse_workspaces(values: &[String]) -> anyhow::Result<Vec<MachineWorkspace>> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let (id, path) = value
            .split_once('=')
            .with_context(|| format!("workspace {value:?} must use id=/absolute/path"))?;
        if id.trim().is_empty() || id.contains('/') {
            bail!("workspace id {id:?} is invalid");
        }
        let (canonical, available) =
            crate::workspace_roots::resolve_configured_root(Path::new(path))
                .with_context(|| format!("canonicalizing workspace {id:?} at {path:?}"))?;
        if available && !canonical.is_dir() {
            bail!("workspace {id:?} is not a directory");
        }
        if !available {
            tracing::warn!(
                workspace_id = id,
                workspace_path = %canonical.display(),
                "configured workspace is pending synchronization on this Machine"
            );
        }
        out.push(MachineWorkspace {
            id: id.to_owned(),
            display_name: id.to_owned(),
            canonical_path: canonical.display().to_string(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

fn reject_untrusted_workspace(
    frame: &crate::runtime_wire::Frame,
    workspaces: &[MachineWorkspace],
    worktree_root: &Path,
) -> Option<crate::runtime_wire::Frame> {
    let crate::runtime_wire::Frame::CoreCommand {
        command: crate::runtime_wire::CoreCommand::EnsureSession { session },
    } = frame
    else {
        return None;
    };
    let allowed = std::fs::canonicalize(&session.cwd)
        .ok()
        .is_some_and(|target| workspace_path_allowed(&target, workspaces, worktree_root));
    (!allowed).then(|| crate::runtime_wire::Frame::CommandAck {
        session_id: session.session_id.clone(),
        command_id: format!("ensure:{}", session.session_id),
        accepted: false,
        reason: Some(format!(
            "session workspace is outside this Machine's trusted roots: {}",
            session.cwd
        )),
    })
}

fn workspace_path_allowed(
    target: &Path,
    workspaces: &[MachineWorkspace],
    worktree_root: &Path,
) -> bool {
    workspaces.iter().any(|workspace| {
        crate::workspace_roots::canonical_target_within_root(
            target,
            Path::new(&workspace.canonical_path),
        )
    }) || worktree_root
        .canonicalize()
        .is_ok_and(|root| target.starts_with(root))
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| value.as_millis().try_into().unwrap_or(i64::MAX))
}

async fn send_frame<S>(socket: &mut S, frame: &MachineFrame) -> anyhow::Result<()>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    send_frame_with_timeout(socket, frame, MACHINE_FRAME_SEND_TIMEOUT).await
}

async fn send_frame_with_timeout<S>(
    socket: &mut S,
    frame: &MachineFrame,
    timeout: Duration,
) -> anyhow::Result<()>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let text = serde_json::to_string(frame).context("serializing Machine frame")?;
    tokio::time::timeout(timeout, socket.send(Message::Text(text.into())))
        .await
        .context("Machine frame send timed out")??;
    Ok(())
}

async fn receive_frame<S>(socket: &mut S) -> anyhow::Result<MachineFrame>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).context("parsing Machine frame");
            }
            Some(Ok(Message::Close(_))) | None => bail!("Machine WebSocket closed"),
            Some(Err(error)) => return Err(error).context("reading Machine WebSocket"),
            Some(Ok(_)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use clap::Parser as _;

    use super::{
        Args, WorkspaceConfig, bootstrap_acp_inventory, claude_runtime_enabled,
        disabled_provider_slots_from, gemini_auth_from_metadata, gemini_env_value_from,
        load_workspace_snapshot, managed_provider_environment, npm_package_for_component,
        npm_update_is_confirmed_by_inventory, parse_workspaces, provider_for_component,
        reject_untrusted_workspace, selected_zed_pair, send_frame_with_timeout,
        validate_controller_url, workspace_path_allowed,
    };
    use crate::machine_components::ComponentStore;
    use crate::machine_protocol::{
        ArtifactFormat, ComponentId, ComponentInventory, ComponentKind, ComponentState,
        ComponentUpdate, DesiredComponent, MachineFrame,
    };
    use crate::runtime_wire::{CoreCommand, Frame, StartSession};

    #[test]
    fn remote_controller_requires_https() {
        assert!(validate_controller_url("https://cowboy.example").is_ok());
        assert!(validate_controller_url("http://cowboy.example").is_err());
    }

    #[test]
    fn loopback_controller_allows_plaintext_for_hermetic_tests() {
        assert!(validate_controller_url("http://127.0.0.1:43333").is_ok());
        assert!(validate_controller_url("http://localhost:43333").is_ok());
    }

    #[test]
    fn provider_usage_status_does_not_require_a_controller() {
        let args = Args::try_parse_from([
            "cowboy-machine",
            "--provider-usage-status",
            "--state-dir",
            "/tmp/cowboy-machine-status-test",
        ])
        .expect("parse read-only status command");
        assert!(args.provider_usage_status);
    }

    #[tokio::test]
    async fn stalled_controller_write_times_out_instead_of_fencing_all_workers() {
        let sink = futures::sink::unfold((), |(), _message| async move {
            std::future::pending::<Result<(), std::io::Error>>().await
        });
        futures::pin_mut!(sink);
        let error = send_frame_with_timeout(
            &mut sink,
            &MachineFrame::Heartbeat { sent_at_ms: 1 },
            Duration::from_millis(10),
        )
        .await
        .expect_err("stalled write must fail closed");
        assert!(error.to_string().contains("Machine frame send timed out"));
    }

    #[test]
    fn gemini_auth_rejects_retired_consumer_oauth() {
        assert_eq!(
            gemini_auth_from_metadata(Some("oauth-personal"), true, false, false, false, false)
                .state,
            crate::machine_protocol::AuthState::SignedOut
        );
        assert_eq!(
            gemini_auth_from_metadata(Some("oauth-personal"), true, false, false, false, true)
                .state,
            crate::machine_protocol::AuthState::SignedIn
        );
        assert_eq!(
            gemini_auth_from_metadata(Some("gemini-api-key"), false, true, false, false, false)
                .state,
            crate::machine_protocol::AuthState::SignedIn
        );
        assert_eq!(
            gemini_auth_from_metadata(None, false, false, false, false, false).state,
            crate::machine_protocol::AuthState::SignedOut
        );
    }

    #[test]
    fn gemini_env_reader_accepts_exported_values_without_exposing_other_keys() {
        let contents = "IGNORED=value\nexport GOOGLE_CLOUD_PROJECT='enterprise-project'\n";
        assert_eq!(
            gemini_env_value_from(contents, "GOOGLE_CLOUD_PROJECT").as_deref(),
            Some("enterprise-project")
        );
        assert_eq!(gemini_env_value_from(contents, "GEMINI_API_KEY"), None);
    }

    #[test]
    fn disabled_provider_aliases_are_normalized() {
        assert_eq!(
            disabled_provider_slots_from("claude-code, claude-deepseek, gemini, codex-deepseek"),
            ["claude", "claude-deepseek", "gemini", "codex-deepseek"]
        );
    }

    #[test]
    fn either_claude_lane_keeps_the_shared_runtime_enabled() {
        assert!(claude_runtime_enabled(&["claude".to_owned()]));
        assert!(claude_runtime_enabled(&["claude-deepseek".to_owned()]));
        assert!(!claude_runtime_enabled(&[
            "claude".to_owned(),
            "claude-deepseek".to_owned(),
        ]));
    }

    #[test]
    fn bootstrap_inventory_reports_effective_worker_generation() {
        let inventory = bootstrap_acp_inventory("worker-content-address");
        assert_eq!(inventory.id.kind, ComponentKind::AcpRuntime);
        assert_eq!(inventory.generation, "worker-content-address");
        assert_eq!(inventory.detail.as_deref(), Some("bootstrap generation"));
    }

    #[test]
    fn npm_updates_are_confined_to_known_component_ids() {
        assert_eq!(
            npm_package_for_component(&ComponentId {
                kind: ComponentKind::ProviderAdapter,
                slot: "codex".to_owned(),
            }),
            Some("@agentclientprotocol/codex-acp")
        );
        assert_eq!(
            npm_package_for_component(&ComponentId {
                kind: ComponentKind::ProviderAdapter,
                slot: "arbitrary-package".to_owned(),
            }),
            None
        );
        assert_eq!(
            provider_for_component(&ComponentId {
                kind: ComponentKind::ProviderCli,
                slot: "claude".to_owned(),
            }),
            Some("claude-code")
        );
    }

    #[test]
    fn npm_update_error_is_recovered_only_by_authoritative_latest_inventory() {
        let id = ComponentId {
            kind: ComponentKind::ProviderCli,
            slot: "claude".to_owned(),
        };
        let current = ComponentInventory {
            id: id.clone(),
            state: ComponentState::Active,
            version: "2.1.226".to_owned(),
            generation: String::new(),
            digest: String::new(),
            rollback_generation: None,
            active_leases: 0,
            auth: None,
            detail: None,
            update: Some(ComponentUpdate {
                latest_version: "2.1.226".to_owned(),
                available: false,
                source: "npm registry".to_owned(),
                checked_at_ms: 1,
                installable: true,
            }),
        };
        assert!(npm_update_is_confirmed_by_inventory(
            &id,
            std::slice::from_ref(&current)
        ));

        let mut outdated = current.clone();
        outdated.update.as_mut().expect("release status").available = true;
        assert!(!npm_update_is_confirmed_by_inventory(
            &id,
            std::slice::from_ref(&outdated)
        ));

        let mut untrusted = current;
        untrusted.update.as_mut().expect("release status").source = "cached inventory".to_owned();
        assert!(!npm_update_is_confirmed_by_inventory(
            &id,
            std::slice::from_ref(&untrusted)
        ));
    }

    #[test]
    fn zed_supervisor_selects_only_an_exact_compatibility_pair() {
        let component = |kind, slot: &str| DesiredComponent {
            id: ComponentId {
                kind,
                slot: slot.to_owned(),
            },
            version: slot.to_owned(),
            generation: slot.to_owned(),
            artifact_url: "https://example.invalid/zed".to_owned(),
            digest: "digest".to_owned(),
            artifact_format: ArtifactFormat::Raw,
            entrypoint: None,
            signature: None,
            probe: None,
            automatic: true,
        };
        let mismatched = vec![
            (
                component(ComponentKind::ZedAdapter, "1.2.0"),
                PathBuf::from("adapter"),
            ),
            (
                component(ComponentKind::ZedServer, "1.1.0"),
                PathBuf::from("server"),
            ),
        ];
        assert!(selected_zed_pair(&mismatched).is_none());
        let mut matched = mismatched;
        matched.push((
            component(ComponentKind::ZedServer, "1.2.0"),
            PathBuf::from("server-1.2"),
        ));
        assert_eq!(
            selected_zed_pair(&matched),
            Some((
                "1.2.0".to_owned(),
                PathBuf::from("adapter"),
                PathBuf::from("server-1.2")
            ))
        );
    }

    #[test]
    fn managed_components_define_worker_provider_commands() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-machine-provider-env-{}",
            std::process::id()
        ));
        let store = ComponentStore::new(root.clone(), None, "worker-test-bootstrap".to_owned())
            .expect("component store");
        let component = |kind, slot: &str| DesiredComponent {
            id: ComponentId {
                kind,
                slot: slot.to_owned(),
            },
            version: "v1".to_owned(),
            generation: "v1".to_owned(),
            artifact_url: "https://example.invalid/provider".to_owned(),
            digest: "digest".to_owned(),
            artifact_format: ArtifactFormat::Raw,
            entrypoint: None,
            signature: None,
            probe: None,
            automatic: true,
        };
        for (name, desired) in [
            (
                "provider_adapter-codex",
                component(ComponentKind::ProviderAdapter, "codex"),
            ),
            (
                "provider_adapter-claude",
                component(ComponentKind::ProviderAdapter, "claude"),
            ),
            (
                "provider_cli-claude",
                component(ComponentKind::ProviderCli, "claude"),
            ),
            (
                "provider_cli-gemini",
                component(ComponentKind::ProviderCli, "gemini"),
            ),
        ] {
            let generation = root.join("test-generations").join(name);
            std::fs::create_dir_all(&generation).expect("generation");
            std::fs::write(
                generation.join("manifest.json"),
                serde_json::to_vec(&desired).expect("manifest"),
            )
            .expect("manifest file");
            std::fs::write(generation.join("bin"), b"test").expect("executable");
            std::os::unix::fs::symlink(&generation, root.join("active").join(name))
                .expect("active link");
        }
        let worker = root.join("release/bin/cowboy-acp-worker");
        let proxy = worker.with_file_name("cowboy-codex-app-server");
        std::fs::create_dir_all(proxy.parent().expect("proxy parent")).expect("proxy directory");
        std::fs::write(&proxy, b"test").expect("proxy executable");
        let environment =
            managed_provider_environment(&store, &worker).expect("provider environment");
        assert!(environment["COWBOY_ACP_CODEX_CMD"].ends_with("commands/codex-acp"));
        assert!(environment["COWBOY_ACP_CLAUDE_CODE_CMD"].ends_with("commands/claude-agent-acp"));
        assert!(environment["COWBOY_ACP_CLAUDE_CODE_EXECUTABLE"].ends_with("commands/claude"));
        assert!(matches!(
            std::path::Path::new(&environment["COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL"])
                .file_name()
                .and_then(std::ffi::OsStr::to_str),
            Some("bash" | "zsh")
        ));
        assert!(environment["COWBOY_ACP_GEMINI_CMD"].ends_with("commands/gemini"));
        assert_eq!(environment["COWBOY_ACP_GEMINI_ARGS"], "--acp");
        assert_eq!(environment["CODEX_PATH"], proxy.display().to_string());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn remote_launch_is_confined_to_canonical_workspace() {
        let root =
            std::env::temp_dir().join(format!("cowboy-machine-workspace-{}", std::process::id()));
        let nested = root.join("nested");
        let managed = root.join("managed");
        let managed_session = managed.join("sess-1");
        std::fs::create_dir_all(&nested).expect("workspace");
        std::fs::create_dir_all(&managed_session).expect("managed workspace");
        let workspaces = parse_workspaces(&[format!("main={}", root.display())]).expect("parse");
        let ensure = |cwd: String| Frame::CoreCommand {
            command: CoreCommand::EnsureSession {
                session: StartSession {
                    session_id: "session".to_owned(),
                    provider: "codex".to_owned(),
                    cwd,
                    agent_session_id: None,
                    system: false,
                    context_window: None,
                    auto_compact_token_limit: None,
                    cache_protection: None,
                    generation: "test".to_owned(),
                    fallback_for: None,
                    adopt_only: false,
                },
            },
        };
        assert!(
            reject_untrusted_workspace(
                &ensure(nested.display().to_string()),
                &workspaces,
                &managed,
            )
            .is_none()
        );
        assert!(
            reject_untrusted_workspace(
                &ensure(managed_session.display().to_string()),
                &[],
                &managed,
            )
            .is_none()
        );
        assert!(
            reject_untrusted_workspace(
                &ensure("/definitely/not/a/workspace".to_owned()),
                &workspaces,
                &managed,
            )
            .is_some()
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn pending_workspace_becomes_usable_without_restarting_the_machine() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-machine-partial-workspaces-{}",
            std::process::id()
        ));
        let available = root.join("available");
        std::fs::create_dir_all(&available).expect("available workspace");

        let workspaces = parse_workspaces(&[
            format!("available={}", available.display()),
            format!("awaiting-sync={}", root.join("missing").display()),
        ])
        .expect("pending workspace should remain configured");
        assert_eq!(workspaces.len(), 2);
        let available_workspace = workspaces
            .iter()
            .find(|workspace| workspace.id == "available")
            .unwrap();
        assert_eq!(
            available_workspace.canonical_path,
            available.canonicalize().unwrap().display().to_string()
        );
        let pending_workspace = workspaces
            .iter()
            .find(|workspace| workspace.id == "awaiting-sync")
            .unwrap();
        assert_eq!(
            pending_workspace.canonical_path,
            root.canonicalize()
                .unwrap()
                .join("missing")
                .display()
                .to_string()
        );

        let synchronized = root.join("missing");
        std::fs::create_dir(&synchronized).expect("synchronize pending workspace");
        let synchronized = synchronized.canonicalize().unwrap();
        assert!(workspace_path_allowed(
            &synchronized,
            &workspaces,
            &root.join("unrelated-managed-root")
        ));
        assert!(parse_workspaces(&["relative=missing".to_owned()]).is_err());
        assert!(parse_workspaces(&["parent=/tmp/../pending".to_owned()]).is_err());

        let file = root.join("not-a-directory");
        std::fs::write(&file, "not a workspace").expect("workspace-shaped file");
        assert!(parse_workspaces(&[format!("file={}", file.display())]).is_err());

        #[cfg(unix)]
        {
            let pending_link = root.join("pending-link");
            let pending = parse_workspaces(&[format!("link={}", pending_link.display())])
                .expect("pending link path");
            let outside = root.with_extension("outside");
            std::fs::create_dir(&outside).expect("outside workspace");
            std::os::unix::fs::symlink(&outside, &pending_link).expect("late workspace symlink");
            assert!(!workspace_path_allowed(
                &outside.canonicalize().unwrap(),
                &pending,
                &root.join("unrelated-managed-root")
            ));
            std::fs::remove_dir_all(outside).expect("cleanup outside workspace");
        }

        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn missing_workspace_file_uses_cli_compatibility_values() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-machine-workspace-fallback-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("workspace");
        let snapshot = load_workspace_snapshot(
            &root.join("missing.json"),
            &[format!("fallback={}", root.display())],
        )
        .expect("fallback workspace");
        assert_eq!(snapshot.revision, None);
        assert_eq!(snapshot.workspaces[0].id, "fallback");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn declarative_workspace_config_reloads_atomically() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-machine-workspace-reload-{}",
            std::process::id()
        ));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).expect("first workspace");
        std::fs::create_dir_all(&second).expect("second workspace");
        let path = root.join("workspaces.json");
        let write_config = |version: u16, revision: &str, workspace: &std::path::Path| {
            std::fs::write(
                &path,
                serde_json::to_vec(&serde_json::json!({
                    "version": version,
                    "revision": revision,
                    "workspaces": [format!("project={}", workspace.display())]
                }))
                .expect("serialize workspace config"),
            )
            .expect("write workspace config");
        };
        write_config(1, "revision-one", &first);
        let config = WorkspaceConfig::new(path.clone(), Vec::new()).expect("initial config");
        let updates = config.subscribe();
        assert_eq!(
            config.snapshot().workspaces[0].canonical_path,
            first.canonicalize().unwrap().display().to_string()
        );

        write_config(1, "revision-two", &second);
        let reloaded = config.reload().expect("reload config");
        assert_eq!(reloaded.revision.as_deref(), Some("revision-two"));
        assert_eq!(
            reloaded.workspaces[0].canonical_path,
            second.canonicalize().unwrap().display().to_string()
        );
        assert!(updates.has_changed().expect("workspace update channel"));

        write_config(2, "unsupported", &first);
        assert!(config.reload().is_err());
        assert_eq!(config.snapshot().revision.as_deref(), Some("revision-two"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}

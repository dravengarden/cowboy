#![warn(clippy::pedantic)]

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cowboy",
    version,
    about = "Drive coding-agent CLIs from anywhere over ACP"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the cowboy daemon (HTTP + WebSocket). The long-running systemd
    /// service that owns the Hub + supervisor; every surface (Web UI, phone,
    /// native shell) connects to it as a client.
    Serve(ServeArgs),
    /// Expose the running cowboy daemon as a stdio ACP agent for Zed or any
    /// other ACP client. This is a thin bridge; it never starts a second Hub.
    ServeAcp(ServeAcpArgs),
    /// Debug: drive one provider end-to-end (spawn, initialize, prompt, stream).
    TryAgent(TryAgentArgs),
    /// Create a short-lived, single-use token for enrolling a remote Machine.
    MachineEnroll(MachineEnrollArgs),
    /// Revoke a remote Machine identity. Re-enrollment requires a new token
    /// and creates an explicit key-rotation boundary.
    MachineRevoke(MachineRevokeArgs),
}

#[derive(Args)]
pub struct MachineEnrollArgs {
    #[arg(long, env = "COWBOY_POSTGRES_URL")]
    postgres_url: String,
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    data_dir: PathBuf,
    #[arg(long)]
    machine_id: String,
    #[arg(long)]
    display_name: String,
    #[arg(long, default_value_t = 600)]
    ttl_seconds: i64,
}

#[derive(Args)]
pub struct MachineRevokeArgs {
    #[arg(long, env = "COWBOY_POSTGRES_URL")]
    postgres_url: String,
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    data_dir: PathBuf,
    #[arg(long)]
    machine_id: String,
}

#[derive(Args)]
pub struct ServeAcpArgs {
    /// Only expose sessions for this provider. Register one Zed External Agent
    /// entry per provider to preserve the provider identity and logo.
    #[arg(long, default_value = "codex")]
    pub provider: String,

    /// Base URL of the already-running cowboy daemon.
    #[arg(
        long,
        env = "COWBOY_DAEMON_URL",
        default_value = "http://127.0.0.1:3333"
    )]
    pub daemon_url: String,
}

#[derive(Args)]
pub struct TryAgentArgs {
    /// Provider id: `claude-code` | `claude-deepseek` | `codex` | `codex-deepseek` | `gemini`.
    #[arg(long)]
    provider: String,
    /// Working directory for the session.
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    /// The prompt to send.
    prompt: String,
}

#[derive(Args)]
pub struct ServeArgs {
    /// Address to bind the HTTP/WebSocket server to.
    #[arg(long, env = "COWBOY_BIND", default_value = "127.0.0.1:3333")]
    pub bind: SocketAddr,

    /// Root directory agents are allowed to operate within.
    #[arg(long, env = "COWBOY_WORKSPACE_ROOT", default_value = ".")]
    pub workspace_root: PathBuf,

    /// State/data directory (config + secrets; transcript log persists in
    /// `--postgres-url` when set).
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    pub data_dir: PathBuf,

    /// Built SPA directory. Static assets are loaded at request time instead
    /// of being embedded in the Rust binary, so a frontend-only rollout does
    /// not restart the API or any agent session.
    #[arg(long, env = "COWBOY_WEB_ROOT", default_value = "web/dist")]
    pub web_root: PathBuf,

    /// Unix socket of Cowboy's isolated Zed protocol adapter. When omitted,
    /// code review remains available but language intelligence is reported as
    /// unavailable.
    #[arg(long, env = "COWBOY_ZED_ADAPTER_SOCKET")]
    pub zed_adapter_socket: Option<PathBuf>,

    /// Hawk-local saved-file content cache quota. Set to zero to disable.
    #[arg(
        long,
        env = "COWBOY_CODE_CACHE_BYTES",
        default_value_t = 2 * 1024 * 1024 * 1024_u64
    )]
    pub code_cache_bytes: u64,

    /// Controller-owned signed desired-component manifest sent to every
    /// authenticated Machine. The browser can request reconciliation but
    /// cannot supply artifact URLs, hashes, or signatures.
    #[arg(long, env = "COWBOY_MACHINE_COMPONENTS_MANIFEST")]
    pub machine_components_manifest: Option<PathBuf>,

    /// `PostgreSQL` connection URL for persistent sessions + events. When
    /// absent the daemon runs in pure in-memory mode (v0 fallback, no
    /// restart recovery). The hawk-provisioned cowboy-private cluster
    /// listens on 127.0.0.1:5433 with DB `cowboy` and trust-auth role
    /// `cowboy`; that's the production URL.
    #[arg(long, env = "COWBOY_POSTGRES_URL")]
    pub postgres_url: Option<String>,

    /// `VictoriaLogs` base URL used by the bounded client observability relay.
    #[arg(
        long,
        env = "COWBOY_VICTORIA_LOGS_URL",
        default_value = "http://127.0.0.1:6302"
    )]
    pub victoria_logs_url: String,

    /// `VictoriaMetrics` base URL used by the bounded client observability relay.
    #[arg(
        long,
        env = "COWBOY_VICTORIA_METRICS_URL",
        default_value = "http://127.0.0.1:6301"
    )]
    pub victoria_metrics_url: String,

    /// Codex CLI used by the shared Luna classifier app-server. The self-managed
    /// `/opt` install follows Codex's own update channel on hawk.
    #[arg(
        long,
        env = "COWBOY_CODEX_COMMAND",
        default_value = "/opt/npm-global/bin/codex"
    )]
    pub codex_command: String,
}

impl Cli {
    /// Runs the selected Cowboy command until it exits.
    ///
    /// # Errors
    ///
    /// Returns an error when command validation, startup, or the selected
    /// long-running service fails.
    pub async fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::Serve(args) => crate::server::serve(args).await,
            Command::ServeAcp(args) => {
                crate::server::init_tracing();
                crate::acp_bridge::serve(args).await
            }
            Command::TryAgent(args) => {
                crate::server::init_tracing();
                let spec = crate::provider::lookup(&args.provider)
                    .ok_or_else(|| anyhow::anyhow!("unknown provider {:?}", args.provider))?;
                let local = tokio::task::LocalSet::new();
                local
                    .run_until(crate::acp::run_oneshot(&spec, args.cwd, args.prompt))
                    .await
            }
            Command::MachineEnroll(args) => {
                let store = crate::store::Store::connect(
                    &args.postgres_url,
                    args.data_dir.join("artifacts"),
                )
                .await?;
                store.migrate().await?;
                let token = store
                    .create_machine_enrollment(
                        &args.machine_id,
                        &args.display_name,
                        args.ttl_seconds,
                    )
                    .await?;
                println!("{token}");
                Ok(())
            }
            Command::MachineRevoke(args) => {
                let store = crate::store::Store::connect(
                    &args.postgres_url,
                    args.data_dir.join("artifacts"),
                )
                .await?;
                store.migrate().await?;
                store.revoke_machine(&args.machine_id).await
            }
        }
    }
}

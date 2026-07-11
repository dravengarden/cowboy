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
    /// Provider id: `claude-code` | `codex`.
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

    /// `PostgreSQL` connection URL for persistent sessions + events. When
    /// absent the daemon runs in pure in-memory mode (v0 fallback, no
    /// restart recovery). The hawk-provisioned cowboy-private cluster
    /// listens on 127.0.0.1:5433 with DB `cowboy` and trust-auth role
    /// `cowboy`; that's the production URL.
    #[arg(long, env = "COWBOY_POSTGRES_URL")]
    pub postgres_url: Option<String>,

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
        }
    }
}

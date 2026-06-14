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
    /// Debug: drive one provider end-to-end (spawn, initialize, prompt, stream).
    TryAgent(TryAgentArgs),
    /// Memory write path. Reads are plain `rg`/`cat` over the store (see the
    /// `memory` skill); `mem` is the validated WRITE: it enqueues a proposal to
    /// the running daemon's queue → the janitor dedups/judges/commits.
    Mem(MemArgs),
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

/// `cowboy mem …` — the validated memory WRITE path (reads are `rg`/`cat`).
#[derive(Args)]
pub struct MemArgs {
    #[command(subcommand)]
    pub command: MemCommand,
}

#[derive(Subcommand)]
pub enum MemCommand {
    /// Record a memory candidate: validate, then enqueue a PROPOSAL to the
    /// daemon (the janitor dedups/judges/commits — this never writes a file).
    Record(MemRecordArgs),
    /// Soft-archive a memory by name (move to archive/; never hard-delete).
    Forget {
        /// Memory name (the `.md` stem).
        name: String,
    },
}

#[derive(Args)]
pub struct MemRecordArgs {
    /// Memory name (kebab-case; the `.md` stem).
    #[arg(long)]
    pub name: String,
    /// One-line description — the recall hook.
    #[arg(long)]
    pub description: String,
    /// Memory type: user | feedback | project | reference.
    #[arg(long = "type", default_value = "reference")]
    pub mem_type: String,
    /// Target tier (a project cwd-slug); omit for the machine tier.
    #[arg(long)]
    pub tier: Option<String>,
    /// The memory body (everything after `--`).
    #[arg(trailing_var_arg = true)]
    pub body: Vec<String>,
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
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        match self.command {
            Command::Serve(args) => crate::server::serve(args).await,
            Command::TryAgent(args) => {
                crate::server::init_tracing();
                let spec = crate::provider::lookup(&args.provider)
                    .ok_or_else(|| anyhow::anyhow!("unknown provider {:?}", args.provider))?;
                let local = tokio::task::LocalSet::new();
                local
                    .run_until(crate::acp::run_oneshot(&spec, args.cwd, args.prompt))
                    .await
            }
            Command::Mem(args) => crate::memory::mem_cli(args).await,
        }
    }
}

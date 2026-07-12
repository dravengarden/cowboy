use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use cowboy::agentd::{AgentdArgs, SpawnMode};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSpawnMode {
    Direct,
    SystemdUser,
}

#[derive(Debug, Parser)]
#[command(name = "cowboy-agentd", version)]
struct Args {
    #[arg(long, default_value = "/run/user/1000/cowboy/agentd.sock")]
    socket: PathBuf,
    #[arg(long, default_value = "cowboy-acp-worker")]
    worker_command: PathBuf,
    /// Optional bootstrap generation. Production leaves this empty and lets
    /// the active Cowboy controller declare the desired generation + exact
    /// worker executable over IPC.
    #[arg(long, default_value = "")]
    desired_generation: String,
    #[arg(long, value_enum, default_value_t = CliSpawnMode::Direct)]
    spawn_mode: CliSpawnMode,
    // ACP itself permits one 60 s handshake plus one retry; stay above both so
    // agentd never kills the legitimate retry path prematurely.
    #[arg(long, default_value_t = 135)]
    worker_ready_timeout_seconds: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    cowboy::agentd::run(AgentdArgs {
        socket: args.socket,
        worker_command: args.worker_command,
        desired_generation: args.desired_generation,
        spawn_mode: match args.spawn_mode {
            CliSpawnMode::Direct => SpawnMode::Direct,
            CliSpawnMode::SystemdUser => SpawnMode::SystemdUser,
        },
        worker_ready_timeout: std::time::Duration::from_secs(args.worker_ready_timeout_seconds),
    })
    .await
}

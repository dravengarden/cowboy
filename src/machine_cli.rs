//! Shared CLI for the stable Machine host and its `cowboy-agentd` transition
//! alias. Provider and Zed lifecycle subcommands join this surface without
//! copying the ACP broker implementation.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::agentd::{AgentdArgs, SpawnMode};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSpawnMode {
    Direct,
    SystemdUser,
}

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
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
    // the Machine host never kills the legitimate retry path prematurely.
    #[arg(long, default_value_t = 135)]
    worker_ready_timeout_seconds: u64,
}

/// Parse the stable local-mode CLI and run the existing detached ACP broker.
///
/// # Errors
/// Returns when broker startup or its listener fails.
pub async fn run(command_name: &'static str) -> anyhow::Result<()> {
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
    crate::agentd::run(AgentdArgs {
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

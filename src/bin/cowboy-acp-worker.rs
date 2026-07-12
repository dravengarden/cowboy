use std::path::PathBuf;

use clap::Parser;
use cowboy::worker::WorkerArgs;

#[derive(Debug, Parser)]
#[command(name = "cowboy-acp-worker", version)]
struct Args {
    #[arg(long)]
    socket: PathBuf,
    #[arg(long)]
    session_id: String,
    #[arg(long)]
    provider: String,
    #[arg(long)]
    cwd: PathBuf,
    #[arg(long)]
    resume: Option<String>,
    #[arg(long, default_value_t = false)]
    system: bool,
    #[arg(long)]
    generation: String,
    #[arg(long)]
    worker_epoch: Option<String>,
    #[arg(long, env = "COWBOY_FALLBACK_FOR")]
    fallback_for: Option<String>,
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
    cowboy::worker::run(WorkerArgs {
        socket: args.socket,
        session_id: args.session_id,
        provider: args.provider,
        cwd: args.cwd,
        resume: args.resume,
        system: args.system,
        generation: args.generation,
        worker_epoch: args.worker_epoch,
        fallback_for: args.fallback_for,
    })
    .await
}

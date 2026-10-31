use std::path::PathBuf;

use clap::Parser;
use cowboy::worker::WorkerArgs;

#[derive(Debug, Parser)]
#[command(name = "cowboy-acp-worker", version)]
struct Args {
    #[arg(long, hide = true)]
    launch_gate: bool,
    #[arg(long)]
    socket: PathBuf,
    #[arg(long)]
    session_id: String,
    #[arg(long)]
    provider: String,
    #[arg(long, default_value = "")]
    provider_version: String,
    #[arg(long, default_value = "")]
    provider_generation_digest: String,
    #[arg(long)]
    provider_auth_generation: Option<u64>,
    #[arg(long, default_value = "default")]
    provider_account_id: cowboy::provider_account::ProviderAccountId,
    #[arg(long)]
    cwd: PathBuf,
    #[arg(long, conflicts_with = "fork_from")]
    resume: Option<String>,
    /// Native session to fork with ACP `session/fork` for the first thread.
    #[arg(long)]
    fork_from: Option<String>,
    /// Fork through this turn-final assistant message only.
    #[arg(long, requires = "fork_from")]
    fork_message_id: Option<String>,
    #[arg(long, default_value_t = false)]
    system: bool,
    #[arg(long)]
    generation: String,
    #[arg(long)]
    worker_epoch: Option<String>,
    #[arg(long, env = "COWBOY_FALLBACK_FOR")]
    fallback_for: Option<String>,
    #[arg(long)]
    execution_binding: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.launch_gate {
        anyhow::ensure!(
            !args.provider_account_id.is_default(),
            "launch gate requires a named account"
        );
        cowboy::worker::startup_gate::wait(
            &mut std::io::stdin().lock(),
            &std::env::var(cowboy::worker::startup_gate::LAUNCH_ENV)?,
        )?;
    }
    let mut context = cowboy::logs::Context::new("cowboy-worker");
    context.session = args.session_id.clone();
    context.generation = args.generation.clone();
    context.machine = std::env::var("COWBOY_LOGS_MACHINE_ID").unwrap_or_default();
    let root = args.socket.parent().unwrap_or(std::path::Path::new("."));
    let logs = cowboy::logs::init(cowboy::logs::directory(root), context)?.track_outcome();
    logs.finish(
        cowboy::worker::until_terminated(
            args.session_id.clone(),
            cowboy::worker::run(WorkerArgs {
                socket: args.socket,
                session_id: args.session_id,
                provider: args.provider,
                provider_version: args.provider_version,
                provider_generation_digest: args.provider_generation_digest,
                provider_account_id: args.provider_account_id.clone(),
                provider_auth_generation: args.provider_auth_generation,
                cwd: args.cwd,
                resume: args.resume,
                native_fork: args.fork_from.map(|agent_session_id| {
                    cowboy::runtime_wire::NativeFork {
                        agent_session_id,
                        message_id: args.fork_message_id,
                    }
                }),
                system: args.system,
                generation: args.generation,
                worker_epoch: args.worker_epoch,
                fallback_for: args.fallback_for,
                execution_binding: args
                    .execution_binding
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            }),
        )?
        .await,
    )
}

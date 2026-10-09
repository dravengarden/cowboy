use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "cowboy-execution-host",
    version,
    subcommand_negates_reqs = true
)]
struct Args {
    #[command(subcommand)]
    file: Option<cowboy::execution_host::file_helper::Command>,
    #[arg(long, required = true)]
    contract: Option<PathBuf>,
    #[arg(long, required = true)]
    state_dir: Option<PathBuf>,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long, env = "COWBOY_LOGS_DIR")]
    logs_dir: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let mut args = Args::parse();
    // File utilities are short synchronous processes, not keeper daemons.
    // Avoid creating a Tokio worker thread for every host CPU on each Read.
    if let Some(command) = args.file.take() {
        match cowboy::execution_host::file_helper::run(command) {
            Ok(code) => std::process::exit(code),
            Err(_) => {
                eprintln!("Cowboy target file operation failed");
                std::process::exit(1);
            }
        }
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(args))
}

async fn serve(args: Args) -> anyhow::Result<()> {
    let contract = args
        .contract
        .ok_or_else(|| anyhow::anyhow!("contract required"))?;
    let state_dir = args
        .state_dir
        .ok_or_else(|| anyhow::anyhow!("state directory required"))?;
    let context = cowboy::execution_host::log_context(&contract)?;
    let logs = cowboy::logs::init(
        args.logs_dir.unwrap_or_else(|| state_dir.join("logs")),
        context,
    )?
    .track_outcome();
    logs.finish(
        cowboy::execution_host::run(cowboy::execution_host::Args {
            contract,
            state_dir,
            socket: args.socket,
        })
        .await,
    )
}

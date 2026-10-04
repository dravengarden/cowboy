use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cowboy-execution-host", version)]
struct Args {
    #[arg(long)]
    contract: PathBuf,
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long, env = "COWBOY_LOGS_DIR")]
    logs_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let context = cowboy::execution_host::log_context(&args.contract)?;
    let _logs = cowboy::logs::init(
        args.logs_dir.unwrap_or_else(|| args.state_dir.join("logs")),
        context,
    )?;
    cowboy::execution_host::run(cowboy::execution_host::Args {
        contract: args.contract,
        state_dir: args.state_dir,
        socket: args.socket,
    })
    .await
}

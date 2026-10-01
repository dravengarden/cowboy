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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    cowboy::execution_host::run(cowboy::execution_host::Args {
        contract: args.contract,
        state_dir: args.state_dir,
        socket: args.socket,
    })
    .await
}

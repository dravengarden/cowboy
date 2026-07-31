use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "COWBOY_CODE_ADAPTER_SOCKET")]
    socket: PathBuf,
    #[arg(
        long = "workspace",
        env = "COWBOY_MACHINE_WORKSPACE",
        value_delimiter = ','
    )]
    workspaces: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    cowboy::code_adapter::serve(&args.socket, args.workspaces).await
}

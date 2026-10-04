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
    let args = Args::parse();
    let root = args
        .socket
        .parent()
        .ok_or_else(|| anyhow::anyhow!("adapter socket needs a parent"))?;
    let _logs = cowboy::logs::init(
        cowboy::logs::directory(root),
        cowboy::logs::Context {
            service: "cowboy-code-adapter".into(),
            machine: std::env::var("COWBOY_LOGS_MACHINE_ID").unwrap_or_default(),
            ..Default::default()
        },
    )?;
    cowboy::code_adapter::serve(&args.socket, args.workspaces).await
}

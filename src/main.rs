mod acp;
mod cli;
mod core;
mod provider;
mod server;
mod supervisor;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::Cli::parse().run().await
}

mod acp;
mod cli;
mod provider;
mod server;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::Cli::parse().run().await
}

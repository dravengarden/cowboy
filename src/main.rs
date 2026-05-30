mod acp;
mod acp_bridge;
mod cli;
mod core;
mod files;
mod provider;
mod server;
mod store;
mod supervisor;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::Cli::parse().run().await
}

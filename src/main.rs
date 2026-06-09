mod acp;
mod cli;
mod core;
mod inference;
mod skills;
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

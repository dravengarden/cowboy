mod acp;
mod cgroup;
mod cli;
mod core;
mod inference;
mod memory;
mod skills;
mod files;
mod provider;
mod scheduler;
mod server;
mod store;
mod supervisor;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::Cli::parse().run().await
}

mod acp;
mod cgroup;
mod cli;
mod core;
mod files;
mod inference;
mod persistence;
mod provider;
mod scheduler;
mod server;
mod skills;
mod store;
mod supervisor;

#[cfg(test)]
mod protocol_contract;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cli::Cli::parse().run().await
}

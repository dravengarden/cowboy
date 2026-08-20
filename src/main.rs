use clap::Parser;
use cowboy::cli::Cli;

#[cfg(all(not(target_env = "msvc"), not(target_family = "wasm")))]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Cli::parse().run().await
}

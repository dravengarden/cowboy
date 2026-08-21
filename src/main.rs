use clap::Parser;
use cowboy::cli::Cli;

#[cfg(all(not(target_env = "msvc"), not(target_family = "wasm")))]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn tokio_worker_threads() -> usize {
    std::env::var("COWBOY_TOKIO_WORKERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4)
        .clamp(1, 16)
}

fn main() -> anyhow::Result<()> {
    let workers = tokio_worker_threads();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?
        .block_on(async move { Cli::parse().run().await })
}

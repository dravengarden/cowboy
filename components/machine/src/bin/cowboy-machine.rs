#[cfg(all(not(target_env = "msvc"), not(target_family = "wasm")))]
#[global_allocator]
static GLOBAL_ALLOCATOR: cowboy::Jemalloc = cowboy::Jemalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cowboy_machine::machine_cli::run("cowboy-machine").await
}

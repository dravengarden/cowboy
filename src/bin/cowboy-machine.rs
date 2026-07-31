#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cowboy::machine_cli::run("cowboy-machine").await
}

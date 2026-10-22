#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cowboy_machine::machine_cli::run("cowboy-machine").await
}

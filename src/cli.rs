#![warn(clippy::pedantic)]

use std::io::{self, IsTerminal as _};
#[cfg(feature = "full")]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cowboy",
    version,
    about = "Drive coding-agent CLIs from anywhere over ACP",
    after_help = "On a new computer:\n  1. In the Cowboy UI, create a one-time code\n  2. cowboy register https://<origin>\n  3. Paste the one-time token when asked\n\nThe default register command runs Cowboy Machine in the current terminal. Add --background to install and start a user background service.\nCowboy assigns the machine id. Each Service gets isolated Machine state under ~/.local/state/cowboy-machine/services/.\nCowboy Service stores only the public key."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the cowboy daemon (HTTP + WebSocket). The long-running systemd
    /// service that owns the Hub + supervisor; every surface (Web UI, phone,
    /// native shell) connects to it as a client.
    #[cfg(feature = "full")]
    Serve(ServeArgs),
    /// Expose the running cowboy daemon as a stdio ACP agent for Zed or any
    /// other ACP client. This is a thin bridge; it never starts a second Hub.
    #[cfg(feature = "full")]
    ServeAcp(ServeAcpArgs),
    /// Debug: drive one provider end-to-end (spawn, initialize, prompt, stream).
    #[cfg(feature = "full")]
    TryAgent(TryAgentArgs),
    /// Create a short-lived, single-use token for enrolling a remote Machine.
    #[cfg(feature = "full")]
    MachineEnroll(MachineEnrollArgs),
    /// Revoke a remote Machine identity. Re-enrollment requires a new token
    /// and creates an explicit key-rotation boundary.
    #[cfg(feature = "full")]
    MachineRevoke(MachineRevokeArgs),
    /// Register this computer with a Cowboy instance.
    ///
    /// Create a one-time code in the web UI first, then run:
    ///   `cowboy register https://cowboy.example`
    /// and paste the token. Cowboy assigns the machine id and generates an
    /// Ed25519 key on this computer (mode 0600). The private key never leaves
    /// this machine.
    Register(RegisterArgs),
    /// Show this computer's Machine fingerprint and private-key path.
    Identity(IdentityArgs),
}

#[cfg(feature = "full")]
#[derive(Args)]
pub struct MachineEnrollArgs {
    #[arg(long, env = "COWBOY_DATABASE_URL")]
    database_url: Option<String>,
    /// Legacy PostgreSQL-only spelling. Kept for deployed callers while the
    /// backend-neutral flag becomes canonical.
    #[arg(long, env = "COWBOY_POSTGRES_URL", hide = true)]
    postgres_url: Option<String>,
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    data_dir: PathBuf,
    #[arg(long)]
    machine_id: String,
    #[arg(long)]
    display_name: String,
    #[arg(long, default_value_t = 600)]
    ttl_seconds: i64,
}

#[cfg(feature = "full")]
#[derive(Args)]
pub struct MachineRevokeArgs {
    #[arg(long, env = "COWBOY_DATABASE_URL")]
    database_url: Option<String>,
    /// Legacy PostgreSQL-only spelling. Kept for deployed callers while the
    /// backend-neutral flag becomes canonical.
    #[arg(long, env = "COWBOY_POSTGRES_URL", hide = true)]
    postgres_url: Option<String>,
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    data_dir: PathBuf,
    #[arg(long)]
    machine_id: String,
}

#[derive(Args)]
pub struct RegisterArgs {
    /// Published HTTPS origin of the Cowboy instance.
    origin: String,
    /// Optional override. When omitted, Cowboy assigns the id from the
    /// enrollment token when this computer first connects.
    machine_id: Option<String>,
    #[arg(long)]
    display_name: Option<String>,
    /// `id=/absolute/path` workspace the Machine may use. Defaults to `home=$HOME`.
    #[arg(long = "workspace")]
    workspaces: Vec<String>,
    /// Read the one-time token from a mode-0600 file instead of the TTY.
    #[arg(long)]
    token_file: Option<PathBuf>,
    /// Install and keep Cowboy Machine running as a background service.
    #[arg(long, default_value_t = false)]
    background: bool,
    /// Override the Service-scoped Machine state directory.
    #[arg(long)]
    state_dir: Option<PathBuf>,
}

#[derive(Args)]
pub struct IdentityArgs {
    /// Override the Machine state directory (default ~/.local/state/cowboy-machine).
    #[arg(long)]
    state_dir: Option<PathBuf>,
}

#[cfg(feature = "full")]
#[derive(Args)]
pub struct ServeAcpArgs {
    /// Only expose sessions for this provider. Register one Zed External Agent
    /// entry per provider to preserve the provider identity and logo.
    #[arg(long, default_value = "codex")]
    pub provider: String,

    /// Base URL of the already-running cowboy daemon.
    #[arg(
        long,
        env = "COWBOY_DAEMON_URL",
        default_value = "http://127.0.0.1:3333"
    )]
    pub daemon_url: String,

    /// Product personal access token (`cow_…`). Required; there is no
    /// headerless ACP path. Create one on `/` → account menu → tokens.
    #[arg(long, env = "COWBOY_USER_TOKEN")]
    pub token: Option<String>,
}

#[cfg(feature = "full")]
#[derive(Args)]
pub struct TryAgentArgs {
    /// Agent Plugin id from the active Cowboy Plugin Catalog.
    #[arg(long)]
    provider: String,
    /// Working directory for the session.
    #[arg(long, default_value = ".")]
    cwd: PathBuf,
    /// The prompt to send.
    prompt: String,
}

#[cfg(feature = "full")]
#[derive(Args)]
pub struct ServeArgs {
    /// Address to bind the HTTP/WebSocket server to.
    #[arg(long, env = "COWBOY_BIND", default_value = "127.0.0.1:3333")]
    pub bind: SocketAddr,

    /// Root directory agents are allowed to operate within.
    #[arg(long, env = "COWBOY_WORKSPACE_ROOT", default_value = ".")]
    pub workspace_root: PathBuf,

    /// State/data directory (config + secrets; transcript log persists in
    /// `--database-url` when set).
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    pub data_dir: PathBuf,

    /// Built SPA directory. Static assets are loaded at request time instead
    /// of being embedded in the Rust binary, so a frontend-only rollout does
    /// not restart the API or any agent session.
    #[arg(long, env = "COWBOY_WEB_ROOT", default_value = "web/dist")]
    pub web_root: PathBuf,

    /// Require product-account authentication for the PWA, WebSocket, and
    /// product APIs. Disabled keeps the legacy single-user local surface open.
    #[arg(
        long,
        env = "COWBOY_PRODUCT_AUTH_ENABLED",
        default_value_t = false,
        action = clap::ArgAction::Set
    )]
    pub product_auth_enabled: bool,

    /// Unix socket of Cowboy's isolated Zed protocol adapter. When omitted,
    /// code review remains available but language intelligence is reported as
    /// unavailable.
    #[arg(long, env = "COWBOY_ZED_ADAPTER_SOCKET")]
    pub zed_adapter_socket: Option<PathBuf>,

    /// Controller-local saved-file content cache quota. Set to zero to disable.
    #[arg(
        long,
        env = "COWBOY_CODE_CACHE_BYTES",
        default_value_t = 2 * 1024 * 1024 * 1024_u64
    )]
    pub code_cache_bytes: u64,

    /// Controller-owned signed desired-component manifest sent to every
    /// authenticated Machine. The browser can request reconciliation but
    /// cannot supply artifact URLs, hashes, or signatures.
    #[arg(long, env = "COWBOY_MACHINE_COMPONENTS_MANIFEST")]
    pub machine_components_manifest: Option<PathBuf>,

    /// Optional directory of externally published `.cowboy-plugin` artifacts.
    /// Each release is accepted only with a trusted publisher key and signed
    /// release envelope. The six first-party UI/package contracts are embedded,
    /// but become installable only when a signed runtime release is published.
    #[arg(long, env = "COWBOY_PLUGIN_CATALOG_DIR")]
    pub plugin_catalog_dir: Option<PathBuf>,

    /// `PostgreSQL` or `SQLite` URL for durable Cowboy state. When absent the
    /// daemon runs in pure in-memory mode without restart recovery.
    #[arg(long, env = "COWBOY_DATABASE_URL")]
    pub database_url: Option<String>,

    /// Legacy PostgreSQL-only spelling retained for existing deployments.
    #[arg(long, env = "COWBOY_POSTGRES_URL", hide = true)]
    pub postgres_url: Option<String>,

    /// `VictoriaLogs` base URL used by the bounded client observability relay.
    #[arg(
        long,
        env = "COWBOY_VICTORIA_LOGS_URL",
        default_value = "http://127.0.0.1:6302"
    )]
    pub victoria_logs_url: String,

    /// `VictoriaMetrics` base URL used by the bounded client observability relay.
    #[arg(
        long,
        env = "COWBOY_VICTORIA_METRICS_URL",
        default_value = "http://127.0.0.1:6301"
    )]
    pub victoria_metrics_url: String,

    /// Codex CLI used to query `OpenAI` account usage.
    #[arg(long, env = "COWBOY_CODEX_COMMAND", default_value = "codex")]
    pub codex_command: String,
}

impl Cli {
    /// Runs the selected Cowboy command until it exits.
    ///
    /// # Errors
    ///
    /// Returns an error when command validation, startup, or the selected
    /// long-running service fails.
    #[cfg_attr(not(feature = "full"), allow(clippy::unused_async))]
    pub async fn run(self) -> anyhow::Result<()> {
        // Reqwest is deliberately provider-neutral. Install the one selected
        // rustls provider before any command can construct an HTTPS client;
        // background startup order must not decide whether TLS panics.
        let _ = rustls::crypto::ring::default_provider().install_default();
        match self.command {
            #[cfg(feature = "full")]
            Command::Serve(args) => crate::server::serve(args).await,
            #[cfg(feature = "full")]
            Command::ServeAcp(args) => {
                crate::server::init_tracing();
                crate::acp_bridge::serve(args).await
            }
            #[cfg(feature = "full")]
            Command::TryAgent(args) => {
                crate::server::init_tracing();
                let spec = crate::provider::lookup(&args.provider)
                    .ok_or_else(|| anyhow::anyhow!("unknown provider {:?}", args.provider))?;
                let local = tokio::task::LocalSet::new();
                local
                    .run_until(crate::acp::run_oneshot(&spec, args.cwd, args.prompt))
                    .await
            }
            #[cfg(feature = "full")]
            Command::MachineEnroll(args) => {
                let database_url = args
                    .database_url
                    .as_deref()
                    .or(args.postgres_url.as_deref())
                    .context("--database-url is required")?;
                let store =
                    crate::store::Store::connect(database_url, args.data_dir.join("artifacts"))
                        .await?;
                store.migrate().await?;
                let token = store
                    .create_machine_enrollment(
                        &args.machine_id,
                        &args.display_name,
                        args.ttl_seconds,
                    )
                    .await?;
                println!("{token}");
                Ok(())
            }
            #[cfg(feature = "full")]
            Command::MachineRevoke(args) => {
                let database_url = args
                    .database_url
                    .as_deref()
                    .or(args.postgres_url.as_deref())
                    .context("--database-url is required")?;
                let store =
                    crate::store::Store::connect(database_url, args.data_dir.join("artifacts"))
                        .await?;
                store.migrate().await?;
                store.revoke_machine(&args.machine_id).await
            }
            Command::Register(args) => register_computer(args).await,
            Command::Identity(args) => {
                let state_dirs = crate::machine_install::identity_state_dirs(args.state_dir)?;
                anyhow::ensure!(
                    !state_dirs.is_empty(),
                    "this computer has no Machine key yet; run cowboy register first"
                );
                for (index, state_dir) in state_dirs.iter().enumerate() {
                    let identity = crate::machine_auth::MachineIdentity::load_or_create(state_dir)?;
                    let fingerprint = crate::machine_auth::fingerprint(identity.public_key())?;
                    if index > 0 {
                        println!();
                    }
                    if let Some(service_id) = state_dir.file_name().and_then(|name| name.to_str())
                        && crate::service_identity::valid_service_id(service_id)
                    {
                        println!("Service id:      {service_id}");
                    }
                    let origin = std::fs::read_to_string(state_dir.join("service-origin"))
                        .ok()
                        .map(|value| value.trim().to_owned())
                        .filter(|value| !value.is_empty());
                    if let Some(origin) = origin {
                        println!("Service origin:  {origin}");
                    }
                    println!("Fingerprint:     {fingerprint}");
                    println!("Private key:     {}", identity.private_key_path().display());
                    println!("State:           {}", state_dir.display());
                }
                println!(
                    "Keep the private key on this computer. Cowboy Service stores only the public key."
                );
                Ok(())
            }
        }
    }
}

async fn register_computer(args: RegisterArgs) -> anyhow::Result<()> {
    let token = read_enrollment_token(args.token_file.as_deref())?;
    let workspaces = if args.workspaces.is_empty() {
        default_home_workspace()?
    } else {
        args.workspaces
    };
    let report = crate::machine_install::register(
        &args.origin,
        args.machine_id.as_deref(),
        args.display_name.as_deref(),
        &workspaces,
        &token,
        args.background,
        args.state_dir,
    )
    .await?;
    print_register_report(&report);
    if args.background {
        println!("Cowboy Machine is starting in the background.");
        return Ok(());
    }
    println!();
    println!("Cowboy Machine is running in this terminal.");
    println!("Keep this terminal open; press Ctrl-C to take this computer offline.");
    println!("For a future background registration, add --background to the register command.");
    crate::machine_install::run_foreground(&report).await
}

fn print_register_report(report: &crate::machine_install::RegisterReport) {
    println!("Registration prepared for {}", report.origin);
    println!("Service id:      {}", report.service_id);
    match report.machine_id.as_deref() {
        Some(machine_id) => println!("Machine id:     {machine_id}"),
        None => println!("Machine id:     assigned when this computer enrolls"),
    }
    println!("Fingerprint:    {}", report.fingerprint);
    println!("Private key:    {}", report.private_key.display());
    println!("State:          {}", report.state_dir.display());
    println!("Keep the private key on this computer. Cowboy Service stores only the public key.");
}

fn default_home_workspace() -> anyhow::Result<Vec<String>> {
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(vec![format!("home={home}")])
}

fn read_enrollment_token(token_file: Option<&Path>) -> anyhow::Result<String> {
    if let Some(path) = token_file {
        let token = std::fs::read_to_string(path)
            .with_context(|| format!("reading enrollment token {}", path.display()))?;
        anyhow::ensure!(!token.trim().is_empty(), "enrollment token file is empty");
        return Ok(token);
    }
    anyhow::ensure!(
        io::stdin().is_terminal(),
        "pass --token-file; refusing to read a token from a non-TTY stdin"
    );
    let config = rpassword::ConfigBuilder::new()
        .password_feedback_mask('*')
        .build();
    let token = rpassword::prompt_password_with_config(
        "Paste the enrollment token from Cowboy, then press Enter: ",
        config,
    )
    .context("reading enrollment token from the terminal")?;
    let token = token.trim().to_owned();
    anyhow::ensure!(!token.is_empty(), "enrollment token is required");
    eprintln!("Token received: {}", mask_secret(&token));
    Ok(token)
}

fn mask_secret(value: &str) -> String {
    let length = value.chars().count();
    let hidden = length.saturating_sub(4);
    format!(
        "{}{}",
        "*".repeat(hidden),
        value.chars().skip(hidden).collect::<String>()
    )
}

#[cfg(feature = "full")]
impl ServeArgs {
    #[must_use]
    pub fn database_url(&self) -> Option<&str> {
        self.database_url
            .as_deref()
            .or(self.postgres_url.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{Cli, Command, mask_secret};

    #[test]
    #[cfg(feature = "full")]
    fn serve_accepts_backend_neutral_database_url() {
        let cli =
            Cli::try_parse_from(["cowboy", "serve", "--database-url", "sqlite::memory:"]).unwrap();
        let Command::Serve(args) = cli.command else {
            panic!("expected serve command");
        };
        assert_eq!(args.database_url(), Some("sqlite::memory:"));
    }

    #[test]
    #[cfg(feature = "full")]
    fn serve_keeps_legacy_postgres_url_compatible() {
        let cli =
            Cli::try_parse_from(["cowboy", "serve", "--postgres-url", "postgresql:///cowboy"])
                .unwrap();
        let Command::Serve(args) = cli.command else {
            panic!("expected serve command");
        };
        assert_eq!(args.database_url(), Some("postgresql:///cowboy"));
    }

    #[test]
    #[cfg(feature = "full")]
    fn serve_product_auth_is_an_explicit_toggle_and_defaults_off() {
        let cli = Cli::try_parse_from(["cowboy", "serve"]).unwrap();
        let Command::Serve(args) = cli.command else {
            panic!("expected serve command");
        };
        assert!(!args.product_auth_enabled);

        let cli =
            Cli::try_parse_from(["cowboy", "serve", "--product-auth-enabled", "true"]).unwrap();
        let Command::Serve(args) = cli.command else {
            panic!("expected serve command");
        };
        assert!(args.product_auth_enabled);
    }

    #[test]
    #[cfg(feature = "full")]
    fn serve_acp_accepts_token_flag() {
        let cli = Cli::try_parse_from([
            "cowboy",
            "serve-acp",
            "--provider",
            "codex",
            "--token",
            "cow_testtoken",
        ])
        .unwrap();
        let Command::ServeAcp(args) = cli.command else {
            panic!("expected serve-acp command");
        };
        assert_eq!(args.token.as_deref(), Some("cow_testtoken"));
    }

    #[test]
    fn register_is_origin_and_optional_machine_id() {
        let cli = Cli::try_parse_from([
            "cowboy",
            "register",
            "https://cowboy.example",
            "--token-file",
            "/tmp/cowboy-enroll.token",
        ])
        .unwrap();
        let Command::Register(args) = cli.command else {
            panic!("expected register command");
        };
        assert_eq!(args.origin, "https://cowboy.example");
        assert_eq!(args.machine_id, None);
        assert!(args.workspaces.is_empty());
        assert!(!args.background);
        assert_eq!(
            args.token_file.as_deref(),
            Some(std::path::Path::new("/tmp/cowboy-enroll.token"))
        );

        let cli = Cli::try_parse_from([
            "cowboy",
            "register",
            "https://cowboy.example",
            "macbook-air",
        ])
        .unwrap();
        let Command::Register(args) = cli.command else {
            panic!("expected register command");
        };
        assert_eq!(args.machine_id.as_deref(), Some("macbook-air"));

        let cli = Cli::try_parse_from([
            "cowboy",
            "register",
            "https://cowboy.example",
            "--background",
        ])
        .unwrap();
        let Command::Register(args) = cli.command else {
            panic!("expected register command");
        };
        assert!(args.background);
    }

    #[test]
    fn identity_is_a_top_level_command() {
        let cli = Cli::try_parse_from(["cowboy", "identity"]).unwrap();
        assert!(matches!(cli.command, Command::Identity(_)));
    }

    #[test]
    fn enrollment_token_mask_reveals_only_the_last_four_characters() {
        let token = "7XSdk_AvMYumg66vkgC6ZkVZ_Ak572CgSC_A9jc6zKA";
        let masked = mask_secret(token);
        assert_eq!(masked.chars().count(), token.chars().count());
        assert!(masked.ends_with("6zKA"));
        assert!(
            masked[..masked.len() - 4]
                .chars()
                .all(|character| character == '*')
        );
        assert_eq!(mask_secret("abc"), "abc");
    }
}

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use std::io::{self, IsTerminal as _};
#[cfg(feature = "full")]
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use clap::{Args, Parser, Subcommand};
#[cfg(feature = "full")]
pub mod cardea;

#[derive(Parser)]
#[command(
    name = "cowboy",
    version,
    about = "Drive coding-agent CLIs from anywhere over ACP",
    after_help = "Product clients sign in with `cowboy login https://<origin>`; Cowboy opens the configured browser login and stores a sender-constrained device credential locally.\n\nMachine registration is separate:\n  1. In the Cowboy UI, create a one-time Machine code\n  2. cowboy register https://<origin>\n  3. Paste the one-time token when asked\n\nThe default register command runs Cowboy Machine in the current terminal. Add --background to install and start a user background service.\nCowboy assigns the machine id. Each Service gets isolated Machine state under ~/.local/state/cowboy-machine/services/.\nCowboy Service stores only the public key."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Delegate bounded work to Codex on the current session's execution target.
    #[cfg(unix)]
    #[command(after_help = crate::managed_calls::cli::START_HELP)]
    Codex(crate::managed_calls::cli::StartArgs),
    /// Delegate bounded work to Claude on the current session's execution target.
    #[cfg(unix)]
    #[command(after_help = crate::managed_calls::cli::START_HELP)]
    Claude(crate::managed_calls::cli::StartArgs),
    /// Get a second-opinion review of this worktree's change from another
    /// agent. Optional focus text on stdin; prints only the findings, or one
    /// `skipped:` line when the session does not allow reviews.
    #[cfg(unix)]
    Review(crate::managed_calls::cli::ReviewArgs),
    /// Observe and control parent-scoped managed agent calls.
    #[cfg(unix)]
    Call(crate::managed_calls::cli::CallArgs),
    /// Serve this session's Cowboy tools to its agent as a stdio MCP server.
    /// Outside a Cowboy session it advertises no tools.
    #[cfg(unix)]
    Mcp(crate::agent_api::mcp::McpArgs),
    /// Query, aggregate and analyze `OTel` evidence where it is stored.
    Logs(Box<crate::logs::cli::LogsArgs>),
    /// Inspect, validate and safely change the Service or Device configuration
    /// file. See docs/configuration.md.
    Config(crate::config::cli::ConfigArgs),
    /// Run the cowboy daemon (HTTP + WebSocket). The long-running systemd
    /// service that owns the Hub + supervisor; every surface (Web UI, phone,
    /// native shell) connects to it as a client.
    #[cfg(feature = "full")]
    Serve(Box<ServeArgs>),
    /// Expose the running cowboy daemon as a stdio ACP agent for Zed or any
    /// other ACP client. This is a thin bridge; it never starts a second Hub.
    #[cfg(feature = "full")]
    ServeAcp(ServeAcpArgs),
    /// Sign this computer in through Cowboy's browser login or the Cardea CLI broker.
    #[cfg(feature = "full")]
    Login(LoginArgs),
    /// List, update, reload, hibernate, prompt, clear, rename, delete or start
    /// sessions through this computer's login, as the Web UI does.
    #[cfg(feature = "full")]
    Session(crate::product_cli::SessionArgs),
    /// Inspect and configure inherited Sessions directory defaults.
    #[cfg(feature = "full")]
    Dir(crate::product_cli::DirArgs),
    /// Inspect Provider subscriptions and manage an explicit native login.
    #[cfg(feature = "full")]
    Account(crate::product_cli::accounts::AccountArgs),
    /// Call any authenticated Cowboy HTTP route through this computer's login.
    #[cfg(feature = "full")]
    Api(crate::product_cli::ApiArgs),
    /// Send any Cowboy WebSocket command through this computer's login.
    #[cfg(feature = "full")]
    Send(crate::product_cli::SendArgs),
    /// Operate the Controller through its explicitly enabled private host endpoint.
    #[cfg(all(feature = "full", unix))]
    Operator(crate::local_operator::OperatorArgs),
    /// Offline Cardea catalog preparation; human activation remains required.
    #[cfg(feature = "full")]
    Cardea {
        #[command(subcommand)]
        command: cardea::Command,
    },
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
    /// Copy a complete Cowboy `PostgreSQL` store into a new `SQLite` database.
    ///
    /// The destination must not exist. The source is read through one
    /// repeatable-read transaction, and the destination is committed only
    /// after every table and integrity check succeeds.
    #[cfg(feature = "full")]
    StoreCopy(StoreCopyArgs),
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
    /// Check an untrusted composition proposal without installing or authorizing it.
    Composition {
        #[command(subcommand)]
        command: CompositionCommand,
    },
}

#[derive(Subcommand)]
pub enum CompositionCommand {
    /// Validate scopes, placements and explicit dependencies; print a read-only report.
    Check { file: PathBuf },
}

#[cfg(test)]
mod composition_cli_tests {
    use super::*;

    #[test]
    fn composition_is_an_explicit_read_only_command() {
        let cli = Cli::try_parse_from(["cowboy", "composition", "check", "proposal.json"]).unwrap();
        assert!(matches!(cli.command, Command::Composition {
            command: CompositionCommand::Check { file }
        } if file == Path::new("proposal.json")));
        assert!(
            Cli::try_parse_from(["cowboy", "composition", "activate", "proposal.json"]).is_err()
        );
        assert!(
            Cli::try_parse_from([
                "cowboy",
                "composition",
                "check",
                "--authorize",
                "proposal.json"
            ])
            .is_err()
        );
    }
}

#[cfg(feature = "full")]
#[derive(Args)]
pub struct StoreCopyArgs {
    /// `PostgreSQL` source URL.
    #[arg(long)]
    pub source: String,
    /// New `SQLite` destination URL. The database file must not exist.
    #[arg(long)]
    pub destination: String,
    /// Cowboy data directory whose artifacts remain shared across the cutover.
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    pub data_dir: PathBuf,
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
    pub origin: String,
    /// Optional override. When omitted, Cowboy assigns the id from the
    /// enrollment token when this computer first connects.
    pub machine_id: Option<String>,
    #[arg(long)]
    pub display_name: Option<String>,
    /// `id=/absolute/path` workspace the Machine may use. Defaults to `home=$HOME`.
    #[arg(long = "workspace")]
    pub workspaces: Vec<String>,
    /// Read the one-time token from a mode-0600 file instead of the TTY.
    #[arg(long)]
    pub token_file: Option<PathBuf>,
    /// Install and keep Cowboy Machine running as a background service.
    #[arg(long, default_value_t = false)]
    pub background: bool,
    /// Override the Service-scoped Machine state directory.
    #[arg(long)]
    pub state_dir: Option<PathBuf>,
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

    /// Place newly created sessions on this registered Machine. Existing
    /// sessions retain their original placement when loaded.
    #[arg(long, default_value = "local")]
    pub machine: String,

    /// Run the AI on this enrolled Machine while --machine owns the Project.
    /// Uses Cowboy's native Remote execution interface; no local fallback.
    #[arg(long, requires = "workspace")]
    pub runtime_machine: Option<String>,

    /// Registered workspace ID for new sessions on a non-local Machine.
    /// Required for remote creation; ACP's cwd remains a local-client path.
    #[arg(long)]
    pub workspace: Option<String>,

    /// Base URL of the already-running cowboy daemon.
    #[arg(
        long,
        env = "COWBOY_DAEMON_URL",
        default_value = "http://127.0.0.1:3333"
    )]
    pub daemon_url: String,

    /// Legacy personal access token. New clients authorize automatically in
    /// the browser and do not need this option.
    #[arg(long, env = "COWBOY_USER_TOKEN", hide = true)]
    pub token: Option<String>,

    /// Override the private device-credential directory.
    #[arg(long, env = "COWBOY_AUTH_STATE_DIR")]
    pub auth_state_dir: Option<PathBuf>,

    /// Name shown when approving this computer and in Account → Devices.
    #[arg(long)]
    pub device_name: Option<String>,
}

#[cfg(feature = "full")]
#[derive(Args)]
pub struct LoginArgs {
    /// Published HTTPS origin of the Cowboy instance.
    pub origin: String,

    /// Override the private device-credential directory.
    #[arg(long, env = "COWBOY_AUTH_STATE_DIR")]
    pub auth_state_dir: Option<PathBuf>,

    /// Name shown when approving this computer and in Account → Devices.
    #[arg(long)]
    pub device_name: Option<String>,
    /// Use the native Cardea device broker instead of a Cowboy browser login.
    #[arg(long, env = "COWBOY_CARDEA_PROFILE")]
    pub cardea_profile: Option<String>,
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

    /// Product authentication is mandatory. The legacy false value is rejected.
    #[arg(
        long,
        env = "COWBOY_PRODUCT_AUTH_ENABLED",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    pub product_auth_enabled: bool,

    /// Protected Cardea OIDC consumer profile. When present, the product login
    /// page offers Cardea alongside the local Cowboy password.
    #[arg(long, env = "COWBOY_CARDEA_OIDC_CONFIG")]
    pub cardea_oidc_config: Option<PathBuf>,

    /// Protected authentication-method configuration. Every referenced
    /// Authentication Provider must be an exact signed Catalog release.
    #[arg(long, env = "COWBOY_AUTH_CONFIG")]
    pub auth_config: Option<PathBuf>,

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
    /// cannot supply artifact URLs, hashes, or signatures. Re-read in place:
    /// `automatic` records converge continuously, and an invalid manifest
    /// keeps the last accepted set.
    #[arg(long, env = "COWBOY_MACHINE_COMPONENTS_MANIFEST")]
    pub machine_components_manifest: Option<PathBuf>,

    /// Optional directory of externally published `.cowboy-plugin` artifacts.
    /// Each release is accepted only with a trusted publisher key and signed
    /// release envelope. The six first-party UI/package contracts are embedded,
    /// but become installable only when a signed runtime release is published.
    #[arg(long, env = "COWBOY_PLUGIN_CATALOG_DIR")]
    pub plugin_catalog_dir: Option<PathBuf>,

    /// Private Controller host activation policy: `bootstrap` or `catalog_only`,
    /// with exact signed host release selections. Publication is independent.
    #[arg(long, env = "COWBOY_PLUGIN_HOST_CONFIG")]
    pub plugin_host_config: Option<PathBuf>,

    /// Private opt-in core local-auth storage policy. Requires durable storage;
    /// adopting a legacy namespace is a stopped-Controller ownership handoff.
    #[arg(long, env = "COWBOY_CORE_SECURITY_CONFIG")]
    pub core_security_config: Option<PathBuf>,

    /// Verify host selections against the signed Catalog, print a JSON report,
    /// and exit without starting services or writing state. Uses the same
    /// authentication and database flags as an actual start; never connects to
    /// the database or executes migrations, Plugins, or login requests.
    /// Validates explicit telemetry writer/background policies against the
    /// existing Service identity; this is not binding or export readiness.
    #[arg(long)]
    pub check_plugin_hosts: bool,

    /// Inspect supported signed Catalog releases without creating Service state,
    /// connecting to storage, running a Plugin, or starting a listener. Does not
    /// validate host policy or login configuration; use `--check-plugin-hosts`
    /// for activation preflight.
    #[arg(long, conflicts_with = "check_plugin_hosts")]
    pub check_plugin_catalog: bool,

    /// `PostgreSQL` or `SQLite` URL for durable Cowboy state. When absent the
    /// daemon runs in pure in-memory mode without restart recovery.
    #[arg(long, env = "COWBOY_DATABASE_URL")]
    pub database_url: Option<String>,

    /// Legacy PostgreSQL-only spelling retained for existing deployments.
    #[arg(long, env = "COWBOY_POSTGRES_URL", hide = true)]
    pub postgres_url: Option<String>,

    /// Private rotating JSONL directory (default: instance-specific /tmp child).
    #[arg(long, env = "COWBOY_TELEMETRY_DIR")]
    pub telemetry_dir: Option<PathBuf>,

    /// Local diagnostic destination. SQLite shares the host `OTel` query store;
    /// JSONL retains the legacy format. Network export is configured separately.
    #[arg(
        long,
        env = "COWBOY_TELEMETRY_LOCAL_BACKEND",
        value_enum,
        default_value = "sqlite"
    )]
    pub telemetry_local_backend: crate::logs::LocalBackend,

    /// Maximum legacy JSONL segment age. SQLite retention uses logs configure.
    #[arg(long, env = "COWBOY_TELEMETRY_RETAIN_SECONDS", default_value_t = 7 * 86400)]
    pub telemetry_retain_seconds: u64,

    /// Maximum bytes per diagnostic segment; incident/accounting ledgers stay in DB.
    #[arg(long, env = "COWBOY_TELEMETRY_SEGMENT_BYTES", default_value_t = crate::telemetry_file::DEFAULT_SEGMENT_BYTES)]
    pub telemetry_segment_bytes: u64,

    /// Total retained diagnostic segments, including the current file.
    #[arg(long, env = "COWBOY_TELEMETRY_RETAINED_FILES", default_value_t = crate::telemetry_file::DEFAULT_RETAINED_FILES)]
    pub telemetry_retained_files: usize,

    /// Optional exact Machine/Plugin/release selection. No remote export by default.
    #[arg(long, env = "COWBOY_TELEMETRY_PLUGIN_CONFIG")]
    pub telemetry_plugin_config: Option<PathBuf>,

    /// Explicit private managed OTLP startup policy; never inferred from history.
    #[arg(
        long,
        env = "COWBOY_TELEMETRY_MANAGED_EXPORT_POLICY",
        conflicts_with = "telemetry_plugin_config"
    )]
    pub telemetry_managed_export_policy: Option<PathBuf>,

    /// Private per-target binding/recovery admission; does not authorize exports.
    #[arg(long, env = "COWBOY_TELEMETRY_WRITER_POLICY")]
    pub telemetry_writer_policy: Option<PathBuf>,

    /// Private candidate admission for owned navigation. Protocol support alone
    /// does not enable acquisition; query/cleanup remain original-owner only.
    #[arg(
        long,
        env = "COWBOY_CODE_NAVIGATION_ADMISSION",
        value_enum,
        default_value = "closed"
    )]
    pub code_navigation_admission: CodeNavigationAdmission,

    /// Bootstrap preferred Agent Machine, used only until Cowboy's native
    /// project-placement policy is saved. Machine policy controls admission.
    #[arg(long, env = "COWBOY_EXECUTION_RUNTIME_MACHINE")]
    pub execution_runtime_machine: Option<String>,

    /// Restrict a Provider's sessions, login executor and usage commands to an
    /// enrolled Machine. Repeat provider=machine to permit multiple Machines.
    /// Omitted Providers retain their ordinary placement policy.
    #[arg(long = "provider-runtime-machine", value_name = "PROVIDER=MACHINE")]
    pub provider_runtime_machines: Vec<String>,

    /// Machines this Controller may execute Code reads for on its OWN
    /// filesystem. A Machine declares its connection mode in its authenticated
    /// hello, and that declaration is not evidence of where it runs: an
    /// enrolled remote Machine could otherwise name Controller-host paths and
    /// have them read back. The declaration is now a request; this list is the
    /// permission. Empty means no Machine may be colocated, so a local Machine
    /// must be named here explicitly. Naming a Machine never makes it
    /// colocated on its own — it must still declare local mode.
    #[arg(
        long = "colocated-machine",
        env = "COWBOY_COLOCATED_MACHINES",
        value_delimiter = ','
    )]
    pub colocated_machines: Vec<String>,
}

/// Explicit private rollout policy, independent of Machine protocol support.
#[cfg(feature = "full")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum CodeNavigationAdmission {
    #[default]
    Closed,
    Candidate,
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
            Command::Logs(args) => args.run().await,
            #[cfg(unix)]
            Command::Codex(args) => crate::managed_calls::cli::start("codex", args).await,
            #[cfg(unix)]
            Command::Claude(args) => crate::managed_calls::cli::start("claude-code", args).await,
            #[cfg(unix)]
            Command::Review(args) => crate::managed_calls::cli::review(args).await,
            #[cfg(unix)]
            Command::Call(args) => crate::managed_calls::cli::call(args).await,
            #[cfg(unix)]
            Command::Mcp(args) => crate::agent_api::mcp::serve(args).await,
            Command::Config(args) => args.run(),
            #[cfg(feature = "full")]
            Command::Serve(args) => crate::controller_hooks::serve(*args).await,
            #[cfg(feature = "full")]
            Command::ServeAcp(args) => {
                crate::controller_hooks::init_tracing();
                crate::acp_bridge::serve(args).await
            }
            #[cfg(feature = "full")]
            Command::Login(args) => login_client(args).await,
            #[cfg(feature = "full")]
            Command::Session(args) => Box::pin(crate::product_cli::run_session(args)).await,
            #[cfg(feature = "full")]
            Command::Dir(args) => Box::pin(crate::product_cli::run_dir(args)).await,
            #[cfg(feature = "full")]
            Command::Account(args) => Box::pin(crate::product_cli::accounts::run(args)).await,
            #[cfg(feature = "full")]
            Command::Api(args) => Box::pin(crate::product_cli::run_api(args)).await,
            #[cfg(feature = "full")]
            Command::Send(args) => Box::pin(crate::product_cli::run_send(args)).await,
            #[cfg(all(feature = "full", unix))]
            Command::Operator(args) => crate::local_operator::run(args).await,
            #[cfg(feature = "full")]
            Command::Cardea { command } => cardea::run(command),
            #[cfg(feature = "full")]
            Command::TryAgent(args) => run_trial_agent(args).await,
            #[cfg(feature = "full")]
            Command::MachineEnroll(args) => enroll_machine(args).await,
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
            #[cfg(feature = "full")]
            Command::StoreCopy(args) => run_store_copy(args).await,
            Command::Register(args) => register_computer(args).await,
            Command::Composition {
                command: CompositionCommand::Check { file },
            } => crate::composition::check_file(&file),
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

#[cfg(feature = "full")]
async fn enroll_machine(args: MachineEnrollArgs) -> anyhow::Result<()> {
    let database_url = args
        .database_url
        .as_deref()
        .or(args.postgres_url.as_deref())
        .context("--database-url is required")?;
    let store = crate::store::Store::connect(database_url, args.data_dir.join("artifacts")).await?;
    store.migrate().await?;
    let token = store
        .create_machine_enrollment(&args.machine_id, &args.display_name, args.ttl_seconds)
        .await?;
    println!("{token}");
    Ok(())
}

#[cfg(feature = "full")]
async fn login_client(args: LoginArgs) -> anyhow::Result<()> {
    crate::controller_hooks::init_tracing();
    let base_url = crate::client_auth_client::normalized_base_url(&args.origin)?;
    let authentication = crate::client_auth_client::ClientAuthentication::new(
        base_url.clone(),
        None,
        args.auth_state_dir,
        args.device_name,
    )?;
    if let Some(profile) = args.cardea_profile.as_deref() {
        authentication.ensure_cardea_login(profile).await?;
    } else {
        authentication.ensure_login().await?;
    }
    eprintln!("Cowboy login ready for {base_url}");
    Ok(())
}

#[cfg(feature = "full")]
async fn run_trial_agent(args: TryAgentArgs) -> anyhow::Result<()> {
    crate::controller_hooks::init_tracing();
    let spec = crate::provider::lookup(&args.provider)
        .ok_or_else(|| anyhow::anyhow!("unknown provider {:?}", args.provider))?;
    tokio::task::LocalSet::new()
        .run_until(crate::acp::run_oneshot(&spec, args.cwd, args.prompt))
        .await
}

#[cfg(feature = "full")]
async fn run_store_copy(args: StoreCopyArgs) -> anyhow::Result<()> {
    let report = crate::store::postgres_to_sqlite(
        &args.source,
        &args.destination,
        args.data_dir.join("artifacts"),
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
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

pub fn mask_secret(value: &str) -> String {
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

    /// Hermetic argument fixture without inherited deployment configuration.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_plugin_check(data_dir: &Path) -> Self {
        let cli = Cli::try_parse_from(["cowboy", "serve", "--check-plugin-hosts"]).unwrap();
        let Command::Serve(mut args) = cli.command else {
            panic!("serve command expected");
        };
        data_dir.clone_into(&mut args.data_dir);
        args.plugin_catalog_dir = None;
        args.plugin_host_config = None;
        args.core_security_config = None;
        args.auth_config = None;
        args.cardea_oidc_config = None;
        args.database_url = None;
        args.postgres_url = None;
        args.telemetry_writer_policy = None;
        args.telemetry_managed_export_policy = None;
        args.telemetry_plugin_config = None;
        args.product_auth_enabled = false;
        *args
    }
}

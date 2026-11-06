//! Arguments of the long-running `serve` and `serve-acp` commands. Plugin
//! activation, telemetry policy and the Controller hooks read them, so they
//! live below `cli`, which only parses and dispatches.

#![allow(clippy::struct_excessive_bools)]

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Args;

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

//! User-scoped macOS/Linux installation for `cowboy-machine`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Parser;

mod bootstrap_probe;
mod signed_bootstrap;

// Service IDs already consume 36 bytes. Compact socket basenames leave room
// for ordinary Linux/macOS home paths without moving private runtime state.
const MACHINE_SOCKET: &str = "run/m";
const USAGE_SOCKET: &str = "run/u";
const CODE_SOCKET: &str = "run/c";
const ZED_SOCKET: &str = "run/z";

fn validate_socket_paths(state: &Path) -> Result<()> {
    // macOS has a 104-byte sun_path including its terminating NUL; Linux's
    // larger limit must not make a generated installation nonportable.
    for relative in [MACHINE_SOCKET, USAGE_SOCKET, CODE_SOCKET, ZED_SOCKET] {
        anyhow::ensure!(
            state.join(relative).as_os_str().as_encoded_bytes().len() <= 103,
            "Machine state directory is too long for Unix sockets; choose a shorter --state-dir"
        );
    }
    Ok(())
}

#[derive(Debug, Parser)]
pub struct InstallArgs {
    #[arg(long)]
    controller_url: String,
    #[arg(long)]
    service_id: String,
    #[arg(long)]
    machine_id: Option<String>,
    #[arg(long)]
    display_name: Option<String>,
    #[arg(long = "workspace", required = true)]
    workspaces: Vec<String>,
    #[arg(long, required_unless_present = "refresh", conflicts_with = "refresh")]
    enrollment_token: Option<String>,
    /// Refresh an enrolled Machine without replacing its identity or token.
    #[arg(long)]
    refresh: bool,
    /// Permit Service-authorized Plugin lifecycle operations on this Machine.
    #[arg(long)]
    plugin_operation_admission: bool,
    #[arg(long)]
    artifact_public_key: Option<PathBuf>,
    #[arg(long, conflicts_with = "bootstrap_manifest")]
    machine_binary: Option<PathBuf>,
    /// Signed singleton Machine host manifest for an exact three-file bootstrap archive.
    #[arg(long, requires_all = ["bootstrap_artifact", "artifact_public_key"])]
    bootstrap_manifest: Option<PathBuf>,
    #[arg(long, requires = "bootstrap_manifest")]
    bootstrap_artifact: Option<PathBuf>,
    #[arg(long)]
    state_dir: Option<PathBuf>,
    #[arg(long, default_value_t = 8)]
    max_sessions: u32,
    #[arg(long, default_value_t = false)]
    draining: bool,
    #[arg(long)]
    no_start: bool,
}

pub fn run() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args = InstallArgs::parse();
    install(args)
}

pub struct RegisterReport {
    pub origin: String,
    pub service_id: String,
    pub machine_id: Option<String>,
    pub state_dir: PathBuf,
    pub private_key: PathBuf,
    pub fingerprint: String,
    pub launcher: PathBuf,
}

pub fn legacy_default_state_dir() -> Result<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    Ok(home.join(".local/state/cowboy-machine"))
}

pub fn identity_state_dirs(explicit: Option<PathBuf>) -> Result<Vec<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(vec![path]);
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let services = home.join(".local/state/cowboy-machine/services");
    let mut states = if services.is_dir() {
        std::fs::read_dir(&services)
            .context("listing Service-scoped Machine identities")?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.join("identity_ed25519").is_file())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    states.sort();
    if states.is_empty() {
        let legacy = legacy_default_state_dir()?;
        if legacy.join("identity_ed25519").is_file() {
            states.push(legacy);
        }
    }
    Ok(states)
}

pub async fn register(
    origin: &str,
    machine_id: Option<&str>,
    display_name: Option<&str>,
    workspaces: &[String],
    token: &str,
    background: bool,
    state_dir: Option<PathBuf>,
) -> Result<RegisterReport> {
    let controller_url = normalize_controller_url(origin)?;
    anyhow::ensure!(!token.trim().is_empty(), "enrollment token is required");
    let service_id = fetch_service_id(&controller_url).await?;
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let state_dir = match state_dir {
        Some(path) => path,
        None => crate::service_identity::service_state_dir(&home, &service_id)?,
    };
    validate_socket_paths(&state_dir)?;
    crate::session_deletion_admission::require_empty_portable_namespace(&state_dir)?;
    crate::session_deletion_admission::reader_floor::require_absent_for_install(&state_dir)?;
    let host = machine_host_binary(None);
    anyhow::ensure!(
        host.is_file(),
        "cowboy-machine was not found next to this cowboy binary ({host}). Install both on this computer, then run register again.",
        host = host.display()
    );
    let bundle = bootstrap_probe::Bundle::prepare(&host)?;
    crate::session_deletion_admission::require_empty_portable_namespace(&state_dir)?;
    crate::session_deletion_admission::reader_floor::require_absent_for_install(&state_dir)?;
    bind_service_origin(&state_dir, &controller_url)?;
    let identity = crate::machine_auth::MachineIdentity::load_or_create(&state_dir)?;
    let fingerprint = crate::machine_auth::fingerprint(identity.public_key())?;
    let machine_id = machine_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned);
    let install_args = InstallArgs {
        controller_url: controller_url.clone(),
        service_id: service_id.clone(),
        machine_id: machine_id.clone(),
        display_name: display_name.map(str::to_owned),
        workspaces: workspaces.to_vec(),
        enrollment_token: Some(token.trim().to_owned()),
        refresh: false,
        plugin_operation_admission: false,
        artifact_public_key: None,
        machine_binary: None,
        bootstrap_manifest: None,
        bootstrap_artifact: None,
        state_dir: Some(state_dir.clone()),
        max_sessions: 8,
        draining: false,
        no_start: false,
    };
    let (home, launcher) = prepare_install_from_bundle(&install_args, &home, Some(bundle))?;
    if background {
        install_background_service(&home, &launcher, &service_id, false)?;
    }
    Ok(RegisterReport {
        origin: controller_url,
        service_id,
        machine_id,
        private_key: identity.private_key_path().to_path_buf(),
        fingerprint,
        state_dir,
        launcher,
    })
}

pub async fn run_foreground(report: &RegisterReport) -> Result<()> {
    let status = tokio::process::Command::new(&report.launcher)
        .status()
        .await
        .with_context(|| format!("starting Cowboy Machine from {}", report.launcher.display()))?;
    anyhow::ensure!(status.success(), "Cowboy Machine exited with {status}");
    Ok(())
}

fn machine_host_binary(explicit: Option<&Path>) -> PathBuf {
    explicit.map_or_else(
        || {
            std::env::current_exe()
                .unwrap_or_else(|_| PathBuf::from("cowboy-machine-install"))
                .with_file_name("cowboy-machine")
        },
        Path::to_path_buf,
    )
}

fn companion_binary(machine: &Path, name: &str) -> PathBuf {
    machine.with_file_name(name)
}

fn normalize_controller_url(origin: &str) -> Result<String> {
    let url = url::Url::parse(origin).context("invalid Cowboy origin")?;
    let host = url.host_str().unwrap_or_default();
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "cowboy register requires https:// except loopback HTTP"
    );
    Ok(origin.trim_end_matches('/').to_owned())
}

#[derive(serde::Deserialize)]
struct MachineServiceResponse {
    service_id: String,
}

async fn fetch_service_id(controller_url: &str) -> Result<String> {
    let endpoint = format!("{controller_url}/api/machine/service");
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(endpoint)
        .send()
        .await
        .context("contacting Cowboy Service")?
        .error_for_status()
        .context("Cowboy Service identity request rejected")?
        .json::<MachineServiceResponse>()
        .await
        .context("decoding Cowboy Service identity")?;
    anyhow::ensure!(
        crate::service_identity::valid_service_id(&response.service_id),
        "Cowboy Service returned an invalid identity"
    );
    Ok(response.service_id)
}

fn bind_service_origin(state_dir: &Path, origin: &str) -> Result<()> {
    std::fs::create_dir_all(state_dir).context("creating Service-scoped Machine state")?;
    set_mode(state_dir, 0o700)?;
    let path = state_dir.join("service-origin");
    if path.exists() {
        let existing = std::fs::read_to_string(&path).context("reading Service origin binding")?;
        anyhow::ensure!(
            existing.trim() == origin,
            "this Service identity is already bound to a different origin ({})",
            existing.trim()
        );
        return Ok(());
    }
    std::fs::write(&path, format!("{origin}\n")).context("writing Service origin binding")?;
    set_mode(&path, 0o600)?;
    Ok(())
}

fn install(args: InstallArgs) -> Result<()> {
    if args.refresh {
        let origin = normalize_controller_url(&args.controller_url)?;
        let service_id = tokio::runtime::Runtime::new()?.block_on(fetch_service_id(&origin))?;
        anyhow::ensure!(
            service_id == args.service_id,
            "refresh Service id does not match the Controller"
        );
    }
    let (home, launcher) = prepare_install(&args)?;
    install_background_service(&home, &launcher, &args.service_id, args.no_start)
}

fn prepare_install(args: &InstallArgs) -> Result<(PathBuf, PathBuf)> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    prepare_install_at(args, &home)
}

fn prepare_install_at(args: &InstallArgs, home: &Path) -> Result<(PathBuf, PathBuf)> {
    prepare_install_from_bundle(args, home, None)
}

fn prepare_install_from_bundle(
    args: &InstallArgs,
    home: &Path,
    bundle: Option<bootstrap_probe::Bundle>,
) -> Result<(PathBuf, PathBuf)> {
    validate_scalar(&args.controller_url)?;
    anyhow::ensure!(
        crate::service_identity::valid_service_id(&args.service_id),
        "invalid Cowboy Service id"
    );
    if let Some(machine_id) = &args.machine_id {
        validate_scalar(machine_id)?;
    }
    for workspace in &args.workspaces {
        validate_scalar(workspace)?;
    }
    let state = args.state_dir.clone().map_or_else(
        || crate::service_identity::service_state_dir(home, &args.service_id),
        Ok,
    )?;
    validate_socket_paths(&state)?;
    crate::session_deletion_admission::require_empty_portable_namespace(&state)?;
    crate::session_deletion_admission::reader_floor::require_absent_for_install(&state)?;
    let installed = if args.refresh {
        installed_launcher(args, &state, home)?
    } else {
        None
    };
    validate_install_mode(args, &state, installed.as_ref())?;
    let bundle = match bundle {
        Some(bundle) => bundle,
        None => {
            if let Some(manifest) = &args.bootstrap_manifest {
                bootstrap_probe::Bundle::prepare_signed(
                    manifest,
                    args.bootstrap_artifact
                        .as_deref()
                        .context("signed bootstrap requires artifact")?,
                    args.artifact_public_key
                        .as_deref()
                        .context("signed bootstrap requires publisher key")?,
                )?
            } else {
                bootstrap_probe::Bundle::prepare(&machine_host_binary(
                    args.machine_binary.as_deref(),
                ))?
            }
        }
    };
    bundle.verify()?;
    // The admitted probe is executable code; do not publish over new refusal state.
    crate::session_deletion_admission::require_empty_portable_namespace(&state)?;
    crate::session_deletion_admission::reader_floor::require_absent_for_install(&state)?;
    bind_service_origin(&state, &normalize_controller_url(&args.controller_url)?)?;
    let config = home
        .join(".config/cowboy-machine/services")
        .join(&args.service_id);
    let runtime = home.join(".local/bin");
    std::fs::create_dir_all(state.join("bootstrap"))?;
    std::fs::create_dir_all(&config)?;
    std::fs::create_dir_all(&runtime)?;
    set_mode(&state, 0o700)?;
    set_mode(&config, 0o700)?;

    for name in bootstrap_probe::PAYLOADS {
        let path = bundle.payload(name);
        atomic_replace(&state.join("bootstrap").join(name), 0o755, |file| {
            let mut input = std::fs::File::open(&path)?;
            std::io::copy(&mut input, file)?;
            Ok(())
        })?;
    }

    let token = state.join("enrollment-token");
    if let Some(value) = &args.enrollment_token {
        atomic_write(&token, value.as_bytes(), 0o600)?;
    }
    let launcher = installed.as_ref().map_or_else(
        || runtime.join(format!("cowboy-machine-launch-{}", args.service_id)),
        |installed| installed.path.clone(),
    );
    let mut script = launcher_script(args, &state, &token);
    if let Some(installed) = &installed {
        for (flag, relative) in [
            ("--socket", MACHINE_SOCKET),
            ("--provider-usage-socket", USAGE_SOCKET),
            ("--code-adapter-socket", CODE_SOCKET),
            ("--zed-adapter-socket", ZED_SOCKET),
        ] {
            if let Some(value) = installed.value(flag) {
                script = script.replace(
                    &shell_quote(&state.join(relative).display().to_string()),
                    &shell_quote(value),
                );
            }
        }
    }
    atomic_write(&launcher, script.as_bytes(), 0o755)?;

    Ok((home.to_path_buf(), launcher))
}

struct InstalledLauncher {
    path: PathBuf,
    arguments: Vec<String>,
}

impl InstalledLauncher {
    fn value(&self, flag: &str) -> Option<&str> {
        self.arguments
            .windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    }
}

fn installed_launcher(
    args: &InstallArgs,
    state: &Path,
    home: &Path,
) -> Result<Option<InstalledLauncher>> {
    for name in [
        format!("cowboy-machine-launch-{}", args.service_id),
        "cowboy-machine-launch".to_owned(),
    ] {
        let path = home.join(".local/bin").join(name);
        if !path.is_file() {
            continue;
        }
        let script = std::fs::read_to_string(&path)?;
        let command = script
            .lines()
            .find(|line| line.starts_with("exec "))
            .context("installed launcher has no exec command")?;
        let installed = InstalledLauncher {
            path,
            arguments: shell_words::split(command)
                .context("decoding installed launcher arguments")?,
        };
        anyhow::ensure!(
            installed.value("--service-id") == Some(args.service_id.as_str()),
            "installed launcher belongs to a different Service"
        );
        anyhow::ensure!(
            installed.value("--state-dir") == state.to_str(),
            "installed launcher uses a different state directory"
        );
        anyhow::ensure!(
            installed
                .value("--controller-url")
                .map(normalize_controller_url)
                .transpose()?
                == Some(normalize_controller_url(&args.controller_url)?),
            "installed launcher uses a different Controller origin"
        );
        return Ok(Some(installed));
    }
    Ok(None)
}

fn validate_install_mode(
    args: &InstallArgs,
    state: &Path,
    installed: Option<&InstalledLauncher>,
) -> Result<()> {
    if args.refresh {
        anyhow::ensure!(
            args.enrollment_token.is_none(),
            "refresh cannot enroll again"
        );
        anyhow::ensure!(
            state.join("identity_ed25519").is_file(),
            "refresh requires an existing Machine identity in --state-dir"
        );
        let origin = match std::fs::read_to_string(state.join("service-origin")) {
            Ok(origin) => origin,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => installed.and_then(|launcher| launcher.value("--controller-url")).context("refresh requires an existing Service origin binding or matching installed launcher")?.to_owned(),
            Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(
            origin.trim() == normalize_controller_url(&args.controller_url)?,
            "refresh cannot change the Service origin"
        );
        let machine_id = std::fs::read_to_string(state.join("machine-id"))
            .context("refresh requires an enrolled Machine id")?;
        anyhow::ensure!(
            !machine_id.trim().is_empty(),
            "refresh requires a nonempty enrolled Machine id"
        );
        if let Some(requested) = &args.machine_id {
            anyhow::ensure!(
                requested == machine_id.trim(),
                "refresh cannot change the enrolled Machine id"
            );
        }
    } else {
        anyhow::ensure!(
            args.enrollment_token
                .as_ref()
                .is_some_and(|value| !value.trim().is_empty()),
            "enrollment token is required"
        );
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    use std::io::Write as _;
    atomic_replace(path, mode, |file| {
        file.write_all(bytes)?;
        Ok(())
    })
}

fn atomic_replace(
    path: &Path,
    mode: u32,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let temporary = path.with_extension(format!("stage-{}", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        write(&mut file)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn install_background_service(
    home: &Path,
    launcher: &Path,
    service_id: &str,
    no_start: bool,
) -> Result<()> {
    let service_id = if launcher
        .file_name()
        .is_some_and(|name| name == "cowboy-machine-launch")
    {
        ""
    } else {
        service_id
    };
    if cfg!(target_os = "macos") {
        install_launch_agent(home, launcher, service_id, no_start)
    } else if cfg!(target_os = "linux") {
        install_systemd_user(home, launcher, service_id, no_start)
    } else {
        bail!("cowboy-machine supports only macOS and Linux")
    }
}

fn launcher_script(args: &InstallArgs, state: &Path, token: &Path) -> String {
    let mut command = vec![
        "--controller-url".to_owned(),
        args.controller_url.clone(),
        "--service-id".to_owned(),
        args.service_id.clone(),
    ];
    if let Some(machine_id) = &args.machine_id {
        command.extend(["--machine-id".to_owned(), machine_id.clone()]);
    }
    command.extend([
        "--state-dir".to_owned(),
        state.display().to_string(),
        "--workspace-config".to_owned(),
        state.join("config/workspaces.json").display().to_string(),
        "--socket".to_owned(),
        state.join(MACHINE_SOCKET).display().to_string(),
        "--provider-usage-socket".to_owned(),
        state.join(USAGE_SOCKET).display().to_string(),
        "--code-adapter-socket".to_owned(),
        state.join(CODE_SOCKET).display().to_string(),
        "--zed-adapter-socket".to_owned(),
        state.join(ZED_SOCKET).display().to_string(),
        "--max-sessions".to_owned(),
        args.max_sessions.max(1).to_string(),
    ]);
    if !args.refresh {
        command.extend([
            "--enrollment-token-file".to_owned(),
            token.display().to_string(),
        ]);
    }
    if args.plugin_operation_admission {
        command.push("--plugin-operation-admission".to_owned());
    }
    if args.draining {
        command.push("--draining".to_owned());
    }
    if let Some(name) = &args.display_name {
        command.extend(["--display-name".to_owned(), name.clone()]);
    }
    if let Some(key) = &args.artifact_public_key {
        command.extend([
            "--artifact-public-key".to_owned(),
            key.display().to_string(),
        ]);
    }
    for workspace in &args.workspaces {
        command.extend(["--workspace".to_owned(), workspace.clone()]);
    }
    let mut script = "#!/bin/sh\nset -eu\n".to_owned();
    // The installer-owned bootstrap is the guard, even when an active signed
    // host is selected later. An older bootstrap lacking the diagnostic fails
    // closed. It also authenticates cached host selection without running it.
    // This refuses terminal state; it does not admit a reader release.
    let _ = writeln!(
        script,
        "{} --check-portable-session-deletion --state-dir {}{} >/dev/null",
        shell_quote(&state.join("bootstrap/cowboy-machine").display().to_string()),
        shell_quote(&state.display().to_string()),
        args.artifact_public_key
            .as_ref()
            .map(|key| format!(
                " --artifact-public-key {}",
                shell_quote(&key.display().to_string())
            ))
            .unwrap_or_default()
    );
    let _ = writeln!(
        script,
        "PATH={}:{}:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH; export PATH",
        shell_quote(&state.join("components/commands").display().to_string()),
        shell_quote(&state.join("bootstrap").display().to_string())
    );
    for detect in crate::plugin_runtime_args::path_detect() {
        let cmd = crate::plugin_runtime_args::acp_env_key(detect.plugin_id, "CMD");
        if let Some(args) = &detect.args {
            let args_key = crate::plugin_runtime_args::acp_env_key(detect.plugin_id, "ARGS");
            let _ = writeln!(
                script,
                "if command -v {} >/dev/null 2>&1; then {cmd}=$(command -v {}); {args_key}={}; export {cmd} {args_key}; fi",
                detect.command,
                detect.command,
                shell_quote(args)
            );
        } else {
            let _ = writeln!(
                script,
                "if command -v {} >/dev/null 2>&1; then {cmd}=$(command -v {}); export {cmd}; fi",
                detect.command, detect.command
            );
        }
    }
    let _ = writeln!(
        script,
        "mkdir -p {}",
        shell_quote(&state.join("run").display().to_string())
    );
    let active = state.join("components/commands/cowboy-machine");
    let bootstrap = state.join("bootstrap/cowboy-machine");
    let _ = writeln!(
        script,
        "machine={}; [ -x {} ] && machine={}",
        shell_quote(&bootstrap.display().to_string()),
        shell_quote(&active.display().to_string()),
        shell_quote(&active.display().to_string())
    );
    script.push_str("exec \"$machine\"");
    for argument in command {
        script.push(' ');
        script.push_str(&shell_quote(&argument));
    }
    script.push('\n');
    script
}

#[cfg(all(test, feature = "machine-host"))]
pub(crate) fn portable_cache_launcher_fixture(state: &Path, key: &Path) -> String {
    let args = InstallArgs::try_parse_from([
        "installer",
        "--controller-url",
        "https://example.invalid",
        "--service-id",
        "svc-0123456789abcdef0123456789abcdef",
        "--workspace",
        "fixture=/tmp",
        "--refresh",
        "--artifact-public-key",
        key.to_str().unwrap(),
    ])
    .unwrap();
    launcher_script(&args, state, &state.join("token"))
}

fn install_systemd_user(
    home: &Path,
    launcher: &Path,
    service_id: &str,
    no_start: bool,
) -> Result<()> {
    let unit_dir = home.join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit = format!(
        "[Unit]\nDescription=Cowboy Machine\nAfter=network-online.target\n\n[Service]\nExecStart={}\nRestart=on-failure\nRestartSec=2\nSuccessExitStatus=75\n\n[Install]\nWantedBy=default.target\n",
        launcher.display()
    );
    let unit_name = if service_id.is_empty() {
        "cowboy-machine.service".to_owned()
    } else {
        format!("cowboy-machine-{service_id}.service")
    };
    std::fs::write(unit_dir.join(&unit_name), unit)?;
    if !no_start {
        checked("systemctl", &["--user", "daemon-reload"])?;
        checked("systemctl", &["--user", "enable", &unit_name])?;
        checked("systemctl", &["--user", "restart", &unit_name])?;
    }
    Ok(())
}

fn install_launch_agent(
    home: &Path,
    launcher: &Path,
    service_id: &str,
    no_start: bool,
) -> Result<()> {
    let agent_dir = home.join("Library/LaunchAgents");
    std::fs::create_dir_all(&agent_dir)?;
    let label = if service_id.is_empty() {
        "xyz.stormbird.cowboy-machine".to_owned()
    } else {
        format!("xyz.stormbird.cowboy-machine.{service_id}")
    };
    let plist = launch_agent_plist(&label, launcher);
    let path = agent_dir.join(format!("{label}.plist"));
    std::fs::write(&path, plist)?;
    if !no_start {
        let domain = format!("gui/{}", unsafe_uid());
        let _ = std::process::Command::new("launchctl")
            .args(["bootout", &domain, path.to_str().unwrap_or_default()])
            .status();
        checked(
            "launchctl",
            &[
                "bootstrap",
                &domain,
                path.to_str().context("plist path is not UTF-8")?,
            ],
        )?;
    }
    Ok(())
}

fn launch_agent_plist(label: &str, launcher: &Path) -> String {
    // launchd opens StandardOutPath once and every worker/provider descendant
    // inherits that descriptor. It has no retention policy, so a retry defect
    // can otherwise keep an unlinked multi-gigabyte file alive indefinitely.
    // Runtime health is reported through the controller; operators can run the
    // launcher in the foreground when raw stderr is required for diagnosis.
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array><string>{}</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict><key>ThrottleInterval</key><integer>2</integer><key>StandardOutPath</key><string>/dev/null</string><key>StandardErrorPath</key><string>/dev/null</string></dict></plist>\n",
        xml_escape(label),
        xml_escape(&launcher.display().to_string())
    )
}

#[cfg(unix)]
fn unsafe_uid() -> u32 {
    // `id -u` avoids adding a libc dependency to the stable installer.
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|value| String::from_utf8(value.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn checked(command: &str, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new(command).args(args).status()?;
    if !status.success() {
        bail!("{command} exited with {status}");
    }
    Ok(())
}

fn validate_scalar(value: &str) -> Result<()> {
    if value.contains(['\n', '\r', '\0']) {
        bail!("configuration values may not contain control characters");
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refresh_args(state: &Path, source: &Path) -> InstallArgs {
        InstallArgs::try_parse_from([
            "installer",
            "--controller-url",
            "https://cowboy.example",
            "--service-id",
            "svc-0123456789abcdef0123456789abcdef",
            "--workspace",
            "home=/work",
            "--refresh",
            "--no-start",
            "--plugin-operation-admission",
            "--state-dir",
            state.to_str().unwrap(),
            "--machine-binary",
            source.to_str().unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn floor_refuses_refresh_before_bootstrap_identity_or_launcher_changes() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(state.join("bootstrap")).unwrap();
        std::fs::write(
            state.join(crate::session_deletion_admission::reader_floor::NAME),
            b"malformed floor",
        )
        .unwrap();
        std::fs::write(state.join("bootstrap/cowboy-machine"), b"retained host").unwrap();
        std::fs::write(state.join("identity_ed25519"), b"retained identity").unwrap();
        let args = refresh_args(&state, &root.path().join("missing-source"));
        assert!(
            prepare_install_at(&args, root.path())
                .unwrap_err()
                .to_string()
                .contains("portable reader floor")
        );
        assert_eq!(
            std::fs::read(state.join("bootstrap/cowboy-machine")).unwrap(),
            b"retained host"
        );
        assert_eq!(
            std::fs::read(state.join("identity_ed25519")).unwrap(),
            b"retained identity"
        );
        assert!(!root.path().join(".local/bin").exists());
        assert!(!state.join("service-origin").exists());
    }

    #[test]
    fn probe_created_floor_refuses_install_before_publication() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let source = root.path().join("cowboy-machine");
        let script = format!(
            "#!/bin/sh\nprintf '{{}}' > '{}/portable-session-deletion-reader-floor.json'\nif [ -e \"$3/portable-session-deletion-reader-floor.json\" ]; then printf '%s' 'portable reader floor' >&2; exit 1; fi\nif [ -e \"$3/session-deletions/deletions.json\" ]; then printf '%s' 'portable Session deletion reader admission' >&2; exit 1; fi\nprintf '%s' '{{\"admitted\":true,\"writer\":false,\"host_cache_guard\":2}}'\n",
            state.display()
        );
        std::fs::write(&source, script).unwrap();
        set_mode(&source, 0o755).unwrap();
        for name in ["cowboy-code-adapter", "cowboy-acp-worker"] {
            std::fs::write(root.path().join(name), "fixture").unwrap();
        }
        let args = InstallArgs::try_parse_from([
            "installer",
            "--controller-url",
            "https://cowboy.example",
            "--service-id",
            "svc-0123456789abcdef0123456789abcdef",
            "--workspace",
            "main=/tmp",
            "--enrollment-token",
            "fixture",
            "--no-start",
            "--state-dir",
            state.to_str().unwrap(),
            "--machine-binary",
            source.to_str().unwrap(),
        ])
        .unwrap();
        assert!(
            prepare_install_at(&args, root.path())
                .unwrap_err()
                .to_string()
                .contains("portable reader floor")
        );
        assert!(!state.join("bootstrap").exists());
        assert!(!state.join("service-origin").exists());
        assert!(!root.path().join(".local/bin").exists());
        assert_eq!(
            std::fs::read(state.join(crate::session_deletion_admission::reader_floor::NAME))
                .unwrap(),
            b"{}"
        );
    }

    #[test]
    fn terminal_journal_refuses_refresh_before_bootstrap_or_launcher_changes() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(state.join("session-deletions")).unwrap();
        std::fs::create_dir_all(state.join("bootstrap")).unwrap();
        std::fs::write(state.join("session-deletions/deletions.json"), "{}").unwrap();
        std::fs::write(state.join("bootstrap/cowboy-machine"), "retained host").unwrap();
        std::fs::write(state.join("identity_ed25519"), "retained identity").unwrap();
        let args = refresh_args(&state, &root.path().join("missing-source"));
        let error = prepare_install_at(&args, root.path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("portable Session deletion reader admission")
        );
        assert_eq!(
            std::fs::read(state.join("bootstrap/cowboy-machine")).unwrap(),
            b"retained host"
        );
        assert_eq!(
            std::fs::read(state.join("identity_ed25519")).unwrap(),
            b"retained identity"
        );
        assert!(!root.path().join(".local/bin").exists());
        assert!(!state.join("service-origin").exists());
    }

    #[test]
    fn install_and_refresh_copy_the_probed_bundle_when_sources_are_replaced() {
        for refresh in [false, true] {
            let root = tempfile::tempdir_in("/tmp").unwrap();
            let state = root.path().join("state");
            let source = root.path().join("bundle");
            std::fs::create_dir(&source).unwrap();
            let probed_path = root.path().join("probed-path");
            let mut script = String::from(
                "#!/bin/sh\nif [ -e \"$3/session-deletions/deletions.json\" ]; then\n",
            );
            // Replace caller paths during the final diagnostic, after its
            // executable was loaded. A successful probe must not publish them.
            for name in bootstrap_probe::PAYLOADS {
                writeln!(
                    script,
                    "printf '%s' {} > {}",
                    shell_quote("#!/bin/sh\nexit 2\n"),
                    shell_quote(&source.join(name).display().to_string()),
                )
                .unwrap();
            }
            script.push_str("fi\n");
            writeln!(
                script,
                "printf '%s' \"$0\" > {}",
                shell_quote(&probed_path.display().to_string())
            )
            .unwrap();
            script.push_str("if [ -e \"$3/portable-session-deletion-reader-floor.json\" ]; then printf '%s' 'portable reader floor' >&2; exit 1; fi\nif [ -e \"$3/session-deletions/deletions.json\" ]; then printf '%s' 'portable Session deletion reader admission' >&2; exit 1; fi\nprintf '%s' '{\"admitted\":true,\"writer\":false,\"host_cache_guard\":2}'\n");
            for name in bootstrap_probe::PAYLOADS {
                std::fs::write(source.join(name), name).unwrap();
            }
            std::fs::write(source.join("cowboy-machine"), &script).unwrap();
            set_mode(&source.join("cowboy-machine"), 0o755).unwrap();
            let mut args = refresh_args(&state, &source.join("cowboy-machine"));
            if refresh {
                std::fs::create_dir(&state).unwrap();
                std::fs::write(state.join("identity_ed25519"), "retained key").unwrap();
                std::fs::write(state.join("machine-id"), "fixture").unwrap();
                std::fs::write(state.join("enrollment-token"), "retained token").unwrap();
                std::fs::write(state.join("service-origin"), "https://cowboy.example").unwrap();
            } else {
                args.refresh = false;
                args.enrollment_token = Some("new token".to_owned());
            }
            prepare_install_at(&args, root.path()).unwrap();
            for name in bootstrap_probe::PAYLOADS {
                let expected = if name == "cowboy-machine" {
                    script.as_bytes()
                } else {
                    name.as_bytes()
                };
                assert_eq!(
                    std::fs::read(state.join("bootstrap").join(name)).unwrap(),
                    expected
                );
                assert_eq!(
                    std::fs::read(source.join(name)).unwrap(),
                    b"#!/bin/sh\nexit 2\n"
                );
            }
            if refresh {
                assert_eq!(
                    std::fs::read(state.join("identity_ed25519")).unwrap(),
                    b"retained key"
                );
                assert_eq!(
                    std::fs::read(state.join("enrollment-token")).unwrap(),
                    b"retained token"
                );
            }
            let probed = std::fs::read_to_string(probed_path).unwrap();
            assert!(
                !Path::new(&probed).exists(),
                "private bundle was not cleaned up"
            );
        }
    }

    #[test]
    fn refresh_preserves_identity_token_and_signed_commands() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("state");
        let bundle = root.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(state.join("components/commands")).unwrap();
        for name in ["cowboy-machine", "cowboy-code-adapter", "cowboy-acp-worker"] {
            std::fs::write(bundle.join(name), name).unwrap();
        }
        std::fs::write(bundle.join("cowboy-machine"), "#!/bin/sh\nif [ -e \"$3/portable-session-deletion-reader-floor.json\" ]; then printf '%s' 'portable reader floor' >&2; exit 1; fi\nif [ -e \"$3/session-deletions/deletions.json\" ]; then printf '%s' 'portable Session deletion reader admission' >&2; exit 1; fi\nprintf '%s' '{\"admitted\":true,\"writer\":false,\"host_cache_guard\":2}'\n").unwrap();
        set_mode(&bundle.join("cowboy-machine"), 0o755).unwrap();
        std::fs::write(state.join("identity_ed25519"), "existing private key").unwrap();
        std::fs::write(state.join("machine-id"), "mac").unwrap();
        std::fs::write(state.join("service-origin"), "https://cowboy.example\n").unwrap();
        std::fs::write(state.join("enrollment-token"), "do not replace").unwrap();
        std::fs::write(
            state.join("components/commands/cowboy-machine"),
            "signed host",
        )
        .unwrap();
        let args = refresh_args(&state, &bundle.join("cowboy-machine"));
        let (_, launcher) = prepare_install_at(&args, root.path()).unwrap();
        let script = std::fs::read_to_string(&launcher).unwrap();
        assert!(!script.contains("--enrollment-token-file"));
        assert!(script.contains("--plugin-operation-admission"));
        assert_eq!(
            std::fs::read_to_string(state.join("identity_ed25519")).unwrap(),
            "existing private key"
        );
        assert_eq!(
            std::fs::read_to_string(state.join("enrollment-token")).unwrap(),
            "do not replace"
        );
        assert_eq!(
            std::fs::read_to_string(state.join("components/commands/cowboy-machine")).unwrap(),
            "signed host"
        );
        assert_eq!(
            std::fs::read_to_string(state.join("bootstrap/cowboy-acp-worker")).unwrap(),
            "cowboy-acp-worker"
        );
        let legacy = root.path().join(".local/bin/cowboy-machine-launch");
        let legacy_socket = state.join("run/cowboy-machine.sock");
        std::fs::rename(&launcher, &legacy).unwrap();
        std::fs::write(
            &legacy,
            script.replace(
                &shell_quote(&state.join(MACHINE_SOCKET).display().to_string()),
                &shell_quote(&legacy_socket.display().to_string()),
            ),
        )
        .unwrap();
        std::fs::remove_file(state.join("service-origin")).unwrap();
        let (_, refreshed) = prepare_install_at(&args, root.path()).unwrap();
        assert_eq!(refreshed, legacy);
        assert!(
            std::fs::read_to_string(refreshed)
                .unwrap()
                .contains(legacy_socket.to_str().unwrap())
        );
        assert!(!launcher.exists());
        let mut wrong = args;
        wrong.machine_id = Some("another-machine".to_owned());
        assert!(prepare_install_at(&wrong, root.path()).is_err());
        wrong.machine_id = None;
        wrong.controller_url = "https://other.example".to_owned();
        assert!(prepare_install_at(&wrong, root.path()).is_err());
    }

    #[test]
    fn incompatible_bootstrap_preserves_the_existing_installation() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("state");
        let bundle = root.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(state.join("bootstrap")).unwrap();
        for name in ["cowboy-machine", "cowboy-code-adapter", "cowboy-acp-worker"] {
            std::fs::write(bundle.join(name), "candidate").unwrap();
            std::fs::write(state.join("bootstrap").join(name), "retained").unwrap();
        }
        std::fs::write(bundle.join("cowboy-machine"), "#!/bin/sh\nexit 2\n").unwrap();
        set_mode(&bundle.join("cowboy-machine"), 0o755).unwrap();
        for (name, bytes) in [
            ("identity_ed25519", "retained identity"),
            ("machine-id", "fixture"),
            ("service-origin", "https://cowboy.example"),
            ("enrollment-token", "retained token"),
        ] {
            std::fs::write(state.join(name), bytes).unwrap();
        }
        let args = refresh_args(&state, &bundle.join("cowboy-machine"));
        let error = prepare_install_at(&args, root.path()).unwrap_err();
        assert!(error.to_string().contains("bootstrap must support"));
        for name in ["cowboy-machine", "cowboy-code-adapter", "cowboy-acp-worker"] {
            assert_eq!(
                std::fs::read(state.join("bootstrap").join(name)).unwrap(),
                b"retained"
            );
        }
        assert_eq!(
            std::fs::read(state.join("identity_ed25519")).unwrap(),
            b"retained identity"
        );
        assert_eq!(
            std::fs::read(state.join("enrollment-token")).unwrap(),
            b"retained token"
        );
        assert!(!root.path().join(".local/bin").exists());
        assert!(!root.path().join(".config").exists());
    }

    #[test]
    fn refresh_requires_enrollment_and_a_complete_bundle_before_writing() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let state = root.path().join("state");
        let args = refresh_args(&state, &root.path().join("cowboy-machine"));
        assert!(prepare_install_at(&args, root.path()).is_err());
        assert!(!state.exists());
        std::fs::create_dir(&state).unwrap();
        std::fs::write(state.join("identity_ed25519"), "key").unwrap();
        std::fs::write(state.join("machine-id"), "mac").unwrap();
        std::fs::write(state.join("service-origin"), "https://cowboy.example").unwrap();
        assert!(prepare_install_at(&args, root.path()).is_err());
        assert!(!state.join("bootstrap").exists());
    }

    #[test]
    fn installer_requires_exactly_one_enrollment_mode() {
        let base = [
            "installer",
            "--controller-url",
            "https://cowboy.example",
            "--service-id",
            "svc-0123456789abcdef0123456789abcdef",
            "--workspace",
            "home=/work",
        ];
        assert!(InstallArgs::try_parse_from(base).is_err());
        let mut enroll = base.to_vec();
        enroll.extend(["--enrollment-token", "one-time"]);
        let args = InstallArgs::try_parse_from(&enroll).unwrap();
        assert!(!args.plugin_operation_admission);
        enroll.push("--refresh");
        assert!(InstallArgs::try_parse_from(enroll).is_err());
    }

    #[test]
    fn signed_bootstrap_cli_requires_package_and_publisher_and_excludes_caller_binary() {
        let base = [
            "installer",
            "--controller-url",
            "https://cowboy.example",
            "--service-id",
            "svc-0123456789abcdef0123456789abcdef",
            "--workspace",
            "main=/work",
            "--refresh",
        ];
        let mut args = base.to_vec();
        args.extend(["--bootstrap-manifest", "/manifest"]);
        assert!(InstallArgs::try_parse_from(&args).is_err());
        args.extend(["--bootstrap-artifact", "/artifact"]);
        assert!(InstallArgs::try_parse_from(&args).is_err());
        args.extend(["--artifact-public-key", "/publisher"]);
        assert!(InstallArgs::try_parse_from(&args).is_ok());
        args.extend(["--machine-binary", "/unsigned"]);
        assert!(InstallArgs::try_parse_from(&args).is_err());
        let mut args = base.to_vec();
        args.extend(["--bootstrap-artifact", "/artifact"]);
        assert!(InstallArgs::try_parse_from(&args).is_err());
    }

    #[test]
    fn launcher_prefers_the_active_signed_generation() {
        let args = InstallArgs {
            controller_url: "https://cowboy.example".to_owned(),
            service_id: "svc-0123456789abcdef0123456789abcdef".to_owned(),
            machine_id: Some("mac".to_owned()),
            display_name: None,
            workspaces: vec!["main=/work/main".to_owned()],
            enrollment_token: Some("secret".to_owned()),
            refresh: false,
            plugin_operation_admission: true,
            artifact_public_key: None,
            machine_binary: None,
            bootstrap_manifest: None,
            bootstrap_artifact: None,
            state_dir: None,
            max_sessions: 8,
            draining: false,
            no_start: true,
        };
        let script = launcher_script(&args, Path::new("/state"), Path::new("/state/token"));
        assert!(script.contains("components/commands/cowboy-machine"));
        assert!(script.contains("'/state/run/m'"));
        assert!(script.contains("'/state/run/u'"));
        assert!(!script.contains("agentd"));
        assert!(script.contains("/opt/homebrew/bin"));
        assert!(script.contains("--enrollment-token-file"));
        assert!(script.contains("--machine-id"));
        assert!(script.contains("--plugin-operation-admission"));
        assert!(script.contains("'/state/bootstrap':"));
        assert!(script.contains("COWBOY_ACP_GROK_CMD"));
        assert!(script.contains("--experimental-memory --rules"));
        assert!(script.contains("Read and follow the closest AGENTS.md"));
        assert!(!script.contains("secret"));
    }

    #[test]
    fn launcher_omits_machine_id_when_unassigned() {
        let args = InstallArgs {
            controller_url: "https://cowboy.example".to_owned(),
            service_id: "svc-0123456789abcdef0123456789abcdef".to_owned(),
            machine_id: None,
            display_name: None,
            workspaces: vec!["home=/home/me".to_owned()],
            enrollment_token: Some("secret".to_owned()),
            refresh: false,
            plugin_operation_admission: false,
            artifact_public_key: None,
            machine_binary: None,
            bootstrap_manifest: None,
            bootstrap_artifact: None,
            state_dir: None,
            max_sessions: 8,
            draining: false,
            no_start: true,
        };
        let script = launcher_script(&args, Path::new("/state"), Path::new("/state/token"));
        assert!(!script.contains("--machine-id"));
    }

    #[test]
    fn service_scoped_default_sockets_fit_linux_and_macos() {
        for home in ["/home/ubuntu", "/Users/draven", "/Users/longusername"] {
            let state = crate::service_identity::service_state_dir(
                Path::new(home),
                "svc-0123456789abcdef0123456789abcdef",
            )
            .expect("Service state path");
            validate_socket_paths(&state).expect("default sockets fit sun_path");
        }
        let too_long = PathBuf::from(format!("/{}", "x".repeat(98)));
        assert!(validate_socket_paths(&too_long).is_err());
        let multibyte = PathBuf::from(format!("/{}", "é".repeat(49)));
        assert!(validate_socket_paths(&multibyte).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn compact_sockets_bind_in_a_long_service_directory() {
        use std::os::unix::net::UnixListener;

        let temporary = tempfile::tempdir_in("/tmp").expect("temporary directory");
        let prefix_bytes = temporary.path().as_os_str().as_encoded_bytes().len();
        // Reproduce the original 110-byte Ubuntu socket path with a private
        // temporary parent, then prove all generated replacements really bind.
        let state = temporary.path().join("s".repeat(86 - prefix_bytes - 1));
        std::fs::create_dir_all(state.join("run")).expect("runtime directory");
        assert!(UnixListener::bind(state.join("run/provider-usage.sock")).is_err());
        for relative in [MACHINE_SOCKET, USAGE_SOCKET, CODE_SOCKET, ZED_SOCKET] {
            let listener = UnixListener::bind(state.join(relative)).expect("bind compact socket");
            drop(listener);
        }
    }

    #[test]
    fn launch_agent_does_not_create_an_unbounded_log_file() {
        let plist = launch_agent_plist(
            "xyz.stormbird.cowboy-machine.svc-test",
            Path::new("/Users/test/.local/bin/cowboy-machine-launch-svc-test"),
        );
        assert_eq!(plist.matches("<string>/dev/null</string>").count(), 2);
        assert!(!plist.contains("Library/Logs"));
    }

    #[test]
    fn register_rejects_cleartext_remote_origins() {
        assert!(normalize_controller_url("https://cowboy.example").is_ok());
        assert!(normalize_controller_url("http://127.0.0.1:3333").is_ok());
        assert!(normalize_controller_url("http://cowboy.example").is_err());
    }

    #[test]
    fn code_adapter_is_resolved_next_to_the_machine_host() {
        assert_eq!(
            companion_binary(
                Path::new("/release/bin/cowboy-machine"),
                "cowboy-code-adapter"
            ),
            Path::new("/release/bin/cowboy-code-adapter")
        );
    }
}

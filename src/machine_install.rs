//! User-scoped macOS/Linux installation for `cowboy-machine`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::Parser;

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
    #[arg(long)]
    enrollment_token: String,
    #[arg(long)]
    artifact_public_key: Option<PathBuf>,
    #[arg(long)]
    machine_binary: Option<PathBuf>,
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
    bind_service_origin(&state_dir, &controller_url)?;
    let host = machine_host_binary(None);
    anyhow::ensure!(
        host.is_file(),
        "cowboy-machine was not found next to this cowboy binary ({host}). Install both on this computer, then run register again.",
        host = host.display()
    );
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
        enrollment_token: token.trim().to_owned(),
        artifact_public_key: None,
        machine_binary: None,
        state_dir: Some(state_dir.clone()),
        max_sessions: 8,
        draining: false,
        no_start: false,
    };
    let (home, launcher) = prepare_install(&install_args)?;
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
    let response = reqwest::Client::new()
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
    let (home, launcher) = prepare_install(&args)?;
    install_background_service(&home, &launcher, &args.service_id, args.no_start)
}

fn prepare_install(args: &InstallArgs) -> Result<(PathBuf, PathBuf)> {
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
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let state = args.state_dir.clone().map_or_else(
        || crate::service_identity::service_state_dir(&home, &args.service_id),
        Ok,
    )?;
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

    let source = machine_host_binary(args.machine_binary.as_deref());
    anyhow::ensure!(
        source.is_file(),
        "cowboy-machine was not found next to this cowboy binary ({})",
        source.display()
    );
    let code_adapter_source = companion_binary(&source, "cowboy-code-adapter");
    anyhow::ensure!(
        code_adapter_source.is_file(),
        "cowboy-code-adapter was not found next to the Machine host ({})",
        code_adapter_source.display()
    );
    let bootstrap = state.join("bootstrap/cowboy-machine");
    std::fs::copy(&source, &bootstrap)
        .with_context(|| format!("copying Machine host from {}", source.display()))?;
    set_mode(&bootstrap, 0o755)?;

    let code_adapter_bootstrap = state.join("bootstrap/cowboy-code-adapter");
    std::fs::copy(&code_adapter_source, &code_adapter_bootstrap).with_context(|| {
        format!(
            "copying Machine code adapter from {}",
            code_adapter_source.display()
        )
    })?;
    set_mode(&code_adapter_bootstrap, 0o755)?;

    let token = state.join("enrollment-token");
    std::fs::write(&token, &args.enrollment_token)?;
    set_mode(&token, 0o600)?;
    let launcher = runtime.join(format!("cowboy-machine-launch-{}", args.service_id));
    std::fs::write(&launcher, launcher_script(args, &state, &token))?;
    set_mode(&launcher, 0o755)?;

    Ok((home, launcher))
}

fn install_background_service(
    home: &Path,
    launcher: &Path,
    service_id: &str,
    no_start: bool,
) -> Result<()> {
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
        "--enrollment-token-file".to_owned(),
        token.display().to_string(),
        "--socket".to_owned(),
        state.join("run/cowboy-machine.sock").display().to_string(),
        "--provider-usage-socket".to_owned(),
        state.join("run/provider-usage.sock").display().to_string(),
        "--code-adapter-socket".to_owned(),
        state.join("run/code-adapter.sock").display().to_string(),
        "--zed-adapter-socket".to_owned(),
        state.join("run/zed-adapter.sock").display().to_string(),
        "--max-sessions".to_owned(),
        args.max_sessions.max(1).to_string(),
    ]);
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
    let _ = writeln!(
        script,
        "PATH={}:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH; export PATH",
        shell_quote(&state.join("components/commands").display().to_string())
    );
    script.push_str(
        "if command -v codex-acp >/dev/null 2>&1; then COWBOY_ACP_CODEX_CMD=$(command -v codex-acp); export COWBOY_ACP_CODEX_CMD; fi\n",
    );
    script.push_str(
        "if command -v claude-agent-acp >/dev/null 2>&1; then COWBOY_ACP_CLAUDE_CODE_CMD=$(command -v claude-agent-acp); export COWBOY_ACP_CLAUDE_CODE_CMD; fi\n",
    );
    script.push_str(
        "if command -v gemini >/dev/null 2>&1; then COWBOY_ACP_GEMINI_CMD=$(command -v gemini); COWBOY_ACP_GEMINI_ARGS=--acp; export COWBOY_ACP_GEMINI_CMD COWBOY_ACP_GEMINI_ARGS; fi\n",
    );
    let _ = writeln!(
        script,
        "if command -v grok >/dev/null 2>&1; then COWBOY_ACP_GROK_CMD=$(command -v grok); COWBOY_ACP_GROK_ARGS={}; export COWBOY_ACP_GROK_CMD COWBOY_ACP_GROK_ARGS; fi",
        shell_quote(crate::grok::RUNTIME_ARGS_ENV)
    );
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
    let unit_name = format!("cowboy-machine-{service_id}.service");
    std::fs::write(unit_dir.join(&unit_name), unit)?;
    if !no_start {
        checked("systemctl", &["--user", "daemon-reload"])?;
        checked("systemctl", &["--user", "enable", "--now", &unit_name])?;
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
    let label = format!("xyz.stormbird.cowboy-machine.{service_id}");
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

    #[test]
    fn launcher_prefers_the_active_signed_generation() {
        let args = InstallArgs {
            controller_url: "https://cowboy.example".to_owned(),
            service_id: "svc-0123456789abcdef0123456789abcdef".to_owned(),
            machine_id: Some("mac".to_owned()),
            display_name: None,
            workspaces: vec!["main=/work/main".to_owned()],
            enrollment_token: "secret".to_owned(),
            artifact_public_key: None,
            machine_binary: None,
            state_dir: None,
            max_sessions: 8,
            draining: false,
            no_start: true,
        };
        let script = launcher_script(&args, Path::new("/state"), Path::new("/state/token"));
        assert!(script.contains("components/commands/cowboy-machine"));
        assert!(script.contains("/state/run/cowboy-machine.sock"));
        assert!(script.contains("/state/run/provider-usage.sock"));
        assert!(!script.contains("agentd"));
        assert!(script.contains("/opt/homebrew/bin"));
        assert!(script.contains("--enrollment-token-file"));
        assert!(script.contains("--machine-id"));
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
            enrollment_token: "secret".to_owned(),
            artifact_public_key: None,
            machine_binary: None,
            state_dir: None,
            max_sessions: 8,
            draining: false,
            no_start: true,
        };
        let script = launcher_script(&args, Path::new("/state"), Path::new("/state/token"));
        assert!(!script.contains("--machine-id"));
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

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
    machine_id: String,
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

fn install(args: InstallArgs) -> Result<()> {
    validate_scalar(&args.controller_url)?;
    validate_scalar(&args.machine_id)?;
    for workspace in &args.workspaces {
        validate_scalar(workspace)?;
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let state = args
        .state_dir
        .clone()
        .unwrap_or_else(|| home.join(".local/state/cowboy-machine"));
    let config = home.join(".config/cowboy-machine");
    let runtime = home.join(".local/bin");
    std::fs::create_dir_all(state.join("bootstrap"))?;
    std::fs::create_dir_all(&config)?;
    std::fs::create_dir_all(&runtime)?;
    set_mode(&state, 0o700)?;
    set_mode(&config, 0o700)?;

    let source = args.machine_binary.clone().unwrap_or_else(|| {
        std::env::current_exe()
            .unwrap_or_else(|_| PathBuf::from("cowboy-machine-install"))
            .with_file_name("cowboy-machine")
    });
    let bootstrap = state.join("bootstrap/cowboy-machine");
    std::fs::copy(&source, &bootstrap)
        .with_context(|| format!("copying Machine host from {}", source.display()))?;
    set_mode(&bootstrap, 0o755)?;

    let token = state.join("enrollment-token");
    std::fs::write(&token, &args.enrollment_token)?;
    set_mode(&token, 0o600)?;
    let launcher = runtime.join("cowboy-machine-launch");
    std::fs::write(&launcher, launcher_script(&args, &state, &token))?;
    set_mode(&launcher, 0o755)?;

    if cfg!(target_os = "macos") {
        install_launch_agent(&home, &launcher, args.no_start)
    } else if cfg!(target_os = "linux") {
        install_systemd_user(&home, &launcher, args.no_start)
    } else {
        bail!("cowboy-machine supports only macOS and Linux")
    }
}

fn launcher_script(args: &InstallArgs, state: &Path, token: &Path) -> String {
    let mut command = vec![
        "--controller-url".to_owned(),
        args.controller_url.clone(),
        "--machine-id".to_owned(),
        args.machine_id.clone(),
        "--state-dir".to_owned(),
        state.display().to_string(),
        "--enrollment-token-file".to_owned(),
        token.display().to_string(),
        "--socket".to_owned(),
        state.join("run/cowboy-machine.sock").display().to_string(),
        "--code-adapter-socket".to_owned(),
        state.join("run/code-adapter.sock").display().to_string(),
        "--zed-adapter-socket".to_owned(),
        state.join("run/zed-adapter.sock").display().to_string(),
        "--max-sessions".to_owned(),
        args.max_sessions.max(1).to_string(),
    ];
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

fn install_systemd_user(home: &Path, launcher: &Path, no_start: bool) -> Result<()> {
    let unit_dir = home.join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit = format!(
        "[Unit]\nDescription=Cowboy Machine\nAfter=network-online.target\n\n[Service]\nExecStart={}\nRestart=on-failure\nRestartSec=2\nSuccessExitStatus=75\n\n[Install]\nWantedBy=default.target\n",
        launcher.display()
    );
    std::fs::write(unit_dir.join("cowboy-machine.service"), unit)?;
    if !no_start {
        checked("systemctl", &["--user", "daemon-reload"])?;
        checked(
            "systemctl",
            &["--user", "enable", "--now", "cowboy-machine.service"],
        )?;
    }
    Ok(())
}

fn install_launch_agent(home: &Path, launcher: &Path, no_start: bool) -> Result<()> {
    let agent_dir = home.join("Library/LaunchAgents");
    std::fs::create_dir_all(&agent_dir)?;
    let label = "xyz.stormbird.cowboy-machine";
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array><string>{}</string></array><key>RunAtLoad</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict><key>ThrottleInterval</key><integer>2</integer><key>StandardOutPath</key><string>{}</string><key>StandardErrorPath</key><string>{}</string></dict></plist>\n",
        xml_escape(&launcher.display().to_string()),
        xml_escape(
            &home
                .join("Library/Logs/cowboy-machine.log")
                .display()
                .to_string()
        ),
        xml_escape(
            &home
                .join("Library/Logs/cowboy-machine.log")
                .display()
                .to_string()
        )
    );
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
            machine_id: "mac".to_owned(),
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
        assert!(!script.contains("agentd"));
        assert!(script.contains("/opt/homebrew/bin"));
        assert!(script.contains("--enrollment-token-file"));
        assert!(!script.contains("secret"));
    }
}

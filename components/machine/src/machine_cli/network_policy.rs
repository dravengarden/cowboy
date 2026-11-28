//! Closed runtime network-policy wrapper (`ipv4_only_v1`) for local native
//! login and logout commands via transient `systemd-run --user` units.
use super::*;

pub(super) struct TransientUnitGuard(Option<String>);

impl TransientUnitGuard {
    pub(super) fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for TransientUnitGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(unit) = self.0.take() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "stop", "--no-block", &unit])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

pub(super) fn authentication_process_with_network(
    command: &Path,
    arguments: &[String],
    environment: &BTreeMap<String, String>,
    terminal: &cowboy_provider_sdk::AuthTerminal,
    home: &Path,
    network: Option<cowboy_provider_sdk::RuntimeNetworkPolicy>,
) -> (tokio::process::Command, TransientUnitGuard) {
    #[cfg(target_os = "linux")]
    if let Some(cowboy_provider_sdk::RuntimeNetworkPolicy::Ipv4OnlyV1) = network {
        let unit = format!("cowboy-auth-{}.service", uuid::Uuid::new_v4().simple());
        let mut process = tokio::process::Command::new("systemd-run");
        process.env_clear();
        for outer in ["PATH", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"] {
            if let Some(value) = std::env::var_os(outer) {
                process.env(outer, value);
            }
        }
        process.args([
            "--user",
            "--quiet",
            "--wait",
            "--pipe",
            "--collect",
            "--service-type=exec",
            "--property=KillMode=control-group",
            "--property=Restart=no",
            "--property=TimeoutStopSec=5s",
            "--property=RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK",
            &format!("--unit={unit}"),
            &format!("--working-directory={}", home.display()),
        ]);
        let mut unit_env = BTreeMap::new();
        for name in [
            "PATH",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NIX_SSL_CERT_FILE",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            if let Ok(value) = std::env::var(name) {
                unit_env.insert(name.to_owned(), value);
            }
        }
        for (key, value) in environment {
            unit_env.insert(key.clone(), value.clone());
        }
        unit_env.insert("HOME".to_owned(), home.display().to_string());
        unit_env.insert(
            "XDG_CONFIG_HOME".to_owned(),
            home.join(".config").display().to_string(),
        );
        unit_env.insert(
            "XDG_DATA_HOME".to_owned(),
            home.join(".local/share").display().to_string(),
        );
        unit_env.insert(
            "XDG_CACHE_HOME".to_owned(),
            home.join(".cache").display().to_string(),
        );
        unit_env.insert(
            "XDG_STATE_HOME".to_owned(),
            home.join(".local/state").display().to_string(),
        );
        unit_env.insert("TMPDIR".to_owned(), home.display().to_string());
        for (key, value) in unit_env {
            process.arg(format!("--setenv={key}={value}"));
        }
        process.arg("--");
        match terminal {
            cowboy_provider_sdk::AuthTerminal::Pipes => {
                process.arg(command);
                process.args(arguments);
            }
            cowboy_provider_sdk::AuthTerminal::Pty => {
                let command_line = std::iter::once(command.display().to_string())
                    .chain(arguments.iter().cloned())
                    .map(|value| shell_quote(&value))
                    .collect::<Vec<_>>()
                    .join(" ");
                process.args(["script", "-q", "-e", "-f", "-c", &command_line, "/dev/null"]);
            }
        }
        process.current_dir(home);
        {
            use std::os::unix::process::CommandExt as _;
            process.as_std_mut().process_group(0);
        }
        return (process, TransientUnitGuard(Some(unit)));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = network;
    (
        authentication_process(command, arguments, environment, terminal, home),
        TransientUnitGuard(None),
    )
}

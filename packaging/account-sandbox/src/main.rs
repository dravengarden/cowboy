//! `cowboy-account-sandbox`: the root daemon that runs each Antigravity
//! account's processes as its own static system identity in confined
//! transient systemd units (`docs/antigravity-real-accounts-design.md`), and
//! the `exec-confined` wrapper those units start through. The wire format and
//! configuration are documented in `packaging/account-sandbox/README.md`.

mod config;
mod confine;
mod daemon;
#[cfg(test)]
mod fake;
mod host;
mod ids;
mod meta;
mod protocol;
mod raw;
mod safefs;
mod server;
mod start;
mod store;
mod tar;
#[cfg(test)]
mod tests;
mod units;

use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};

const USAGE: &str = "usage: cowboy-account-sandbox serve [--config PATH] [--socket PATH]\n       cowboy-account-sandbox exec-confined -- PROGRAM [ARGS...]\n       cowboy-account-sandbox --version";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("--version") if arguments.len() == 1 => {
            println!(
                "cowboy-account-sandbox {} {}",
                env!("CARGO_PKG_VERSION"),
                protocol::PROTOCOL
            );
            ExitCode::SUCCESS
        }
        Some("exec-confined") => match confine::parse(&arguments[1..]) {
            Ok(argv) => {
                let error = confine::run(argv);
                eprintln!("cowboy-account-sandbox exec-confined: {error:#}");
                ExitCode::from(126)
            }
            Err(error) => {
                eprintln!("cowboy-account-sandbox: {error}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        Some("serve") => match serve(&arguments[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("cowboy-account-sandbox: {error:#}");
                ExitCode::from(1)
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn serve(arguments: &[String]) -> Result<()> {
    let mut config_path: Option<PathBuf> = None;
    let mut socket: Option<PathBuf> = None;
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        let value = rest
            .next()
            .with_context(|| format!("{argument} needs a value"));
        match argument.as_str() {
            "--config" => config_path = Some(value?.into()),
            "--socket" => socket = Some(value?.into()),
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }
    let config = match &config_path {
        Some(path) => config::Config::load(path, true)?,
        None => config::Config::load(std::path::Path::new(config::DEFAULT_CONFIG_PATH), false)?,
    };
    let listener = match (raw::take_listen_fd()?, socket) {
        (Some(fd), None) => UnixListener::from(fd),
        (None, Some(path)) => {
            let _ = std::fs::remove_file(&path);
            let listener =
                UnixListener::bind(&path).with_context(|| format!("bind {}", path.display()))?;
            rustix::fs::chmod(&path, rustix::fs::Mode::from_raw_mode(0o660))?;
            listener
        }
        (Some(_), Some(_)) => bail!("--socket conflicts with socket activation"),
        (None, None) => bail!("no socket: start through socket activation or pass --socket"),
    };
    if !rustix::process::geteuid().is_root() {
        bail!("serve must run as root");
    }
    let host = Arc::new(host::SystemHost {
        tools: config.tools.clone(),
    });
    let daemon = daemon::Daemon::new(config, host, host::var_run_is_directory())?;
    daemon.reconcile()?;
    server::serve(&daemon, &listener);
    Ok(())
}

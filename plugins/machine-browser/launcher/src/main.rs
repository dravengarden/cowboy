//! `cowboy-browser-launcher`: the static supervisor of one Machine browser
//! instance (`docs/machine-browser.md`, "Launcher contract"). The process
//! systemd starts is a thin outer supervisor; it forks the namespace init,
//! which builds the isolation, starts the display server, the window manager
//! and the browser under a sealed policy, serves the control socket and
//! copies profile snapshots on the Machine's request. The exact wire format
//! is documented in `plugins/machine-browser/PROTOCOL.md`.

mod args;
mod cgroup;
mod children;
mod commands;
mod control;
mod copier;
mod init;
mod mounts;
mod procinfo;
mod raw;
mod snapshot;
mod startup;
mod supervisor;

use std::process::ExitCode;

const USAGE: &str = "usage: cowboy-browser-launcher --probe\n       cowboy-browser-launcher run --control-dir DIR --runtime-dir DIR --staging-dir DIR --display :N --geometry WxH --scale N --profile-bytes N --instance-bytes N [--tmp-bytes N] --policy-file REL --policy-directory ABS --browser REL --display-server REL --window-manager REL --xrandr REL --xkbcomp REL --xauth REL [--snapshot-dir DIR] [--sealed-profile NAME] [--cgroup-dir DIR] [--mask PATH]...";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("--probe") if arguments.len() == 1 => {
            println!("{}", control::probe_line());
            ExitCode::SUCCESS
        }
        Some("run") => match args::Args::parse(&arguments[1..]) {
            Ok(args) => ExitCode::from(supervisor::run(args)),
            Err(error) => {
                eprintln!("cowboy-browser-launcher: {error}\n{USAGE}");
                ExitCode::from(2)
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

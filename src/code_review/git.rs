//! Review's Git reads start no program the repository configures.
//!
//! Repository configuration is writable by the session's agent, and Review
//! also reads contexts nobody executes in (other worktrees, project
//! checkouts). Git would otherwise start an fsmonitor daemon on every status,
//! hooks, signature verification, external diff and textconv drivers, and
//! clean/smudge/process filters named by `.gitattributes`. Review needs none
//! of them: it reports what Git stores, not what a project program renders.
//!
//! Filter drivers have no command-line switch, so worktree comparisons first
//! list the filter keys the repository configures (a pure config read) and
//! blank each one. A writer racing that read can still name a new driver for
//! the following command; submodule configuration is not inspected.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};

const GIT_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Overrides every invocation carries, in Git's unambiguous environment form.
const FIXED: [(&str, &str); 4] = [
    ("core.fsmonitor", "false"),
    ("core.hookspath", "/dev/null"),
    ("core.pager", "cat"),
    ("log.showsignature", "false"),
];

/// Subcommands that render diffs and so would run diff drivers.
const DIFF_RENDERING: [&str; 3] = ["diff", "show", "log"];

/// Subcommands that compare the working tree and so would run filters.
const WORKTREE_COMPARING: [&str; 2] = ["diff", "status"];

fn command(root: &Path, overrides: &[(String, String)]) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env(
            "GIT_CONFIG_COUNT",
            (FIXED.len() + overrides.len()).to_string(),
        );
    let all = FIXED.iter().map(|(key, value)| (*key, *value)).chain(
        overrides
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    for (index, (key, value)) in all.enumerate() {
        command
            .env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    command
}

/// Configured filter-driver keys, each blanked so Git treats the file as
/// unfiltered. Keys come from Git itself, including included files.
fn filter_overrides(root: &Path) -> Result<Vec<(String, String)>, String> {
    let output = command(root, &[])
        .args([
            "config",
            "--null",
            "--name-only",
            "--get-regexp",
            r"^filter\..+\.(clean|smudge|process)$",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("starting git: {error}"))?;
    // Exit 1 means no key matched.
    if !output.status.success() && output.status.code() != Some(1) {
        return Err("reading git configuration failed".to_owned());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|key| !key.is_empty())
        .map(|key| (String::from_utf8_lossy(key).into_owned(), String::new()))
        .collect())
}

/// A bounded, non-interactive fetch over the repository's configured transport.
/// Repository config is agent-writable, so `ext::` helpers stay refused.
pub(super) fn git_fetch(root: &Path, remote: &str, refspec: &str) -> Result<(), String> {
    let mut child = command(root, &[])
        .args(["fetch", "--quiet", "--no-tags", "--no-write-fetch-head"])
        // Without an empty refmap, Git also moves the remote-tracking ref.
        .args([
            "--refmap=",
            "--no-recurse-submodules",
            "--",
            remote,
            refspec,
        ])
        .env("GIT_ALLOW_PROTOCOL", "https:ssh:git:file")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("starting git: {error}"))?;
    let deadline = std::time::Instant::now() + GIT_FETCH_TIMEOUT;
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => return Err(format!("could not fetch from {remote}")),
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("fetching from {remote} timed out"));
            }
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

pub(super) fn git_output(root: &Path, args: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    let subcommand = args.first().copied().unwrap_or_default();
    let overrides = if WORKTREE_COMPARING.contains(&subcommand) {
        filter_overrides(root)?
    } else {
        Vec::new()
    };
    let mut command = command(root, &overrides);
    // Review paths identify exact files, including names such as [id].tsx
    // or :(glob)*.txt. They are never Git pathspec patterns or magic.
    command.arg("--literal-pathspecs");
    if DIFF_RENDERING.contains(&subcommand) {
        command
            .arg(subcommand)
            .args(["--no-ext-diff", "--no-textconv"])
            .args(&args[1..]);
    } else {
        command.args(args);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("starting git: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("missing git stdout")?;
    let mut bytes = Vec::new();
    stdout
        .by_ref()
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading git output: {error}"))?;
    drop(stdout);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("waiting for git: {error}"))?;
    if output.status.success() || bytes.len() == limit {
        Ok(bytes)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

pub(super) fn git_path_is_tracked(root: &Path, path: &str) -> bool {
    git_output(root, &["ls-files", "--error-unmatch", "--", path], 1024).is_ok()
}

#[cfg(test)]
mod tests;

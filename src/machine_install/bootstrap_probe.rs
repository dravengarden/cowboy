//! Check the trusted caller-selected bootstrap before replacing an installation.

use std::fs::{DirBuilder, File, OpenOptions};
use std::io::Read as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "cowboy-bootstrap-probe-{:032x}",
            rand::random::<u128>()
        ));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Probe(Child);

impl Drop for Probe {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        if let Some(group) = rustix::process::Pid::from_raw(self.0.id() as i32) {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bounded_output(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(4097).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 4096,
        "bootstrap guard probe output exceeds limit"
    );
    Ok(bytes)
}

pub(super) fn check(source: &Path) -> Result<()> {
    check_with_timeout(source, Duration::from_secs(5))
        .context("bootstrap must support the portable Session deletion guard before installation")
}

fn check_with_timeout(source: &Path, timeout: Duration) -> Result<()> {
    let source = source
        .canonicalize()
        .context("resolving bootstrap probe executable")?;
    let scratch = Scratch::new()?;
    let state = scratch.0.join("state");
    for committed in [false, true] {
        if committed {
            let journal = state.join("session-deletions");
            std::fs::create_dir_all(&journal)?;
            std::fs::write(journal.join("deletions.json"), b"{}")?;
        }
        let stdout_path = scratch.0.join(format!("stdout-{committed}"));
        let stderr_path = scratch.0.join(format!("stderr-{committed}"));
        let output_file = |path: &Path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
        };
        let mut probe = Probe(
            Command::new(&source)
                .env_clear()
                .env("HOME", scratch.0.join("home"))
                .env("XDG_CONFIG_HOME", scratch.0.join("config"))
                .env("XDG_CACHE_HOME", scratch.0.join("cache"))
                .env("XDG_DATA_HOME", scratch.0.join("data"))
                .arg("--check-portable-session-deletion")
                .arg("--state-dir")
                .arg(&state)
                .current_dir(&scratch.0)
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(output_file(&stdout_path)?)
                .stderr(output_file(&stderr_path)?)
                .spawn()
                .context("starting offline bootstrap guard probe")?,
        );
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = probe.0.try_wait()? {
                break status;
            }
            ensure!(Instant::now() < deadline, "bootstrap guard probe timed out");
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = bounded_output(&stdout_path)?;
        let stderr = bounded_output(&stderr_path)?;
        if committed {
            ensure!(
                status.code() == Some(1),
                "bootstrap did not refuse committed terminal state"
            );
            ensure!(
                String::from_utf8_lossy(&stderr)
                    .contains("portable Session deletion reader admission"),
                "bootstrap returned the wrong terminal-state refusal"
            );
            ensure!(
                bounded_output(&state.join("session-deletions/deletions.json"))? == b"{}",
                "bootstrap probe changed the committed fixture"
            );
            ensure!(
                std::fs::read_dir(&state)?.take(2).count() == 1,
                "bootstrap guard probe opened Machine stores"
            );
            ensure!(
                std::fs::read_dir(state.join("session-deletions"))?
                    .take(2)
                    .count()
                    == 1,
                "bootstrap guard probe changed the deletion namespace"
            );
        } else {
            ensure!(
                status.success(),
                "bootstrap empty-state guard failed: {}",
                String::from_utf8_lossy(&stderr)
            );
            let report: serde_json::Value =
                serde_json::from_slice(&stdout).context("decoding bootstrap guard report")?;
            ensure!(
                report == serde_json::json!({"admitted": true, "writer": false}),
                "bootstrap guard report is incompatible"
            );
            ensure!(
                matches!(std::fs::symlink_metadata(&state), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
                "bootstrap empty-state probe created Machine state"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn script(root: &Path, contents: &str) -> PathBuf {
        let path = root.join("candidate");
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn empty_success_alone_cannot_admit_a_bootstrap() {
        let root = tempfile::tempdir().unwrap();
        let source = script(
            root.path(),
            "#!/bin/sh\nprintf '%s' '{\"admitted\":true,\"writer\":false}'\n",
        );
        assert!(
            check(&source)
                .unwrap_err()
                .root_cause()
                .to_string()
                .contains("did not refuse committed")
        );
    }

    #[test]
    fn hanging_probe_is_killed_and_reaped() {
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("pid");
        let source = script(
            root.path(),
            &format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nwhile :; do :; done\n",
                pid_file.display()
            ),
        );
        let error = check_with_timeout(&source, Duration::from_millis(200)).unwrap_err();
        assert!(error.to_string().contains("timed out"));
        let pid = std::fs::read_to_string(pid_file)
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let group = rustix::process::Pid::from_raw(pid).unwrap();
        let alive = rustix::process::test_kill_process_group(group);
        if alive.is_ok() {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            let _ = rustix::process::waitpid(Some(group), rustix::process::WaitOptions::empty());
        }
        assert_eq!(
            alive,
            Err(rustix::io::Errno::SRCH),
            "timed-out probe survived"
        );
    }
}

//! Check the trusted caller-selected bootstrap before replacing an installation.

use std::fs::{DirBuilder, File, OpenOptions};
use std::io::Read as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};

pub(super) const PAYLOADS: [&str; 3] =
    ["cowboy-machine", "cowboy-code-adapter", "cowboy-acp-worker"];

/// Own the exact bundle used for both the guard probe and installation copies.
/// Caller paths can be replaced during the probe without changing this snapshot.
pub(super) struct Bundle(
    Scratch,
    Option<super::signed_bootstrap::Authenticated>,
    Option<Vec<u8>>,
);

impl Bundle {
    pub(super) fn prepare(source: &Path) -> Result<Self> {
        let bundle = Self(Scratch::new()?, None, None);
        for name in PAYLOADS {
            let path = if name == "cowboy-machine" {
                source.to_path_buf()
            } else {
                super::companion_binary(source, name)
            };
            ensure!(
                path.is_file(),
                "bootstrap bundle is missing {}",
                path.display()
            );
            let target = bundle.payload(name);
            std::fs::copy(&path, &target)
                .with_context(|| format!("snapshotting bootstrap payload {}", path.display()))?;
            super::set_mode(&target, 0o755)?;
        }
        check(&bundle.payload("cowboy-machine"))?;
        Ok(bundle)
    }

    pub(super) fn prepare_signed(manifest: &Path, artifact: &Path, key: &Path) -> Result<Self> {
        let payloads = super::signed_bootstrap::authenticate(manifest, artifact, key)?;
        let mut bundle = Self(Scratch::new()?, Some(payloads), None);
        std::fs::copy(std::env::current_exe()?, bundle.payload(".verifier"))?;
        bundle.2 = Some(super::signed_bootstrap::verifier_digest(
            &bundle.payload(".verifier"),
        )?);
        for (name, bytes) in &bundle.1.as_ref().expect("authenticated payloads").payloads {
            std::fs::write(bundle.payload(name), bytes)?;
            super::set_mode(&bundle.payload(name), 0o755)?;
        }
        check(&bundle.payload("cowboy-machine"))?;
        bundle.verify()?;
        Ok(bundle)
    }

    pub(super) fn verify(&self) -> Result<()> {
        let Some(expected) = &self.1 else {
            return Ok(());
        };
        ensure!(
            std::fs::read_dir(&self.0.0)?.count() == PAYLOADS.len() + 1,
            "signed bootstrap probe changed bundle entries"
        );
        ensure!(
            super::signed_bootstrap::verifier_digest(&self.payload(".verifier"))?
                == *self.2.as_ref().expect("captured verifier"),
            "signed bootstrap probe changed installer-owned verifier"
        );
        for (name, bytes) in &expected.payloads {
            ensure!(
                super::signed_bootstrap::read_regular(&self.payload(name), bytes.len() as u64)?
                    == *bytes,
                "signed bootstrap probe changed authenticated payload"
            );
        }
        Ok(())
    }

    pub(super) fn install_signed(&self, state: &Path) -> Result<Option<PathBuf>> {
        let Some(evidence) = &self.1 else {
            return Ok(None);
        };
        self.verify()?;
        let root = state.join("signed-bootstrap");
        match DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    std::fs::symlink_metadata(&root)?.is_dir(),
                    "signed bootstrap root is not regular"
                );
            }
            Err(error) => return Err(error.into()),
        }
        let generation = root.join(format!("{:032x}", rand::random::<u128>()));
        DirBuilder::new().mode(0o700).create(&generation)?;
        for name in PAYLOADS.into_iter().chain(["verifier"]) {
            let source = self.payload(if name == "verifier" {
                ".verifier"
            } else {
                name
            });
            super::atomic_replace(&generation.join(name), 0o755, |file| {
                std::io::copy(&mut File::open(source)?, file)?;
                Ok(())
            })?;
        }
        super::atomic_write(&generation.join("manifest.json"), &evidence.manifest, 0o600)?;
        super::atomic_write(&generation.join("artifact"), &evidence.artifact, 0o600)?;
        ensure!(
            super::signed_bootstrap::verifier_digest(&generation.join("verifier"))?
                == *self.2.as_ref().expect("captured verifier"),
            "installed verifier differs from snapshot"
        );
        for (name, bytes) in &evidence.payloads {
            ensure!(
                super::signed_bootstrap::read_regular(&generation.join(name), bytes.len() as u64)?
                    == *bytes,
                "published bootstrap differs from captured package"
            );
        }
        ensure!(
            super::signed_bootstrap::read_regular(
                &generation.join("manifest.json"),
                evidence.manifest.len() as u64
            )? == evidence.manifest
                && super::signed_bootstrap::read_regular(
                    &generation.join("artifact"),
                    evidence.artifact.len() as u64
                )? == evidence.artifact,
            "published bootstrap evidence differs from snapshot"
        );
        File::open(&generation)?.sync_all()?;
        File::open(&root)?.sync_all()?;
        File::open(state)?.sync_all()?;
        Ok(Some(generation))
    }

    pub(super) fn payload(&self, name: &str) -> PathBuf {
        self.0.0.join(name)
    }
}

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
        .context("bootstrap must support the portable Session deletion and host cache guard before installation")
}

fn check_with_timeout(source: &Path, timeout: Duration) -> Result<()> {
    let source = source
        .canonicalize()
        .context("resolving bootstrap probe executable")?;
    let scratch = Scratch::new()?;
    let state = scratch.0.join("state");
    for case in ["empty", "committed", "floor"] {
        let committed = case == "committed";
        if committed {
            let journal = state.join("session-deletions");
            std::fs::create_dir_all(&journal)?;
            std::fs::write(journal.join("deletions.json"), b"{}")?;
        } else if case == "floor" {
            std::fs::create_dir(&state)?;
            std::fs::write(
                state.join(crate::session_deletion_admission::reader_floor::NAME),
                b"{}",
            )?;
        }
        let stdout_path = scratch.0.join(format!("stdout-{case}"));
        let stderr_path = scratch.0.join(format!("stderr-{case}"));
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
            std::fs::remove_file(state.join("session-deletions/deletions.json"))?;
            std::fs::remove_dir(state.join("session-deletions"))?;
            std::fs::remove_dir(&state)?;
        } else if case == "floor" {
            ensure!(
                status.code() == Some(1),
                "bootstrap did not refuse portable reader floor"
            );
            ensure!(
                String::from_utf8_lossy(&stderr).contains("portable reader floor"),
                "bootstrap returned the wrong reader-floor refusal"
            );
            ensure!(
                bounded_output(&state.join(crate::session_deletion_admission::reader_floor::NAME))?
                    == b"{}",
                "bootstrap probe changed the reader floor"
            );
            ensure!(
                std::fs::read_dir(&state)?.take(2).count() == 1,
                "bootstrap floor probe opened Machine stores"
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
                report
                    == serde_json::json!({"admitted": true, "writer": false, "host_cache_guard": 2}),
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
    #[ignore = "requires exact old and new immutable Machine host releases"]
    fn immutable_bootstrap_admits_cache_guard_and_refuses_deletion_only_release() {
        let new = PathBuf::from(
            std::env::var("COWBOY_TEST_PORTABLE_HOST_RELEASE").expect("new release required"),
        );
        let old = PathBuf::from(
            std::env::var("COWBOY_TEST_PORTABLE_OLD_HOST_RELEASE").expect("old release required"),
        );
        check(&new.join("bin/cowboy-machine")).unwrap();
        assert!(
            check(&old.join("bin/cowboy-machine"))
                .unwrap_err()
                .root_cause()
                .to_string()
                .contains("report is incompatible")
        );
    }

    #[test]
    fn advertised_floor_capability_without_floor_refusal_is_incompatible() {
        let root = tempfile::tempdir().unwrap();
        let source = script(
            root.path(),
            "#!/bin/sh\nif [ -e \"$3/session-deletions/deletions.json\" ]; then printf '%s' 'portable Session deletion reader admission' >&2; exit 1; fi\nprintf '%s' '{\"admitted\":true,\"writer\":false,\"host_cache_guard\":2}'\n",
        );
        assert!(
            check(&source)
                .unwrap_err()
                .root_cause()
                .to_string()
                .contains("did not refuse portable reader floor")
        );
    }

    #[test]
    fn deletion_guard_without_cache_authentication_cannot_be_installed() {
        let root = tempfile::tempdir().unwrap();
        let source = script(
            root.path(),
            "#!/bin/sh\nif [ -e \"$3/session-deletions/deletions.json\" ]; then printf '%s' 'portable Session deletion reader admission' >&2; exit 1; fi\nprintf '%s' '{\"admitted\":true,\"writer\":false}'\n",
        );
        assert!(
            check(&source)
                .unwrap_err()
                .root_cause()
                .to_string()
                .contains("report is incompatible")
        );
    }

    #[test]
    fn empty_success_alone_cannot_admit_a_bootstrap() {
        let root = tempfile::tempdir().unwrap();
        let source = script(
            root.path(),
            "#!/bin/sh\nprintf '%s' '{\"admitted\":true,\"writer\":false,\"host_cache_guard\":2}'\n",
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

//! Fail-closed process containment. This is lifecycle ownership, not a
//! sandbox against root or a same-user process deliberately migrating itself.
use std::io;
use std::path::PathBuf;

pub(crate) struct Containment {
    directory: PathBuf,
}

impl Containment {
    pub(crate) fn prepare(command: &mut tokio::process::Command) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let directory =
                crate::cgroup::agent_dir(&format!("contained-{}", uuid::Uuid::new_v4()))
                    .ok_or_else(|| io::Error::other("cgroup v2 unavailable"))?;
            // Exclusive creation: never adopt an existing group's processes.
            std::fs::create_dir(&directory)?;
            let guard = Self { directory };
            let membership = std::fs::OpenOptions::new()
                .write(true)
                .open(guard.directory.join("cgroup.procs"))?;
            let _kill = std::fs::OpenOptions::new()
                .write(true)
                .open(guard.directory.join("cgroup.kill"))?;
            if !guard.empty()? {
                return Err(io::Error::other("new native auth cgroup is populated"));
            }
            attach_before_exec(command, membership);
            Ok(guard)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = command;
            Err(io::Error::other(
                "native auth process containment unavailable",
            ))
        }
    }

    fn empty(&self) -> io::Result<bool> {
        let events = std::fs::read_to_string(self.directory.join("cgroup.events"))?;
        match events
            .lines()
            .find_map(|line| line.strip_prefix("populated "))
        {
            Some("0") => Ok(true),
            Some("1") => Ok(false),
            _ => Err(io::Error::other("invalid cgroup population observation")),
        }
    }

    pub(crate) async fn finish(self) -> io::Result<()> {
        std::fs::write(self.directory.join("cgroup.kill"), b"1")?;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !self.empty()? {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok(())
        })
        .await
        .map_err(|_| io::Error::other("native auth process tree exit unconfirmed"))?
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn attach_before_exec(command: &mut tokio::process::Command, membership: std::fs::File) {
    // SAFETY: after fork, only a write syscall and stack-only error conversion
    // occur. The descriptor was opened before fork and remains owned by the
    // closure. Writing 0 moves the calling child before any provider code runs.
    unsafe {
        command.pre_exec(move || match rustix::io::write(&membership, b"0") {
            Ok(1) => Ok(()),
            Ok(_) => Err(io::Error::from_raw_os_error(5)),
            Err(error) => Err(error.into()),
        });
    }
}

impl Drop for Containment {
    fn drop(&mut self) {
        // Aborted futures still signal their owned tree, but do not manufacture
        // an exit receipt. No blocking retry loop in Drop.
        let _ = std::fs::write(self.directory.join("cgroup.kill"), b"1");
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires delegated writable cgroup v2"]
    async fn native_auth_contains_detached_descendants_before_exec() {
        use tokio::io::AsyncReadExt as _;
        let mut command = tokio::process::Command::new("sh");
        command
            .args([
                "-c",
                "setsid sh -c 'cat /proc/self/cgroup; exec sleep 60' & wait",
            ])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true);
        let guard = Containment::prepare(&mut command).unwrap();
        let directory = guard.directory.clone();
        let mut child = command.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut line = vec![0; 4096];
        let n = tokio::time::timeout(std::time::Duration::from_secs(2), stdout.read(&mut line))
            .await
            .unwrap()
            .unwrap();
        let membership = std::str::from_utf8(&line[..n]).unwrap();
        assert!(membership.contains(directory.file_name().unwrap().to_str().unwrap()));
        assert!(!guard.empty().unwrap());
        // The membership report came from inside setsid, so detachment has
        // actually happened; no timing assumption stands in for that evidence.
        guard.finish().await.unwrap();
        assert!(!child.wait().await.unwrap().success());
        assert!(!directory.exists());
    }

    #[tokio::test]
    async fn failed_pre_exec_membership_never_runs_provider_code() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("executed");
        let mut command = tokio::process::Command::new("touch");
        command.arg(&marker);
        attach_before_exec(
            &mut command,
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        );
        assert!(command.spawn().is_err());
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn missing_population_after_kill_remains_unconfirmed() {
        let temp = tempfile::tempdir().unwrap();
        let guard = Containment {
            directory: temp.path().join("fixture"),
        };
        std::fs::create_dir(&guard.directory).unwrap();
        assert!(guard.finish().await.is_err());
    }

    #[test]
    fn absent_or_malformed_population_is_not_exit_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let guard = Containment {
            directory: temp.path().join("fixture"),
        };
        std::fs::create_dir(&guard.directory).unwrap();
        assert!(guard.empty().is_err());
        for value in ["", "frozen 0\n", "populated 2\n"] {
            std::fs::write(guard.directory.join("cgroup.events"), value).unwrap();
            assert!(guard.empty().is_err());
        }
        std::fs::write(guard.directory.join("cgroup.events"), "populated 1\n").unwrap();
        assert!(!guard.empty().unwrap());
        std::fs::write(
            guard.directory.join("cgroup.events"),
            "populated 0\nfrozen 0\n",
        )
        .unwrap();
        assert!(guard.empty().unwrap());
    }
}

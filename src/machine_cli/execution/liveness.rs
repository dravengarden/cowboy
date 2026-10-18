//! Prove loss of an execution process without treating network delay as loss.
use super::*;

fn current_boot() -> Option<String> {
    let value = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let value = value.trim();
    uuid::Uuid::parse_str(value).ok()?;
    Some(value.to_owned())
}

pub(super) fn record_boot(directory: &Path) -> Result<()> {
    let Some(boot) = current_boot() else {
        return Ok(());
    };
    let path = directory.join("host-boot-id");
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(mut file) => {
            std::io::Write::write_all(&mut file, boot.as_bytes())?;
            file.sync_all()?;
            std::fs::File::open(directory)?.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure!(
                std::fs::read_to_string(path)? == boot,
                "execution launch belongs to another boot"
            );
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(super) fn boot_changed(directory: &Path) -> bool {
    let previous = crate::logs::storage::private_read(&directory.join("host-boot-id"), 128)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok());
    match (previous, current_boot()) {
        (Some(previous), Some(current)) => {
            uuid::Uuid::parse_str(&previous).is_ok() && previous != current
        }
        _ => false,
    }
}

pub(super) fn missing_endpoint(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
        )
    })
}

fn unit_absent(output: &str) -> bool {
    output.lines().any(|line| line == "LoadState=not-found")
        && output.lines().any(|line| line == "ActiveState=inactive")
}

pub(super) async fn lifetime_gone(directory: &Path, binding: &BindingV1, systemd: bool) -> bool {
    if boot_changed(directory) {
        return true;
    }
    // Older contracts have no boot marker. The exact transient unit being
    // absent and inactive proves that none of its command descendants remain.
    if !systemd {
        return false;
    }
    let unit = format!(
        "cowboy-execution-{}.service",
        binding.environment.incarnation
    );
    matches!(tokio::time::timeout(Duration::from_secs(3),
        tokio::process::Command::new("systemctl")
            .args(["--user", "show", &unit, "--property=LoadState", "--property=ActiveState"])
            .stdin(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).output()).await,
        Ok(Ok(output)) if output.status.success() && unit_absent(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_is_not_proof_of_loss() {
        for kind in [
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::ConnectionReset,
        ] {
            assert!(!missing_endpoint(&std::io::Error::from(kind).into()));
        }
        assert!(missing_endpoint(
            &std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into()
        ));
        for state in [
            "LoadState=loaded\nActiveState=active",
            "LoadState=loaded\nActiveState=failed",
            "LoadState=not-found\nActiveState=activating",
            "ActiveState=inactive",
        ] {
            assert!(!unit_absent(state));
        }
        assert!(unit_absent("LoadState=not-found\nActiveState=inactive\n"));
    }

    #[test]
    fn boot_marker_is_private_persistent_and_never_reused() {
        if current_boot().is_none() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        assert!(!boot_changed(root.path()));
        record_boot(root.path()).unwrap();
        record_boot(root.path()).unwrap();
        assert!(!boot_changed(root.path()));
        let marker = root.path().join("host-boot-id");
        assert_eq!(std::fs::metadata(&marker).unwrap().mode() & 0o777, 0o600);
        std::fs::write(&marker, "00000000-0000-0000-0000-000000000000").unwrap();
        assert!(boot_changed(root.path()));
        assert!(record_boot(root.path()).is_err());
        std::fs::write(marker, "invalid").unwrap();
        assert!(!boot_changed(root.path()));
    }
}

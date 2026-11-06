//! Kernel boot identity shared by execution and native account recovery.
use anyhow::{Result, ensure};

pub(crate) fn current() -> Result<Option<uuid::Uuid>> {
    #[cfg(target_os = "linux")]
    let value = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    #[cfg(target_os = "macos")]
    let value = {
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()?;
        ensure!(output.status.success(), "cannot read kernel boot identity");
        String::from_utf8(output.stdout)?
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let boot = uuid::Uuid::parse_str(value.trim())?;
        ensure!(!boot.is_nil(), "invalid kernel boot identity");
        Ok(Some(boot))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Ok(None)
}

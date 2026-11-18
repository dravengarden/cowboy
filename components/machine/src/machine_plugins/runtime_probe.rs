//! Install-time probes of a staged Provider runtime.
//!
//! No vendor code of an `account_sandbox_v1` Provider runs as the Machine
//! user, not even a `--version`: its components and launcher are not probed
//! on the host. The daemon probes the generation's registered runtime instead
//! (`probe_runtime`), in a confined unit of its own probe identity rather
//! than a real account, before the generation's first account unit
//! (`account_sandbox_connection`); a failed or unavailable probe refuses the
//! generation.
use super::*;

/// Probe the staged launch command, except for an account-sandboxed Provider.
pub(super) async fn probe_provider_runtime(
    runtime: &RuntimeContract,
    command: &Path,
) -> Result<()> {
    ensure!(
        runtime.protocol == "agent-client-protocol-1.3",
        "unsupported Provider driver protocol"
    );
    if runtime.isolation == Some(cowboy_provider_sdk::RuntimeIsolationPolicy::AccountSandboxV1) {
        tracing::info!(
            command = %command.display(),
            "launcher probe skipped for an account-sandboxed Provider; the daemon probes its runtime"
        );
        return Ok(());
    }
    let output = tokio::time::timeout(
        PROVIDER_PROBE_TIMEOUT,
        tokio::process::Command::new(command)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .context("Provider staged probe timed out")?
    .with_context(|| format!("starting Provider entrypoint {}", command.display()))?;
    ensure!(
        output.status.success(),
        "Provider staged probe exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_launcher_probe_is_skipped_only_for_sandboxed_providers() {
        let root = tempfile::tempdir().unwrap();
        let launcher = root.path().join("launcher");
        let touched = root.path().join("touched");
        fs::write(
            &launcher,
            format!("#!/bin/sh\ntouch {}\n", touched.display()),
        )
        .unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
        let authored: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../plugins/antigravity/provider.json"
        ))
        .unwrap();
        let mut runtime: RuntimeContract =
            serde_json::from_value(authored["runtime"].clone()).unwrap();
        runtime.isolation = Some(cowboy_provider_sdk::RuntimeIsolationPolicy::AccountSandboxV1);
        probe_provider_runtime(&runtime, &launcher).await.unwrap();
        assert!(!touched.exists());
        runtime.isolation = None;
        probe_provider_runtime(&runtime, &launcher).await.unwrap();
        assert!(touched.exists());
    }
}

//! Install-time probes of a staged Provider runtime.
//!
//! An `account_sandbox_v1` Provider is probed without its launcher and
//! without the daemon. The daemon's `probe` mode runs only inside an existing
//! account's unit: probing through it would either create an account (a new
//! system user and home image per install) or run the new, not yet accepted
//! generation against a real account's persistent home. Neither belongs to an
//! install. The launcher's `--version` resolves and prepares its HOME-based
//! state (`~/.gemini`), so running it as the Machine user would touch the
//! Machine user's home, which a sandboxed Provider must never do. What is
//! probed instead is each vendor component (`agy --version`) with a
//! disposable HOME and a closed environment, and the probe must leave that
//! HOME empty: a component that writes state on `--version` fails the
//! install rather than being trusted with a real account. The launcher and
//! proxy are then first exercised inside the daemon's confined unit.
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
            "launcher probe skipped for an account-sandboxed Provider; its components were probed sealed"
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

/// A sealed component probe must not have written anything to its HOME.
pub(super) fn ensure_sealed(home: &Path, sealed: bool) -> Result<()> {
    if sealed {
        ensure!(
            fs::read_dir(home)?.next().is_none(),
            "a sandboxed Provider component wrote to its disposable HOME during its probe"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sealed_probe_home_must_stay_empty() {
        let home = tempfile::tempdir().unwrap();
        ensure_sealed(home.path(), true).unwrap();
        fs::write(home.path().join(".state"), b"x").unwrap();
        assert!(ensure_sealed(home.path(), false).is_ok());
        let error = ensure_sealed(home.path(), true).unwrap_err();
        assert!(error.to_string().contains("disposable HOME"));
    }

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

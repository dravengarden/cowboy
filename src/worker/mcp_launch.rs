//! Freeze mandatory MCP metadata before the adapter starts. The session-level
//! descriptor may be rewritten by another launch; this worker owns its copy.
use anyhow::{Context as _, Result, ensure};
use std::io::Write as _;

pub(super) fn configure(
    args: &super::WorkerArgs,
    prepared: &crate::provider::PreparedLaunch,
    spec: &mut crate::provider::LaunchSpec,
) -> Result<Option<tempfile::NamedTempFile>> {
    // Reserved launch descriptors must never be inherited from a parent process.
    spec.package_remove_env
        .insert(crate::worker_execution::DESCRIPTOR_ENV.to_owned());
    spec.package_remove_env
        .insert(super::startup_gate::LAUNCH_ENV.to_owned());
    let file = freeze(args, prepared.mcp_bindings)?;
    if let Some(file) = &file {
        spec.env.insert(
            "COWBOY_SESSION_EXTENSIONS".into(),
            file.path().display().to_string(),
        );
    }
    Ok(file)
}

pub(super) fn freeze(
    args: &super::WorkerArgs,
    capable: bool,
) -> Result<Option<tempfile::NamedTempFile>> {
    let Some(plan) = &args.mcp_plan else {
        return Ok(None);
    };
    ensure!(
        capable
            && !args.system
            && args
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding
                    .decode()
                    .is_ok_and(|binding| binding.managed.is_none())),
        "MCP requires a capable bound ordinary worker"
    );
    let path =
        std::env::var_os("COWBOY_SESSION_EXTENSIONS").context("required MCP descriptor missing")?;
    snapshot(plan, std::path::Path::new(&path)).map(Some)
}

fn snapshot(
    plan: &crate::mcp_binding::NativeLaunchPlan,
    path: &std::path::Path,
) -> Result<tempfile::NamedTempFile> {
    plan.validate()?;
    let bytes = crate::logs::storage::private_read(path, 65536)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(
        value.get("mcp") == Some(&serde_json::to_value(plan)?),
        "MCP descriptor differs from launch"
    );
    let mut file = tempfile::Builder::new()
        .prefix("cowboy-worker-mcp-")
        .tempfile()?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn mandatory_plan_is_private_exact_and_frozen_for_the_worker() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("extensions.json");
        let plan: crate::mcp_binding::NativeLaunchPlan = serde_json::from_value(json!({"schema":1,"command":"/nix/store/fixture/bin/cowboy","bindings":[{"name":"suger","descriptor":"/private/binding.json"}]})).unwrap();
        let value = json!({"schema":1,"mcp":plan,"memory":null});
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let file = snapshot(&plan, &path).unwrap();
        assert_eq!(
            std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&path, b"{}").unwrap();
        assert!(snapshot(&plan, &path).is_err());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(file.path()).unwrap())
                .unwrap(),
            value
        );
        let frozen = file.path().to_owned();
        drop(file);
        assert!(!frozen.exists());
    }
}

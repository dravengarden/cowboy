//! Native command runner and scoped post-logout home cleanup.
use super::*;
use std::process::Stdio;

/// Returns historical native command success, never whole-account revocation.
pub(super) async fn run(
    providers: &MachinePluginStore,
    provider: &str,
    generation: u64,
    login: &str,
    operation: &str,
) -> Result<Option<bool>, String> {
    let prepared = providers
        .prepare_native_logout(provider, generation, login, operation)
        .map_err(|_| "native logout preparation failed".to_owned())?;
    let crate::machine_plugins::LogoutPreparation::Execute(prepared) = prepared else {
        let crate::machine_plugins::LogoutPreparation::Observed(outcome) = prepared else {
            unreachable!()
        };
        if outcome == Some(true) {
            cleanup(providers, provider, generation, login, operation)?;
        }
        return Ok(outcome);
    };
    let cowboy_provider_sdk::AuthExecutor::CommandV1 {
        arguments,
        environment,
        ..
    } = prepared.method.executor
    else {
        return Err("native logout command required".into());
    };
    let mut command = authentication_process(
        &prepared.command,
        &arguments,
        &environment,
        &cowboy_provider_sdk::AuthTerminal::Pipes,
        &prepared.home,
    );
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let containment = login_process::Containment::prepare(&mut command)
        .map_err(|_| "native logout containment unavailable".to_owned())?;
    let child = command
        .spawn()
        .map_err(|_| "native logout spawn unconfirmed".to_owned())?;
    let (_cancel_owner, mut cancel) = tokio::sync::watch::channel(false);
    let (_input_owner, mut input) = tokio::sync::mpsc::channel(1);
    let status = login_process::run_contained(
        child,
        &mut cancel,
        &mut input,
        tokio::time::Instant::now() + Duration::from_secs(120),
        |_| {},
        containment,
    )
    .await
    .map_err(|_| "native logout outcome unconfirmed".to_owned())?;
    providers
        .record_native_logout_tree_exit(provider, generation, operation, status.success())
        .map_err(|_| "native logout outcome persistence failed".to_owned())?;
    if status.success() {
        cleanup(providers, provider, generation, login, operation)?;
    }
    Ok(Some(status.success()))
}

fn cleanup(
    providers: &MachinePluginStore,
    provider: &str,
    generation: u64,
    login: &str,
    operation: &str,
) -> Result<(), String> {
    providers
        .cleanup_logged_out_native_profile(provider, generation, login, operation)
        .map_err(|_| "native profile cleanup unconfirmed".to_owned())
}

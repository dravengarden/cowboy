//! Native command runner and scoped post-logout home cleanup.
use super::*;
use std::process::Stdio;

/// Historical native command success, never whole-account revocation, and
/// for an account-sandboxed Provider the daemon's token cleanup receipt.
pub(super) struct Outcome {
    pub succeeded: Option<bool>,
    pub sandbox: Option<crate::account_sandbox::LogoutCleanup>,
}

/// Run the logout and project its outcome for the Controller.
pub(super) async fn observe(
    store: &MachinePluginStore,
    logout: crate::provider_account::NativeLogoutRequest,
) -> Option<crate::provider_account::NativeLogoutObservation> {
    let _observation = store.native_observation_guard().await;
    let outcome = run(
        store,
        &logout.provider_id,
        logout.generation,
        &logout.login_request_id,
        &logout.operation_id,
    )
    .await
    .ok()?;
    let succeeded = outcome.succeeded;
    let receipt = outcome.sandbox.as_ref();
    Some(crate::provider_account::NativeLogoutObservation {
        process_trees_exited: succeeded.is_some()
            && store
                .native_auth_trees_exited(
                    &logout.provider_id,
                    logout.generation,
                    &logout.login_request_id,
                    &logout.operation_id,
                )
                .unwrap_or(false),
        request: logout,
        succeeded,
        profile_cleaned: succeeded == Some(true),
        credential_cleared: receipt.map(|receipt| receipt.credential_cleared),
        history_retained: receipt.map(|receipt| receipt.history_retained),
    })
}

pub(super) async fn run(
    providers: &MachinePluginStore,
    provider: &str,
    generation: u64,
    login: &str,
    operation: &str,
) -> Result<Outcome, String> {
    providers
        .recover_native_auth_processes(provider, generation, login, operation)
        .await
        .map_err(|_| "native auth process recovery unconfirmed".to_owned())?;
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
        return Ok(Outcome {
            succeeded: outcome,
            sandbox: None,
        });
    };
    if let Some(digest) = &prepared.sandbox_generation {
        // The daemon owns the unit; its confirmed end is the tree evidence.
        let logout = native_sandbox::logout(providers, provider, digest, generation).await?;
        providers
            .record_native_logout_tree_exit(provider, generation, operation, logout.succeeded)
            .map_err(|_| "native logout outcome persistence failed".to_owned())?;
        if logout.succeeded {
            cleanup(providers, provider, generation, login, operation)?;
        }
        return Ok(Outcome {
            succeeded: Some(logout.succeeded),
            sandbox: logout.receipt,
        });
    }
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
    providers
        .bind_native_logout_process(provider, generation, operation, containment.identity())
        .map_err(|_| "native logout process ownership persistence failed".to_owned())?;
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
    Ok(Outcome {
        succeeded: Some(status.success()),
        sandbox: None,
    })
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

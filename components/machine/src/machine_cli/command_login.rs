//! `MachineCommand` family routed by `handle_machine_command`: Provider login and logout flows.

use super::*;

pub(super) fn handle(command: MachineCommand, context: MachineCommandContext) {
    let MachineCommandContext {
        protocol,
        events,
        components,
        providers,
        zed_adapter_socket,
        login_sessions,
        ..
    } = context;
    match command {
        MachineCommand::QueryLoginStop { request_id, login } => {
            let observation = login_account_store(&providers, &login.account_id, protocol)
                .and_then(|store| {
                    anyhow::ensure!(!login.account_id.is_default(), "named account required");
                    store.observe_login_stop(&login)
                })
                .ok()
                .flatten();
            let _ = events.send(MachineEvent::LoginStopObserved {
                request_id,
                observation,
            });
        }
        MachineCommand::NativeLogout { request_id, logout } => {
            let scoped =
                login_account_store(&providers, &logout.account_id, protocol).and_then(|store| {
                    anyhow::ensure!(
                        !logout.account_id.is_default()
                            && !login_sessions.lock().contains_key(&logout.login_request_id),
                        "native login must be stopped before logout"
                    );
                    Ok(store)
                });
            tokio::spawn(async move {
                let observation = match scoped {
                    Ok(store) => {
                        let _observation = store.native_observation_guard().await;
                        native_logout::run(
                            &store,
                            &logout.provider_id,
                            logout.generation,
                            &logout.login_request_id,
                            &logout.operation_id,
                        )
                        .await
                        .ok()
                        .map(|succeeded| {
                            crate::provider_account::NativeLogoutObservation {
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
                            }
                        })
                    }
                    Err(_) => None,
                };
                let _ = events.send(MachineEvent::NativeLogoutObserved {
                    request_id,
                    observation,
                });
            });
        }
        MachineCommand::BeginNativeLogin {
            request_id,
            provider,
            account_id,
            spec,
        } => {
            let scoped = login_account_store(&providers, &account_id, protocol).and_then(|store| {
                anyhow::ensure!(
                    !account_id.is_default() && provider == "claude-code",
                    "native account required"
                );
                Ok(store)
            });
            match scoped {
                Ok(providers) => native_login::start(
                    request_id,
                    provider,
                    spec,
                    providers,
                    events,
                    login_sessions,
                ),
                Err(error) => {
                    let _ = events.send(MachineEvent::CommandResult {
                        request_id,
                        accepted: false,
                        detail: Some(error.to_string()),
                    });
                }
            }
        }
        MachineCommand::BeginLogin {
            request_id,
            provider,
            account_id,
            auth_method,
        } => {
            let providers = match login_account_store(&providers, &account_id, protocol) {
                Ok(providers) => providers,
                Err(error) => {
                    let _ = events.send(MachineEvent::CommandResult {
                        request_id,
                        accepted: false,
                        detail: Some(error.to_string()),
                    });
                    return;
                }
            };
            let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
            let (input_tx, input_rx) = tokio::sync::mpsc::channel(1);
            {
                let mut sessions = login_sessions.lock();
                let std::collections::hash_map::Entry::Vacant(entry) =
                    sessions.entry(request_id.clone())
                else {
                    let _ = events.send(MachineEvent::CommandResult {
                        request_id,
                        accepted: false,
                        detail: Some("authentication request is already active".into()),
                    });
                    return;
                };
                let session = LoginSession::new(cancel_tx, input_tx);
                entry.insert(session);
            }
            tokio::spawn(run_login(
                request_id,
                provider,
                auth_method,
                events,
                components,
                providers,
                zed_adapter_socket,
                LoginIo {
                    cancel: cancel_rx,
                    input: input_rx,
                    sessions: login_sessions,
                },
            ));
        }
        MachineCommand::CancelLogin { request_id } => {
            let accepted = request_login_cancel(&login_sessions, &request_id);
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted,
                detail: (!accepted).then_some(
                    "login request is inactive or cleanup remains unconfirmed".to_owned(),
                ),
            });
        }
        MachineCommand::CancelLoginRequest {
            request_id,
            login_request_id,
        } => {
            let accepted = request_login_cancel(&login_sessions, &login_request_id);
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted,
                detail: (!accepted).then_some(
                    "login request is inactive or cleanup remains unconfirmed".to_owned(),
                ),
            });
        }
        MachineCommand::SubmitLoginCode { request_id, code } => {
            let accepted = submit_login_code(&login_sessions, &request_id, &code);
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted,
                detail: (!accepted).then_some("login request is not accepting a code".to_owned()),
            });
        }
        other => tracing::error!(
            command = ?std::mem::discriminant(&other),
            "machine command routed to the wrong family (command_login)"
        ),
    }
}

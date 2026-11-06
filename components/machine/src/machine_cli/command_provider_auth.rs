//! `MachineCommand` family routed by `handle_machine_command`: Provider authentication, native profiles and usage, and native bindings.

use super::*;

pub(super) fn handle(
    command: MachineCommand,
    context: MachineCommandContext,
    execution: &PluginExecutionScope,
) {
    let MachineCommandContext {
        protocol,
        events,
        providers,
        login_sessions,
        runtime_commands,
        ..
    } = context;
    match command {
        MachineCommand::ApplyProviderAuth {
            request_id,
            envelope,
        } => {
            // Storage can verify account-bound envelopes before the runtime
            // supports account-scoped refresh, rolling and worker admission.
            // Do not let an early sender reach the legacy Provider-wide path.
            if !envelope.account_id.is_default() {
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted: false,
                    detail: Some(
                        "Named Provider account runtime admission is not available".into(),
                    ),
                });
                return;
            }
            tokio::spawn(async move {
                match providers.apply_auth(&envelope).await {
                    Ok(receipt) => {
                        let roll_provider = provider_auth_roll_target(&receipt, envelope.action);
                        let _ = events.send(MachineEvent::ProviderAuthReceipt {
                            request_id: request_id.clone(),
                            provider_id: receipt.provider_id,
                            account_id: envelope.account_id.clone(),
                            auth_generation: receipt.auth_generation,
                            replica_state: receipt.replica_state,
                            materialization_state: receipt.materialization_state,
                            detail: None,
                        });
                        // Applying an authoritative generation can migrate a
                        // complete legacy refresh into the writable runtime
                        // projection via an atomic directory rename. Publish
                        // observations directly instead of relying on a
                        // platform-specific filesystem event shape.
                        publish_provider_auth_observations(&providers, &events, true, protocol)
                            .await;
                        if let Some(provider) = roll_provider {
                            let _ =
                                runtime_commands.send(crate::runtime_wire::Frame::CoreCommand {
                                    command: crate::runtime_wire::CoreCommand::RollProvider {
                                        provider,
                                        account_id: Some(envelope.account_id.clone()),
                                    },
                                });
                        }
                        let _ = events.send(MachineEvent::CommandResult {
                            request_id,
                            accepted: true,
                            detail: None,
                        });
                    }
                    Err(error) => {
                        let _ = events.send(MachineEvent::ProviderAuthReceipt {
                            request_id: request_id.clone(),
                            provider_id: envelope.provider_id.clone(),
                            account_id: envelope.account_id.clone(),
                            auth_generation: envelope.auth_generation,
                            replica_state: crate::machine_protocol::ProviderReplicaState::Failed,
                            materialization_state:
                                crate::machine_protocol::ProviderMaterializationState::Failed,
                            detail: Some(format!("{error:#}")),
                        });
                        let _ = events.send(MachineEvent::CommandResult {
                            request_id,
                            accepted: false,
                            detail: Some(format!("{error:#}")),
                        });
                    }
                }
            });
        }
        MachineCommand::FinalizeProviderAuthCandidate {
            request_id,
            provider_id,
            account_id,
            auth_method,
            candidate_request_id,
        } => {
            let mut sessions = login_sessions.lock();
            let result =
                login_account_store(&providers, &account_id, protocol).and_then(|providers| {
                    if let Some(session) = sessions.get(&candidate_request_id) {
                        anyhow::ensure!(
                            session.candidate.as_ref().is_some_and(
                                |candidate| candidate.matches(&provider_id, &account_id)
                            ),
                            "candidate is still running or belongs to another authentication scope"
                        );
                    }
                    providers.finalize_auth_candidate(
                        &provider_id,
                        &auth_method,
                        &candidate_request_id,
                    )?;
                    if let Some(candidate) = sessions
                        .get_mut(&candidate_request_id)
                        .and_then(|session| session.candidate.as_mut())
                    {
                        candidate.discard()?;
                    }
                    Ok(())
                });
            if result.is_ok() {
                sessions.remove(&candidate_request_id);
            }
            let _ = events.send(MachineEvent::CommandResult {
                request_id,
                accepted: result.is_ok(),
                detail: result.err().map(|error| format!("{error:#}")),
            });
        }
        MachineCommand::QueryNativeProfile {
            request_id,
            provider,
            account_id,
            generation,
            login_request_id,
        } => {
            let profile = (protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION)
                .then(|| {
                    providers
                        .observe_native_profile(
                            &account_id,
                            &provider,
                            generation,
                            &login_request_id,
                        )
                        .ok()
                })
                .flatten()
                .map(Box::new);
            let _ = events.send(MachineEvent::NativeProfileObserved {
                request_id,
                profile,
            });
        }
        MachineCommand::CollectNativeUsage {
            request_id,
            profile,
        } => {
            let account = profile.account_id.clone();
            let provider = profile.provider_id.clone();
            let generation = profile.generation;
            let invocation = execution.native_usage(*profile);
            tokio::spawn(async move {
                let result = if protocol >= crate::machine_protocol::NATIVE_USAGE_PROTOCOL_VERSION {
                    providers.collect_native_usage(invocation).await
                } else {
                    Err(anyhow::anyhow!("machine_usage_update_required"))
                };
                let detail = result.as_ref().err().map(|error| {
                    let code = crate::machine_protocol::native_usage_error_code(&error.to_string());
                    tracing::warn!(account = account.as_str(), %provider, generation, reason = code, "Native account usage failed");
                    code.to_owned()
                });
                let _ = events.send(MachineEvent::PluginHostResponse {
                    request_id,
                    accepted: result.is_ok(),
                    started: true,
                    payload: result.ok(),
                    detail,
                });
            });
        }
        MachineCommand::PrepareNativeBinding {
            request_id,
            binding,
        } => {
            let accepted = protocol >= crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION
                && providers.prepare_native_binding(&binding).is_ok();
            let _ = events.send(MachineEvent::NativeBindingPrepared {
                request_id,
                binding: accepted.then_some(binding),
            });
        }
        other => tracing::error!(
            command = ?std::mem::discriminant(&other),
            "machine command routed to the wrong family (command_provider_auth)"
        ),
    }
}

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
            if let Some(detail) = named_auth_refusal(&envelope, protocol) {
                let _ = events.send(MachineEvent::CommandResult {
                    request_id,
                    accepted: false,
                    detail: Some(detail.into()),
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
        } => observe_native_profile(
            protocol,
            events,
            providers,
            (
                request_id,
                provider,
                account_id,
                generation,
                login_request_id,
            ),
        ),
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
        reset @ MachineCommand::ConsumeNativeReset { .. } => {
            consume_native_reset(protocol, events, providers, execution, reset);
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

/// Answer `ConsumeNativeReset` with the collector's own reply (success or its
/// closed refusal) or a closed Machine code. `started` keeps whether the
/// collector may have reached the Provider, so the Controller never retries an
/// unknown outcome.
fn consume_native_reset(
    protocol: u16,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    providers: Arc<MachinePluginStore>,
    execution: &PluginExecutionScope,
    command: MachineCommand,
) {
    let MachineCommand::ConsumeNativeReset {
        request_id,
        profile,
        request,
        dispatch,
    } = command
    else {
        return;
    };
    let account = profile.account_id.clone();
    let generation = profile.generation;
    let invocation = execution.native_usage(*profile);
    tokio::spawn(async move {
        let result = if protocol >= crate::machine_protocol::NATIVE_RESET_SPACING_PROTOCOL_VERSION {
            providers
                .consume_native_reset(invocation, &request, &dispatch)
                .await
        } else {
            Err(anyhow::anyhow!("machine_usage_update_required").into())
        };
        let event = match result {
            Ok(payload) => MachineEvent::PluginHostResponse {
                request_id,
                accepted: true,
                started: true,
                payload: Some(payload),
                detail: None,
            },
            Err(failure) => {
                let code =
                    crate::machine_protocol::native_usage_error_code(&failure.error.to_string());
                tracing::warn!(
                    account = account.as_str(),
                    generation,
                    started = failure.started,
                    reason = code,
                    "Native account reset failed"
                );
                MachineEvent::PluginHostResponse {
                    request_id,
                    accepted: false,
                    started: failure.started,
                    payload: None,
                    detail: Some(code.to_owned()),
                }
            }
        };
        let _ = events.send(event);
    });
}

/// Named portable account usage (protocol 50). Only a closed reason code
/// leaves the Machine; see `machine_plugins::native_usage::portable`.
pub(super) fn portable_usage(
    command: MachineCommand,
    context: MachineCommandContext,
    execution: &PluginExecutionScope,
) {
    let MachineCommand::CollectPortableUsage {
        request_id,
        request,
    } = command
    else {
        return;
    };
    let MachineCommandContext {
        protocol,
        events,
        providers,
        ..
    } = context;
    let account = request.account_id.clone();
    let provider = request.plugin_id.clone();
    let invocation = execution.portable_usage(*request);
    tokio::spawn(async move {
        let result = if protocol >= crate::machine_protocol::PORTABLE_USAGE_PROTOCOL_VERSION {
            providers.collect_portable_usage(invocation).await
        } else {
            Err(anyhow::anyhow!("machine_usage_update_required"))
        };
        let detail = result.as_ref().err().map(|error| {
            let code = crate::machine_protocol::portable_usage::portable_usage_error_code(
                &error.to_string(),
            );
            tracing::warn!(account = account.as_str(), %provider, reason = code, "Portable account usage failed");
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

/// Immediate banked reset of a named portable account (protocol 51). Only
/// the closed reset reply or a closed code leaves the Machine.
pub(super) fn portable_reset(
    command: MachineCommand,
    context: MachineCommandContext,
    execution: &PluginExecutionScope,
) {
    let MachineCommand::ConsumePortableReset {
        request_id,
        request,
    } = command
    else {
        return;
    };
    let MachineCommandContext {
        protocol,
        events,
        providers,
        ..
    } = context;
    let account = request.account_id.clone();
    let provider = request.plugin_id.clone();
    let invocation = execution.portable_reset(*request);
    tokio::spawn(async move {
        let result = if protocol
            >= crate::machine_protocol::portable_usage::PORTABLE_RESET_PROTOCOL_VERSION
        {
            providers.consume_portable_reset(invocation).await
        } else {
            Err(anyhow::anyhow!("machine_usage_update_required").into())
        };
        let event = match result {
            Ok(payload) => MachineEvent::PluginHostResponse {
                request_id,
                accepted: true,
                started: true,
                payload: Some(payload),
                detail: None,
            },
            Err(failure) => {
                let code =
                    crate::machine_protocol::native_usage_error_code(&failure.error.to_string());
                let code = if code == "native_usage_unavailable" {
                    crate::machine_protocol::portable_usage::portable_usage_error_code(
                        &failure.error.to_string(),
                    )
                } else {
                    code
                };
                tracing::warn!(account = account.as_str(), %provider, started = failure.started,
                    reason = code, "Portable account reset failed");
                MachineEvent::PluginHostResponse {
                    request_id,
                    accepted: false,
                    started: failure.started,
                    payload: None,
                    detail: Some(code.to_owned()),
                }
            }
        };
        let _ = events.send(event);
    });
}

/// A named envelope carries an account vault's Service-held credential (an
/// entered key or a subscription sign-in), applied into that account's own
/// scope. Named credentials of a native-only Provider (Claude) live solely in
/// its native home and are never delivered this way; an old Controller cannot
/// address account scopes at all.
fn named_auth_refusal(
    envelope: &crate::machine_protocol::SealedProviderAuth,
    protocol: u16,
) -> Option<&'static str> {
    if envelope.account_id.is_default() {
        return None;
    }
    (protocol < crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION
        || crate::provider_account::portable_credentials_allowed(
            &envelope.account_id,
            &envelope.provider_id,
        )
        .is_err())
    .then_some("Named Provider account runtime admission is not available")
}

/// Answer `QueryNativeProfile`: a login interrupted by a Machine crash (for a
/// sandboxed one, finished by the daemon) is recovered before it is
/// observed, not only when another login starts. A live owner is never
/// touched.
fn observe_native_profile(
    protocol: u16,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    providers: Arc<MachinePluginStore>,
    (request_id, provider, account_id, generation, login_request_id): (
        String,
        String,
        crate::provider_account::ProviderAccountId,
        u64,
        String,
    ),
) {
    tokio::spawn(async move {
        if protocol < crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION {
            let _ = events.send(MachineEvent::NativeProfileObserved {
                request_id,
                profile: None,
            });
            return;
        }
        if let Err(error) = providers
            .recover_abandoned_native_logins(&provider, &login_request_id)
            .await
        {
            tracing::warn!(%provider, generation, error = %format!("{error:#}"), "native login recovery before observation failed");
        }
        let profile = providers
            .observe_native_profile(&account_id, &provider, generation, &login_request_id)
            .ok()
            .map(Box::new);
        let _ = events.send(MachineEvent::NativeProfileObserved {
            request_id,
            profile,
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_vault_named_accounts_receive_service_credentials() {
        let envelope = |provider: &str, account: crate::provider_account::ProviderAccountId| {
            serde_json::from_value::<crate::machine_protocol::SealedProviderAuth>(
                serde_json::json!({
                    "envelope_schema": 2, "provider_id": provider, "account_id": account,
                    "auth_generation": 1, "auth_contract_fingerprint": "sha256:x",
                    "projection_schema": "p", "action": "apply",
                    "ephemeral_public_key": "", "nonce": "", "ciphertext": "",
                    "service_public_key": "", "signature": ""
                }),
            )
            .unwrap()
        };
        let named: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let current = crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION;
        for provider in [
            "claude-deepseek",
            "codex-deepseek",
            "deepseek-harness",
            "codex",
            "grok",
        ] {
            assert_eq!(
                named_auth_refusal(&envelope(provider, named.clone()), current),
                None
            );
            assert!(named_auth_refusal(&envelope(provider, named.clone()), current - 1).is_some());
        }
        // Claude named accounts stay native per Machine.
        assert!(named_auth_refusal(&envelope("claude-code", named.clone()), current).is_some());
        assert_eq!(
            named_auth_refusal(&envelope("claude-code", Default::default()), 0),
            None
        );
    }
}

//! Handlers for individual Machine events, called from the Machine
//! connection loop with the connection state they read or update.

use super::*;

/// Validate and ingest one Provider usage batch from a Machine and acknowledge it;
/// a batch this Controller cannot accept stays unacknowledged and is reported once.
#[allow(clippy::too_many_arguments)]
pub(super) async fn ingest_machine_usage_batch(
    state: &Arc<AppState>,
    store: &Store,
    hello: &crate::machine_protocol::MachineHello,
    challenge_id: &str,
    machine_write_tx: &mpsc::UnboundedSender<Message>,
    current_providers: &[crate::machine_protocol::PluginInventory],
    declined_usage_batch: &mut Option<(String, u64)>,
    producer_id: String,
    first_sequence: u64,
    last_sequence: u64,
    events: Vec<crate::machine_protocol::ProviderUsageEvent>,
) -> anyhow::Result<()> {
    let plugin_runtime = state.plugin_catalog.runtime();
    let shared_gateways = plugin_runtime
        .as_ref()
        .map_or_else(BTreeSet::new, |runtime| {
            provider_usage_shared_gateways(current_providers, runtime, &state.provider_catalog)
        });
    let bounded = plugin_runtime.as_ref().is_some_and(|runtime| {
        events.len() <= 200
            && events
                .first()
                .is_some_and(|event| event.sequence == first_sequence)
            && events
                .last()
                .is_some_and(|event| event.sequence == last_sequence)
            && events
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
            && events.iter().all(|event| {
                provider_usage_source_is_authorized(
                    current_providers,
                    runtime,
                    &shared_gateways,
                    &producer_id,
                    &event.provider,
                    &event.agent,
                )
            })
    });
    let mut result = Ok(());
    let declined = if !bounded {
        Some("invalid provider usage sequence envelope".to_owned())
    } else {
        match store
            .ingest_provider_usage(&hello.machine_id, &producer_id, &events)
            .await
        {
            Ok(acknowledged) => {
                let ack = crate::machine_protocol::MachineFrame::Command {
                    command: crate::machine_protocol::MachineCommand::ProviderUsageAck {
                        producer_id: producer_id.clone(),
                        sequence: acknowledged,
                    },
                };
                result = match queue_machine_json(machine_write_tx, &ack) {
                    Ok(()) => {
                        store
                            .machine_seen(&hello.machine_id, challenge_id, None)
                            .await
                    }
                    Err(()) => Err(anyhow::anyhow!(
                        "Machine disconnected while acknowledging provider usage"
                    )),
                };
                None
            }
            Err(error) => Some(format!("{error:#}")),
        }
    };
    // A batch this Controller cannot accept stays unacknowledged in
    // the Machine's spool and the connection is kept: under release
    // skew a newer Machine's usage would otherwise reconnect-loop
    // every session there. Report each declined batch once.
    if let Some(reason) = declined {
        let batch = (producer_id, first_sequence);
        if declined_usage_batch.as_ref() != Some(&batch) {
            tracing::warn!(
                machine = %hello.machine_id,
                producer_id = %batch.0,
                first_sequence,
                last_sequence,
                events = events.len(),
                %reason,
                "provider usage batch declined; left unacknowledged on the Machine"
            );
            *declined_usage_batch = Some(batch);
        }
    }
    result
}

/// Promote a temporary Provider login a Machine completed for a Service request,
/// in the background, and record the Machine as seen.
#[allow(clippy::too_many_arguments)]
pub(super) async fn accept_machine_service_auth_candidate(
    state: &Arc<AppState>,
    store: &Store,
    hello: &crate::machine_protocol::MachineHello,
    challenge_id: &str,
    request_id: String,
    provider_id: String,
    account_id: crate::provider_account::ProviderAccountId,
    native_identity: Option<crate::provider_account::NativeIdentity>,
    auth_method: String,
    provider_version: String,
    generation_digest: String,
    auth_contract_fingerprint: String,
    portable_schema: String,
    bundle: std::collections::BTreeMap<String, String>,
    account_label: Option<String>,
) -> anyhow::Result<()> {
    let candidate_state = Arc::clone(state);
    let candidate_machine_id = hello.machine_id.clone();
    tokio::spawn(async move {
        if let Err(error) = accept_service_auth_candidate(
            &candidate_state,
            &candidate_machine_id,
            &request_id,
            &provider_id,
            &account_id,
            native_identity.as_ref(),
            &auth_method,
            &provider_version,
            &generation_digest,
            &auth_contract_fingerprint,
            &portable_schema,
            bundle,
            account_label,
        )
        .await
        {
            tracing::warn!(%error, %provider_id, "temporary Provider login could not be promoted");
            // A rejected candidate from another account must not
            // poison the legitimate request's transient status.
            // Serialize failure publication with promotion. An
            // older rejected candidate must not overwrite a later
            // successful commit's status.
            let executors = candidate_state.provider_auth_executors.lock();
            let Some(executor) = executors.get(&request_id).filter(|executor| {
                !executor.promotion_started
                    && executor.accepts_candidate(
                        &candidate_machine_id,
                        &account_id,
                        &provider_id,
                        &auth_method,
                        &provider_version,
                        &generation_digest,
                        &auth_contract_fingerprint,
                    )
            }) else {
                return;
            };
            let Some(scope) = executor.auth_scope(&candidate_state) else {
                return;
            };
            scope.fail_authentication(&provider_id, &request_id);
            candidate_state.machine_control.record_service_login(
                &candidate_machine_id,
                crate::machine_control::ServiceLoginNotice {
                    request_id,
                    provider: provider_id,
                    state: crate::machine_protocol::AuthState::Error,
                    account_label: None,
                    detail: Some(error),
                },
            );
        }
    });
    store
        .machine_seen(&hello.machine_id, challenge_id, None)
        .await
}

/// Accept a refreshed Provider credential a Machine observed, when it matches
/// the expected generation and an installed Provider.
#[allow(clippy::too_many_arguments)]
pub(super) async fn accept_machine_auth_refresh_candidate(
    state: &Arc<AppState>,
    store: &Store,
    hello: &crate::machine_protocol::MachineHello,
    challenge_id: &str,
    current_providers: &[crate::machine_protocol::PluginInventory],
    native_identity: Option<crate::provider_account::NativeIdentity>,
    request_id: String,
    provider_id: String,
    account_id: crate::provider_account::ProviderAccountId,
    expected_generation: u64,
    provider_version: String,
    generation_digest: String,
    auth_contract_fingerprint: String,
    portable_schema: String,
    auth_method: String,
    bundle: std::collections::BTreeMap<String, String>,
) -> anyhow::Result<()> {
    let refresh_state = Arc::clone(state);
    let refresh_machine_id = hello.machine_id.clone();
    let installed = current_providers
        .iter()
        .find(|installed| installed.plugin_id == provider_id)
        .cloned();
    tokio::spawn(async move {
        if let Err(error) = accept_provider_auth_refresh_candidate(
            &refresh_state,
            &refresh_machine_id,
            installed,
            &request_id,
            &provider_id,
            &account_id,
            native_identity.as_ref(),
            expected_generation,
            &provider_version,
            &generation_digest,
            &auth_contract_fingerprint,
            &portable_schema,
            &auth_method,
            bundle,
        )
        .await
        {
            tracing::warn!(
                %error,
                machine = %refresh_machine_id,
                %provider_id,
                expected_generation,
                "Machine-refreshed Provider credentials were rejected"
            );
            if let Err(reconcile_error) = sync_provider_account_auth_to_machine(
                &refresh_state,
                &refresh_machine_id,
                &provider_id,
                &account_id,
            )
            .await
            {
                tracing::warn!(
                    error = %reconcile_error,
                    machine = %refresh_machine_id,
                    %provider_id,
                    "restoring authoritative Provider credentials after rejected refresh"
                );
            }
        }
    });
    store
        .machine_seen(&hello.machine_id, challenge_id, None)
        .await
}

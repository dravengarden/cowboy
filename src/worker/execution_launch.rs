//! The worker's execution endpoint for a session with an execution binding.
//!
//! An ordinary Provider reaches it through the loopback descriptor named by
//! `COWBOY_EXECUTION_DESCRIPTOR`. An account-sandboxed Provider's unit can
//! reach no host loopback service: its endpoint serves the Unix socket the
//! daemon binds into the unit, the daemon hands the unit the descriptor, and
//! the trusted process ledger admits every effect the unit has on the target
//! (docs/antigravity-real-accounts-design.md, R3-11 and "Revision 5
//! simplification").

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::{ManagedRound, WorkerArgs};
use crate::provider::{LaunchSpec, PreparedLaunch};
use crate::runtime_wire::StartSession;
use crate::worker_execution::{Client, Endpoint, LEDGER_ENV, Ledger};

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare(
    args: &WorkerArgs,
    launch: &StartSession,
    prepared: &PreparedLaunch,
    managed_child: bool,
    epoch: &str,
    notify_tx: &mpsc::UnboundedSender<()>,
    spec: &mut LaunchSpec,
) -> Result<(Option<Endpoint>, Option<ManagedRound>)> {
    let Some(binding) = args.execution_binding.as_ref().filter(|_| !managed_child) else {
        return Ok((None, None));
    };
    anyhow::ensure!(
        prepared.execution_jsonrpc,
        "Provider generation does not support execution environments"
    );
    let binding = binding.decode().map_err(anyhow::Error::msg)?;
    let binding_profile = binding.managed.as_ref().map(|managed| managed.profile);
    anyhow::ensure!(
        launch
            .provider_behavior
            .as_ref()
            .and_then(|behavior| behavior.execution.as_ref())
            .is_some_and(|execution| execution.accepts(
                binding.environment.protocol,
                &binding.environment.executor_digest
            )),
        "Provider generation has not accepted this exact executor"
    );
    anyhow::ensure!(
        binding.runtime.cwd == args.cwd.display().to_string(),
        "execution runtime directory mismatch"
    );
    let client = Client::new(
        args.session_id.clone(),
        epoch.to_owned(),
        binding.clone(),
        notify_tx.clone(),
    );
    let mut managed_round = None;
    if let Some(profile) = binding_profile {
        let (ready, receiver) = tokio::sync::watch::channel(None);
        let marker = Arc::new(Mutex::new(None));
        let (fetch, stored) = (Arc::clone(&client), Arc::clone(&marker));
        tokio::spawn(async move {
            let result = fetch.managed_round(profile).await.map(|round| {
                *stored.lock() = Some(round);
            });
            let _ =
                ready.send(Some(result.map_err(|error| {
                    format!("managed call round is unavailable: {error}")
                })));
        });
        managed_round = Some(ManagedRound {
            ready: receiver,
            marker,
        });
    }
    if spec.account_sandbox.is_some() {
        let path = std::env::var_os(LEDGER_ENV)
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_absolute());
        anyhow::ensure!(
            path.is_some(),
            "a sandboxed execution binding requires the Machine's process ledger"
        );
        client.set_ledger(Ledger::open(&args.session_id, path)?);
        // One read of the verified extension descriptor serves both what the
        // worker runs and what the unit learns.
        // The frozen copy exists only when the launch carried an MCP plan.
        let frozen = args
            .mcp_plan
            .as_ref()
            .and_then(|_| spec.env.get("COWBOY_SESSION_EXTENSIONS").cloned());
        let descriptor = super::sandbox_extensions::load(frozen.as_deref());
        let extensions = super::sandbox_extensions::derive(
            descriptor.as_ref(),
            frozen.is_some(),
            &args.session_id,
        );
        if let Some(memory) = &extensions.memory {
            client.set_memory(memory.clone());
        }
        client.set_extensions(crate::worker_execution::ExtensionBroker::start(
            &client,
            extensions.plan,
            extensions.memory,
        ));
        let summary = Some(extensions.summary);
        let (endpoint, bridge) = Endpoint::start_unix(Arc::clone(&client), summary)?;
        spec.execution_bridge = Some(bridge);
        return Ok((Some(endpoint), managed_round));
    }
    let endpoint = Endpoint::start(client, &args.cwd).await?;
    if let Some(descriptor) = endpoint.descriptor() {
        spec.env.insert(
            crate::worker_execution::DESCRIPTOR_ENV.to_owned(),
            descriptor.display().to_string(),
        );
    }
    Ok((Some(endpoint), managed_round))
}

/// Bound of the teardown settlement; what remains is left to the target
/// keeper's unattended reap.
const TEARDOWN_SETTLE: std::time::Duration = std::time::Duration::from_secs(60);

/// Fence every turn and terminate every recorded process at teardown.
pub(super) async fn settle(execution: Arc<Client>) {
    if execution.ledger().is_none() {
        return;
    }
    if tokio::time::timeout(TEARDOWN_SETTLE, execution.settle_all())
        .await
        .is_err()
    {
        let pending = execution.ledger().map_or(0, |ledger| ledger.live().len());
        tracing::warn!(
            event_name = "cowboy.execution.ledger_pending",
            pending,
            "target processes remain; the keeper's reap is the backstop"
        );
        if let Some(ledger) = execution.ledger() {
            ledger.mark_unresolved_effects_uncertain();
        }
    }
}

/// Surface the ledger's uncertain effect, durable across workers, in this
/// worker's snapshot.
pub(super) fn observe(shared: &Arc<super::Shared>) {
    let Some(ledger) = shared.execution.as_ref().and_then(|client| client.ledger()) else {
        return;
    };
    if let Some(method) = ledger.uncertain_effect() {
        shared.snapshot.lock().uncertain_effect = Some(crate::runtime_wire::UncertainEffect {
            method,
            sequence: ledger.uncertain_sequence(),
        });
    }
    let weak = Arc::downgrade(shared);
    ledger.set_observer(Box::new(move |method, sequence| {
        if let Some(shared) = weak.upgrade() {
            let _pub_guard = shared.crashed_drain.enter_event_publisher(&shared);
            shared.mark_uncertain_effect(method, sequence);
        }
    }));
}

pub(super) fn observe_with_slot(
    shared: &Arc<super::Shared>,
    ledger_slot: Option<Arc<Mutex<Option<Arc<Ledger>>>>>,
) {
    observe(shared);
    if let (Some(slot), Some(ledger)) = (
        ledger_slot,
        shared
            .execution
            .as_ref()
            .and_then(|client| client.ledger())
            .cloned(),
    ) {
        *slot.lock() = Some(ledger);
    }
}

/// Fence new target effects, start full execution settlement in parallel with
/// ACP shutdown (whose relay drain can run up to 10s), cancel any active turn,
/// and arm a bounded settlement deadline without severing broker IPC.
pub(super) fn trigger_sigterm_settlement(
    shared: &Arc<super::Shared>,
    cmd_tx: &mut Option<mpsc::UnboundedSender<crate::acp::AgentCommand>>,
    shutdown_deadline: &mut Option<tokio::time::Instant>,
) {
    if shutdown_deadline.is_some() {
        return;
    }
    if let Some(execution) = shared.execution.clone() {
        tokio::spawn(async move {
            execution.settle_all().await;
        });
    }
    if let Some(tx) = cmd_tx.take() {
        let _ = tx.send(crate::acp::AgentCommand::Cancel);
    }
    *shutdown_deadline = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(12));
}

pub(super) fn finalize_sigterm_cutoff(shared: &super::Shared) {
    if let Some(ledger) = shared.execution.as_ref().and_then(|client| client.ledger()) {
        ledger.mark_unresolved_effects_uncertain();
    }
}

/// Atomically verify epoch, workspace identity, and (when negotiated via
/// `Some(ack)`) uncertainty sequence, opening the execution ledger turn under
/// the ledger lock before ACP delivery or `TurnStarted` emission.
/// `None` preserves legacy Controller / legacy Broker hops without lockout.
pub(super) fn admit_prompt_turn(
    shared: &super::Shared,
    turn_id: &str,
    expected_worker_epoch: Option<&str>,
    acknowledged_uncertain_sequence: Option<u64>,
) -> std::result::Result<(), String> {
    if let Some(expected) = expected_worker_epoch
        && expected != shared.worker_epoch
    {
        return Err("worker epoch changed before prompt admission".to_owned());
    }
    if let Some(message) = shared.workspace_replaced() {
        return Err(message);
    }
    if let Some(acknowledged) = acknowledged_uncertain_sequence
        && let Some(snap) = shared.snapshot.lock().uncertain_effect.clone()
    {
        let required = snap.sequence.max(1);
        if required > acknowledged {
            return Err(format!(
                "Target {} outcome is uncertain (sequence {required}); review and acknowledge before continuing",
                snap.method
            ));
        }
    }
    if let Some(ledger) = shared.execution.as_ref().and_then(|client| client.ledger()) {
        ledger.try_begin_turn(turn_id, acknowledged_uncertain_sequence)?;
    }
    Ok(())
}

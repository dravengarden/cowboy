//! The worker's execution endpoint for a session with an execution binding.
//!
//! An ordinary Provider reaches it through the loopback descriptor named by
//! `COWBOY_EXECUTION_DESCRIPTOR`. An account-sandboxed Provider's unit can
//! reach no host loopback service: its endpoint serves the Unix socket the
//! daemon binds into the unit, the daemon hands the unit the descriptor, and
//! the trusted process ledger accounts every process the unit starts on the
//! target (docs/antigravity-real-accounts-design.md, R3-5, R3-6, R3-11).

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
        let ledger = Ledger::open(&args.session_id, path)?;
        ledger.bind(&binding)?;
        client.set_ledger(ledger);
        let (endpoint, bridge) = Endpoint::start_unix(Arc::clone(&client))?;
        client.settle_recovered();
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

/// Bound of the teardown settlement; what remains stays in the durable
/// ledger for the next worker of the binding.
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
        tracing::warn!(
            event_name = "cowboy.execution.ledger_pending",
            pending = execution.ledger().map_or(0, |ledger| ledger.live().len()),
            "target processes remain recorded for the next worker"
        );
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
            shared.mark_uncertain_effect(method, sequence);
        }
    }));
}

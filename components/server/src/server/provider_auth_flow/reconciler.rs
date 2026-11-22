//! The one Provider credential delivery reconciler, driven by durable state.
//!
//! Every successful delivery records a per-Machine acknowledgement in its
//! credential scope (`ProviderAuthService::record_delivery`). Periodically,
//! and when nudged (a Machine connected or reported its inventory), one loop
//! sends the current generation of every Service-held credential (the root
//! vault and each portable account) to each connected, enrolled Machine whose
//! acknowledgement is missing, and recomputes the scope's distribution state
//! from the acknowledgements. A recipient that fails backs off in memory,
//! keyed by Machine id (not by connection), up to an hour; a new connection
//! of that Machine resets it. Automatic repairs of a failed projection go
//! through this loop too: an inventory contradicting an acknowledgement only
//! forgets it, so a Machine reporting the same failure keeps its backoff. Nothing is owned per task:
//! an acknowledgement or a newer generation simply ends the resends.
use super::*;
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;
use std::time::Instant;

const TICK: Duration = Duration::from_secs(10);
const DELAYS: [Duration; 6] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(60),
    Duration::from_secs(300),
    Duration::from_secs(900),
    Duration::from_secs(3600),
];

type Key = (String, crate::provider_account::ProviderAccountId, String);

/// Failed recipients: the generation that failed, failures, next attempt.
#[derive(Default)]
struct Backoff {
    entries: HashMap<Key, (u64, usize, Instant)>,
}

impl Backoff {
    fn due(&self, key: &Key, generation: u64, now: Instant) -> bool {
        match self.entries.get(key) {
            Some((failed, _, next)) if *failed == generation => now >= *next,
            _ => true,
        }
    }

    fn failed(&mut self, key: Key, generation: u64, now: Instant) {
        let failures = match self.entries.get(&key) {
            Some((failed, failures, _)) if *failed == generation => failures + 1,
            _ => 1,
        };
        let delay = DELAYS[(failures - 1).min(DELAYS.len() - 1)];
        self.entries
            .insert(key, (generation, failures, now + delay));
    }

    fn succeeded(&mut self, key: &Key) {
        self.entries.remove(key);
    }

    fn reset_machine(&mut self, machine: &str) {
        self.entries.retain(|(failed, _, _), _| failed != machine);
    }
}

static BACKOFF: LazyLock<parking_lot::Mutex<Backoff>> = LazyLock::new(Default::default);
static NUDGE: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

/// A Machine's fresh inventory: forget each root acknowledgement it
/// contradicts (the Provider's projection failed, or it reports an older
/// generation than it acknowledged), then run a pass. The backoff of a
/// recipient that keeps reporting the same failed generation stays; only a
/// new connection (or a newer generation) makes it due sooner.
pub(in crate::server) fn observe_inventory(
    state: &AppState,
    machine: &str,
    plugins: &[crate::machine_protocol::PluginInventory],
) {
    for plugin in plugins {
        let Some(known) = state
            .provider_auth
            .deliveries(&plugin.plugin_id)
            .remove(machine)
        else {
            continue;
        };
        if contradicts(plugin, known.generation)
            && let Err(error) = state
                .provider_auth
                .forget_delivery(&plugin.plugin_id, machine)
        {
            tracing::warn!(%error, %machine, provider_id = %plugin.plugin_id, "forgetting a contradicted delivery acknowledgement");
        }
    }
    nudge(None);
}

/// Whether `plugin`'s report contradicts an acknowledgement of `generation`.
fn contradicts(plugin: &crate::machine_protocol::PluginInventory, generation: u64) -> bool {
    !failed_provider_auth_projection_ids(std::slice::from_ref(plugin)).is_empty()
        || plugin
            .auth_generation
            .is_some_and(|reported| reported < generation)
}

/// Run a pass soon; `machine` connected, so retry it at once.
pub(in crate::server) fn nudge(machine: Option<&str>) {
    if let Some(machine) = machine {
        BACKOFF.lock().reset_machine(machine);
    }
    NUDGE.notify_one();
}

pub(in crate::server) fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            let Some(state) = weak.upgrade() else { return };
            if *state.shutdown.borrow() {
                return;
            }
            pass(&state).await;
            drop(state);
            tokio::select! {
                () = NUDGE.notified() => {}
                () = tokio::time::sleep(TICK) => {}
            }
        }
    });
}

/// The Service-held credential scopes and their current statuses.
fn scopes(
    state: &AppState,
) -> Vec<(
    crate::provider_account::ProviderAccountId,
    Arc<crate::provider_service::ProviderAuthService>,
    Vec<crate::provider_service::ProviderAuthenticationStatus>,
)> {
    let mut scopes = vec![(
        crate::provider_account::ProviderAccountId::default(),
        Arc::clone(&state.provider_auth),
        state.provider_auth.replica_statuses(),
    )];
    let mut named = BTreeMap::<_, Vec<_>>::new();
    for status in state.provider_auth.portable_account_statuses() {
        named
            .entry(status.account_id.clone())
            .or_default()
            .push(status);
    }
    for (account, statuses) in named {
        if let Some(scope) = state.provider_auth.existing_account(&account) {
            scopes.push((account, scope, statuses));
        }
    }
    scopes
}

async fn pass(state: &Arc<AppState>) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let Ok(recipients) = store.credential_recipients().await else {
        return;
    };
    let connected = state.machine_control.connected_machine_ids();
    for (account, scope, statuses) in scopes(state) {
        for status in statuses {
            let provider = status.provider_id.as_str();
            let mut failed = BTreeSet::new();
            for machine in scope.unacknowledged(provider, &recipients) {
                let key = (machine.clone(), account.clone(), provider.to_owned());
                if !connected.contains(&machine) {
                    continue;
                }
                if !BACKOFF
                    .lock()
                    .due(&key, status.auth_generation, Instant::now())
                {
                    failed.insert(machine);
                    continue;
                }
                // Records the acknowledgement on success.
                match sync_provider_account_auth_to_machine(state, &machine, provider, &account)
                    .await
                {
                    Ok(_) => BACKOFF.lock().succeeded(&key),
                    Err(error) => {
                        tracing::warn!(%error, machine_id = %machine, provider_id = %provider,
                            account = account.as_str(), generation = status.auth_generation,
                            "Provider credential delivery not acknowledged; retrying later");
                        BACKOFF
                            .lock()
                            .failed(key, status.auth_generation, Instant::now());
                        failed.insert(machine);
                    }
                }
            }
            match scope.reconcile_distribution(provider, &recipients, &failed) {
                Ok(Some(marked)) => {
                    tracing::info!(provider_id = %provider, account = account.as_str(),
                    generation = marked.auth_generation, state = ?marked.distribution_state,
                    "Provider credential distribution reconciled")
                }
                Ok(None) => {}
                // Superseded meanwhile: the next pass reconciles the new one.
                Err(error) => {
                    tracing::debug!(%error, provider_id = %provider, "stale distribution reconciliation")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(machine: &str) -> Key {
        (machine.into(), Default::default(), "codex".into())
    }

    fn inventory(
        generation: u64,
        state: crate::machine_protocol::ProviderMaterializationState,
    ) -> crate::machine_protocol::PluginInventory {
        serde_json::from_value(serde_json::json!({
            "plugin_id": "codex", "plugin_version": "1.0.0", "plugin_kind": "agent_provider",
            "generation_digest": "d", "contract_fingerprint": "f", "state": "active",
            "auth_generation": generation, "replica_state": "current", "materialization_state": state,
        }))
        .unwrap()
    }

    #[test]
    fn only_contradicting_evidence_forgets_an_acknowledgement() {
        use crate::machine_protocol::ProviderMaterializationState::{
            Current, Failed, NotInstalled,
        };
        assert!(!contradicts(&inventory(16, NotInstalled), 16));
        assert!(!contradicts(&inventory(17, Current), 16));
        assert!(contradicts(&inventory(16, Failed), 16));
        assert!(contradicts(&inventory(15, Current), 16));
    }

    #[test]
    fn a_failed_recipient_backs_off_until_it_reconnects() {
        let mut backoff = Backoff::default();
        let now = Instant::now();
        assert!(backoff.due(&key("hawk"), 16, now));
        backoff.failed(key("hawk"), 16, now);
        assert!(!backoff.due(&key("hawk"), 16, now));
        assert!(backoff.due(&key("hawk"), 16, now + DELAYS[0]));
        // A newer generation is sent at once; another Machine is unaffected.
        assert!(backoff.due(&key("hawk"), 17, now));
        assert!(backoff.due(&key("ovh"), 16, now));
        // Failures back off further, capped at an hour.
        for _ in 0..10 {
            backoff.failed(key("hawk"), 16, now);
        }
        assert!(!backoff.due(&key("hawk"), 16, now + DELAYS[4]));
        assert!(backoff.due(&key("hawk"), 16, now + DELAYS[5]));
        // A reconnect (by Machine id, whatever its connection) retries at once.
        backoff.reset_machine("hawk");
        assert!(backoff.due(&key("hawk"), 16, now));
        backoff.failed(key("hawk"), 16, now);
        backoff.succeeded(&key("hawk"));
        assert!(backoff.due(&key("hawk"), 16, now));
    }
}

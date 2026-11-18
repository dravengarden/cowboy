//! Account-sandbox evidence the Controller's account controls ask a Machine
//! for (`docs/antigravity-real-accounts-design.md`, R3-6, R3-10): whether
//! the execution target of a dispatched turn whose worker is gone is proven
//! terminated, and the revocation's removal of an account's home and user.
//!
//! A turn's target is terminated only when all of these hold on this, its
//! original runtime Machine:
//!
//! - no worker of the turn's epoch runs: the session's connected worker has
//!   another epoch (a session has one journaled launch at a time, and a new
//!   launch requires the previous one's confirmed tree exit), or none is
//!   connected and the launch journal is absent (no worker ever ran) or
//!   records the tree exit;
//! - the session's durable execution ledger records no target process,
//!   including a start whose reply was lost;
//! - no account unit of the session survives its worker (an alive unit no
//!   live connection owns is still being stopped).
//!
//! Two parts stay conservative. A ledger that still records processes is
//! drained only by the next worker of the same binding: the Controller
//! forwards execution requests only from the session's current worker
//! epoch, so this Machine has no independent execution client and answers
//! `terminated: false` (the Controller keeps the account's slot). And the
//! closed `agy` log's class is never reported: the turn's log window (the
//! unit's `agy` process, its byte cursor and conversation ID) is known only
//! to the proxy inside the unit, so the daemon cannot attribute a window to
//! the turn; the Controller therefore settles a proven loss as
//! `unknown_refusal`-grade `UnitLost { evidence_complete: false }`.
use super::*;
use crate::account_sandbox as daemon;
use crate::provider_account::{
    AccountRemoveRequest, AccountTurnTerminationQuery, TargetTermination,
};

/// Everything the decision reads, gathered without holding any lock.
#[derive(Debug, Default)]
pub(super) struct Evidence {
    pub connected_epoch: Option<String>,
    /// The session's launch journal: `None` when no worker was ever
    /// journaled here, else whether its tree exit is recorded.
    pub worker_tree_exited: Option<bool>,
    pub ledger_processes: usize,
    /// The daemon's record of the account; `None` when it never created it.
    pub units: Option<daemon::AccountUnits>,
}

/// `Ok` when the turn's target is proven terminated, else why not.
pub(super) fn decide(
    query: &AccountTurnTerminationQuery,
    evidence: &Evidence,
) -> Result<(), String> {
    match (&evidence.connected_epoch, query.worker_epoch.as_str()) {
        (Some(epoch), turn) if turn.is_empty() || epoch == turn => {
            return Err("a worker of the turn's epoch may still run it".into());
        }
        (Some(_), _) => {}
        (None, _) if evidence.worker_tree_exited == Some(false) => {
            return Err("the session's worker tree exit is unconfirmed".into());
        }
        (None, _) => {}
    }
    if evidence.ledger_processes > 0 {
        return Err(format!(
            "{} target process(es) remain in the session's execution ledger",
            evidence.ledger_processes
        ));
    }
    if let Some(units) = &evidence.units {
        let replaced = evidence.connected_epoch.is_some();
        if let Some(unit) = units.units.iter().find(|unit| {
            unit.alive
                && unit.session_id.as_deref() == Some(query.session_id.as_str())
                && (!unit.owned || !replaced)
        }) {
            return Err(format!(
                "account unit {} of the session is alive",
                unit.unit
            ));
        }
        if let Some(unit) = units.unrecorded.first() {
            return Err(format!("unrecorded account unit {unit} is alive"));
        }
    }
    Ok(())
}

/// Gather the evidence for `query` (blocking: files and the daemon).
fn gather(
    providers: &MachinePluginStore,
    durable_state: &crate::machine_broker::DurableStateView,
    query: &AccountTurnTerminationQuery,
) -> Result<Evidence> {
    anyhow::ensure!(
        !query.account_id.is_default() && crate::provider_account::named_only(&query.provider_id),
        "turn termination evidence applies only to named accounts of account-sandbox Providers"
    );
    let target = durable_state
        .session_target(&query.session_id)
        .context("the Machine broker is not running")?;
    let worker_tree_exited = providers
        .account_worker_exit(&query.session_id)?
        .map(|receipt| {
            anyhow::ensure!(
                receipt.account == query.account_id,
                "the session's launch journal belongs to another account"
            );
            Ok(receipt.tree_exited)
        })
        .transpose()?;
    let ledger_processes = match &target.ledger {
        Some(path) => daemon::read_execution_ledger(path)?
            .map(|ledger| {
                anyhow::ensure!(
                    ledger.session_id.is_empty() || ledger.session_id == query.session_id,
                    "the execution ledger belongs to another session"
                );
                Ok(ledger.processes.len())
            })
            .transpose()?
            .unwrap_or(0),
        None => 0,
    };
    let units = providers.account_sandbox_units_of(&query.provider_id, &query.account_id)?;
    Ok(Evidence {
        connected_epoch: target.connected_epoch,
        worker_tree_exited,
        ledger_processes,
        units,
    })
}

pub(super) async fn termination(
    providers: Arc<MachinePluginStore>,
    durable_state: crate::machine_broker::DurableStateView,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    request_id: String,
    query: AccountTurnTerminationQuery,
) {
    let result = tokio::task::spawn_blocking(move || {
        let evidence = gather(&providers, &durable_state, &query)?;
        let decision = decide(&query, &evidence);
        if let Err(reason) = &decision {
            tracing::debug!(session = %query.session_id, reason, "account turn target not proven terminated");
        }
        anyhow::Ok(TargetTermination {
            terminated: decision.is_ok(),
            closed_log_class: None,
        })
    })
    .await
    .map_err(anyhow::Error::from)
    .and_then(std::convert::identity);
    let (observation, detail) = match result {
        Ok(observation) => (Some(observation), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let _ = events.send(MachineEvent::AccountTurnTerminationObserved {
        request_id,
        observation,
        detail,
    });
}

/// Remove a revoked account's home and user; only the daemon's receipt is
/// an observation.
pub(super) async fn remove(
    providers: Arc<MachinePluginStore>,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    request_id: String,
    remove: AccountRemoveRequest,
) {
    let result = tokio::task::spawn_blocking(move || providers.account_sandbox_remove(&remove))
        .await
        .map_err(anyhow::Error::from)
        .and_then(std::convert::identity);
    let (observation, detail) = match result {
        Ok(observation) => (Some(observation), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let _ = events.send(MachineEvent::AccountSandboxRemoveObserved {
        request_id,
        observation,
        detail,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(epoch: &str) -> AccountTurnTerminationQuery {
        AccountTurnTerminationQuery {
            account_id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            provider_id: "antigravity".into(),
            session_id: "s1".into(),
            worker_epoch: epoch.into(),
            delivery_id: "c1".into(),
        }
    }

    fn units(alive: bool, owned: bool, unrecorded: &[&str]) -> daemon::AccountUnits {
        serde_json::from_value(serde_json::json!({
            "key": "k", "boot_id": "b", "fenced": null,
            "credential": {"generation": 1, "state": "signed_in"},
            "units": [{"unit": "cga-k-runtime-1.service", "mode": "runtime",
                "invocation_id": "i", "boot_id": "b", "credential_generation": 1,
                "session_id": "s1", "state": "active", "alive": alive, "owned": owned}],
            "unrecorded": unrecorded,
        }))
        .unwrap()
    }

    #[test]
    fn a_turn_is_terminated_only_on_every_part_of_its_target() {
        let gone = Evidence {
            worker_tree_exited: Some(true),
            units: Some(units(false, false, &[])),
            ..Evidence::default()
        };
        decide(&query("e1"), &gone).unwrap();
        // No launch journal: no worker ever ran here; no daemon account.
        decide(&query("e1"), &Evidence::default()).unwrap();
        // The turn's own worker (or an unknown epoch) is connected.
        for (connected, turn) in [("e1", "e1"), ("e2", "")] {
            let running = Evidence {
                connected_epoch: Some(connected.into()),
                ..Evidence::default()
            };
            assert!(decide(&query(turn), &running).is_err());
        }
        // A replacement worker proves the old launch exited, and its own
        // (owned) unit is not the lost turn's.
        let replaced = Evidence {
            connected_epoch: Some("e2".into()),
            worker_tree_exited: Some(false),
            units: Some(units(true, true, &[])),
            ..Evidence::default()
        };
        decide(&query("e1"), &replaced).unwrap();
        // An unconfirmed worker exit, a recorded process, a unit no live
        // connection owns, or an unrecorded unit keeps the slot.
        let pending = [
            Evidence {
                worker_tree_exited: Some(false),
                ..Evidence::default()
            },
            Evidence {
                ledger_processes: 1,
                ..Evidence::default()
            },
            Evidence {
                connected_epoch: Some("e2".into()),
                units: Some(units(true, false, &[])),
                ..Evidence::default()
            },
            Evidence {
                units: Some(units(true, true, &[])),
                ..Evidence::default()
            },
            Evidence {
                units: Some(units(false, false, &["cga-k-runtime-2.service"])),
                ..Evidence::default()
            },
        ];
        for evidence in &pending {
            assert!(decide(&query("e1"), evidence).is_err(), "{evidence:?}");
        }
    }
}

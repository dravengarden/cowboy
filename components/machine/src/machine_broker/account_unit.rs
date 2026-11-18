//! Teardown of a worker whose Provider runs in the account sandbox: the
//! worker counts as stopped only after the daemon confirms that no unit of
//! its session remains. The worker's snapshot names its unit when it is
//! known; the session's durable launch record covers a worker that was lost
//! before (or without) reporting one.

use super::*;

impl Broker {
    /// `true` when the daemon confirmed every account unit of the session
    /// gone (stopping any a lost worker left behind), or it has none.
    pub(super) async fn confirm_account_unit_gone(&self, session_id: &str) -> bool {
        let unit = self
            .workers
            .lock()
            .get(session_id)
            .and_then(|worker| worker.snapshot.account_unit.clone())
            .map(|unit| unit.unit);
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        let confirmed = tokio::task::spawn_blocking(move || {
            if let Some(unit) = &unit {
                store.confirm_account_unit_gone(unit)?;
            }
            store.confirm_session_account_units_gone(&session)
        })
        .await;
        match confirmed {
            Ok(Ok(())) => true,
            outcome => {
                tracing::error!(
                    session = session_id,
                    ?outcome,
                    "account sandbox unit stop unconfirmed"
                );
                false
            }
        }
    }
}

/// What the broker knows about one session's turn target, for the
/// Controller's termination query (`machine_cli`'s account turn evidence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTarget {
    /// The epoch of the session's connected worker, if any.
    pub connected_epoch: Option<String>,
    /// The session's durable execution ledger file, when the Machine has a
    /// state root.
    pub ledger: Option<PathBuf>,
}

impl DurableStateView {
    /// `None` until a broker is attached (or after it ended).
    pub fn session_target(&self, session_id: &str) -> Option<SessionTarget> {
        let broker = self.0.lock().as_ref()?.upgrade()?;
        let connected_epoch = broker
            .workers
            .lock()
            .get(session_id)
            .map(|worker| worker.epoch.clone());
        let ledger = broker
            .args
            .call_context_root
            .as_deref()
            .and_then(|root| crate::account_sandbox::execution_ledger_file(root, session_id));
        Some(SessionTarget {
            connected_epoch,
            ledger,
        })
    }
}

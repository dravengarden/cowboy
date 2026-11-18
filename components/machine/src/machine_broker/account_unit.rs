//! Teardown of a worker whose Provider runs in the account sandbox: the
//! worker's launch record names its daemon unit, and the worker counts as
//! stopped only after the daemon confirms that unit is gone too.

use super::*;

impl Broker {
    /// `true` when the worker had no account unit, or the daemon confirmed
    /// it gone (stopping a unit a lost worker left behind).
    pub(super) async fn confirm_account_unit_gone(&self, session_id: &str) -> bool {
        let unit = self
            .workers
            .lock()
            .get(session_id)
            .and_then(|worker| worker.snapshot.account_unit.clone());
        let Some(unit) = unit else {
            return true;
        };
        let store = Arc::clone(&self.args.provider_store);
        let name = unit.unit.clone();
        match tokio::task::spawn_blocking(move || store.confirm_account_unit_gone(&name)).await {
            Ok(Ok(())) => true,
            outcome => {
                tracing::error!(
                    session = session_id,
                    unit = %unit.unit,
                    ?outcome,
                    "account sandbox unit stop unconfirmed"
                );
                false
            }
        }
    }
}

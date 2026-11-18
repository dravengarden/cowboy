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

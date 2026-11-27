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

    /// Confirm every durable account sandbox unit recorded for `session_id` is
    /// gone without trusting any worker-reported unit name.
    pub(super) async fn confirm_durable_session_account_units_gone(
        &self,
        session_id: &str,
    ) -> bool {
        let store = Arc::clone(&self.args.provider_store);
        let session = session_id.to_owned();
        let confirmed =
            tokio::task::spawn_blocking(move || store.confirm_session_account_units_gone(&session))
                .await;
        match confirmed {
            Ok(Ok(())) => true,
            outcome => {
                tracing::error!(
                    session = session_id,
                    ?outcome,
                    "durable account sandbox unit teardown unconfirmed"
                );
                false
            }
        }
    }
    pub(super) async fn handle_readmit_account_session(
        self: &Arc<Self>,
        session: StartSession,
        binding: crate::provider_account::PreparedBinding,
        command_id: String,
    ) {
        let session_id = session.session_id.clone();
        let result = self.readmit_account_session(session, &binding).await;
        self.send_controller(Frame::CommandAck {
            session_id,
            command_id,
            accepted: result.is_ok(),
            reason: result.err().map(|error| error.to_string()),
        });
    }
    pub(super) fn revoke_cache_protection(
        &self,
        session: &StartSession,
        session_id: &str,
        reason: &'static str,
    ) {
        let configuration = session.provider_behavior.as_ref().map_or_else(
            || crate::provider_behavior::legacy_behavior(&session.provider).configuration,
            |behavior| behavior.configuration.clone(),
        );
        if !crate::deepseek_cache::supported_behavior(&configuration) {
            return;
        }
        #[cfg(test)]
        self.revoked_cache_protection
            .lock()
            .push((session_id.to_owned(), reason));
        let provider = session.provider.clone();
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            if let Err(error) =
                crate::deepseek_cache::revoke_local_snapshot(&configuration, &session_id).await
            {
                tracing::warn!(
                    provider,
                    session = %session_id,
                    reason,
                    %error,
                    "failed to revoke DeepSeek cache-protection snapshot"
                );
            } else {
                tracing::debug!(
                    provider,
                    session = %session_id,
                    reason,
                    "revoked DeepSeek cache-protection snapshot"
                );
            }
        });
    }
}

/// Why a live worker cannot hibernate now, if anything would be lost.
pub(super) fn hibernation_refusal(worker: &WorkerSnapshot) -> Option<&'static str> {
    if worker.state != WorkerState::Running || worker.current_turn_id.is_some() {
        Some("session is busy")
    } else if !worker.pending_permissions.is_empty() {
        Some("session is waiting for a permission decision")
    } else if worker.background_tasks.unwrap_or(0) > 0 {
        Some("session has background work in progress")
    } else if worker.pending_prompt_count > 0 {
        Some("session has a queued prompt")
    } else {
        None
    }
}

pub(super) fn cutover_concurrency() -> usize {
    std::thread::available_parallelism()
        .map_or(4, std::num::NonZeroUsize::get)
        .div_ceil(2)
        .clamp(2, 8)
}

use super::{Status, Supervisor};

impl Supervisor {
    /// Ask a capable Machine to cooperatively drain an idle crashed worker
    /// while preserving the session's `Crashed` status, detail and held queue.
    /// Holds the same lifecycle lock as every wake/reset path and binds the
    /// request to `expected_worker_epoch` so a replacement worker is never
    /// touched. Returns `Ok(true)` if a `DrainCrashedWorker` command was queued.
    ///
    /// # Errors
    /// If the session is unknown or its runtime Machine cannot be resolved.
    pub fn drain_crashed_worker(
        &self,
        session_id: &str,
        expected_worker_epoch: &str,
    ) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        let runtime = self.runtime_for_session(session_id)?;
        let crate::remote_runtime::CrashedWorkerObservation::Seen(worker) =
            runtime.last_crashed_worker(session_id)
        else {
            return Ok(false);
        };
        if worker.worker_epoch != expected_worker_epoch {
            return Ok(false);
        }
        let named = !meta.provider_account_id.is_default();
        if meta.status != Status::Crashed && !named {
            return Ok(false);
        }
        if named {
            // Named-account admission can stop projecting activity either before
            // or after Hub records `Crashed`, leaving stale in-flight or
            // background counts; trust fresh epoch-bound Machine idle evidence
            // alongside active runtime prompt state.
            if !worker.idle || runtime.has_pending_prompt(session_id) {
                return Ok(false);
            }
        } else if !worker.idle
            || runtime.has_pending_prompt(session_id)
            || self.hub.session_has_in_flight_prompt(session_id)
            || meta.background_tasks > 0
        {
            return Ok(false);
        }
        Ok(runtime.drain_crashed_worker(session_id, expected_worker_epoch))
    }
}

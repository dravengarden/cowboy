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
        // Only automatically drain sessions whose Hub lifecycle already
        // reached `Status::Crashed` with no in-flight prompt. An unprojected
        // named-account crash whose Hub is still Busy stays owned by the
        // account lifecycle (`QuiesceAccountSession`) rather than draining
        // out from under Hub (R22-M3).
        if meta.status != Status::Crashed
            || !worker.idle
            || runtime.has_pending_prompt(session_id)
            || self.hub.session_has_in_flight_prompt(session_id)
            || meta.background_tasks > 0
        {
            return Ok(false);
        }
        Ok(runtime.drain_crashed_worker(session_id, expected_worker_epoch))
    }
}

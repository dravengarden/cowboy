//! Restart reconciliation of restored Busy turns: a turn without a worker
//! snapshot is reported cut off only after its runtime Machine has had its
//! chance to replay snapshots.
use super::{Hub, Status};

impl Hub {
    /// Finalize persisted Busy sessions whose detached owner did not reconnect
    /// within the server's bounded grace period. Returns exactly the sessions
    /// newly marked Interrupted for observability.
    pub fn finalize_runtime_reconciliation(&self) -> Vec<String> {
        self.finalize_settled_runtime_reconciliation(|_| true)
    }

    /// Finalize only restored Busy sessions whose runtime Machine has had
    /// its chance to replay worker snapshots (`settled(machine_id)`). A
    /// Machine that is itself restarting can reconnect long after the
    /// Controller's grace; judging its sessions before then reports turns
    /// whose workers survived as cut off. Unsettled sessions stay pending.
    pub fn finalize_settled_runtime_reconciliation(
        &self,
        settled: impl Fn(&str) -> bool,
    ) -> Vec<String> {
        let pending: Vec<String> = self
            .inner
            .runtime_reconciliation
            .lock()
            .iter()
            .cloned()
            .collect();
        let mut interrupted = Vec::new();
        for id in pending {
            let placement = self
                .inner
                .sessions
                .lock()
                .get(&id)
                .map(|session| (session.meta.status, session.meta.machine_id.clone()));
            let finalize = match &placement {
                Some((Status::Busy, machine_id)) => settled(machine_id),
                _ => true,
            };
            // A snapshot accepted meanwhile removed the id and owns the turn;
            // only the remover decides it.
            if !finalize || !self.inner.runtime_reconciliation.lock().remove(&id) {
                continue;
            }
            if matches!(placement, Some((Status::Busy, _)))
                && self.status(&id) == Some(Status::Busy)
            {
                self.record_restart_interruption(&id);
                interrupted.push(id);
            }
        }
        interrupted
    }

    /// Whether restored Busy sessions still await their Machine's snapshots.
    #[must_use]
    pub fn runtime_reconciliation_pending(&self) -> bool {
        !self.inner.runtime_reconciliation.lock().is_empty()
    }
}

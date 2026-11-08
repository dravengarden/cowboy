//! Resolve authority only from the currently observed worker.
use super::RemoteRuntime;
use anyhow::Result;

impl RemoteRuntime {
    /// Resolve call authority from this runtime's current worker observation.
    /// A broker registry placeholder or a disconnected/resetting runtime is not
    /// evidence of a live caller, even when its durable launch still matches.
    pub fn managed_call_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
    ) -> Result<crate::managed_calls::authority::Authority> {
        self.worker_authority(service, parent, None)
    }

    pub fn mcp_launch_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
        plan: &crate::mcp_binding::NativeLaunchPlan,
    ) -> Result<crate::managed_calls::authority::Authority> {
        plan.validate()?;
        self.worker_authority(service, parent, Some(plan))
    }

    fn worker_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
        plan: Option<&crate::mcp_binding::NativeLaunchPlan>,
    ) -> Result<crate::managed_calls::authority::Authority> {
        anyhow::ensure!(self.connected(), "parent runtime is disconnected");
        // Match merge_worker_snapshots' lock order (workers, then resetting).
        let workers = self.shared.workers.lock();
        let resetting = self.shared.resetting.lock();
        anyhow::ensure!(
            !resetting.contains(&parent.id),
            "parent worker is resetting"
        );
        let worker = workers
            .get(&parent.id)
            .ok_or_else(|| anyhow::anyhow!("parent worker is unavailable"))?;
        anyhow::ensure!(
            plan.is_none_or(|plan| worker
                .launch
                .as_ref()
                .and_then(|launch| launch.mcp_plan.as_deref())
                == Some(plan)),
            "worker MCP launch differs from admitted plan"
        );
        crate::managed_calls::authority::Authority::for_worker(service, parent, worker)
    }
}

//! Resolve authority only from the currently observed worker.
use super::RemoteRuntime;
use anyhow::{Context as _, Result};

type Authority = crate::managed_calls::authority::Authority;

impl RemoteRuntime {
    /// Resolve call authority from this runtime's current worker observation.
    /// A broker registry placeholder or a disconnected/resetting runtime is not
    /// evidence of a live caller, even when its durable launch still matches.
    pub fn managed_call_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
    ) -> Result<Authority> {
        self.worker_authority(service, parent, None)
    }

    pub fn mcp_launch_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
        plan: &crate::mcp_binding::NativeLaunchPlan,
    ) -> Result<Authority> {
        plan.validate()?;
        self.worker_authority(service, parent, Some(plan))
    }

    /// Like [`Self::mcp_launch_authority`], but `Ok(None)` when the worker
    /// owner is currently unobservable: the transport is down or the broker
    /// reports only its registry placeholder. That gap hides a surviving
    /// worker without replacing it, so it is classified from the same
    /// observation instead of being read as a different worker. This is never
    /// authority; callers use it only to decide whether a plan stays owned.
    pub fn observe_mcp_launch_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
        plan: &crate::mcp_binding::NativeLaunchPlan,
    ) -> Result<Option<Authority>> {
        plan.validate()?;
        self.observe_worker_authority(service, parent, Some(plan), true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn disconnect_for_test(&self) {
        self.shared
            .connected
            .store(false, std::sync::atomic::Ordering::Release);
    }

    fn worker_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
        plan: Option<&crate::mcp_binding::NativeLaunchPlan>,
    ) -> Result<Authority> {
        self.observe_worker_authority(service, parent, plan, false)?
            .context("parent worker is unobservable")
    }

    fn observe_worker_authority(
        &self,
        service: &str,
        parent: &crate::core::SessionMeta,
        plan: Option<&crate::mcp_binding::NativeLaunchPlan>,
        allow_pending: bool,
    ) -> Result<Option<Authority>> {
        // A runtime seeds its worker map before it is marked connected, so a
        // connected read below observes the reconnected inventory.
        if !self.connected() {
            anyhow::ensure!(allow_pending, "parent runtime is disconnected");
            return Ok(None);
        }
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
        if allow_pending && !worker.has_connected_owner() {
            return Ok(None);
        }
        anyhow::ensure!(
            plan.is_none_or(|plan| worker
                .launch
                .as_ref()
                .and_then(|launch| launch.mcp_plan.as_deref())
                == Some(plan)),
            "worker MCP launch differs from admitted plan"
        );
        Authority::for_worker(service, parent, worker).map(Some)
    }
}

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

    /// Ensure the session, replacing an idle worker whose MCP launch plan is
    /// no longer the session's current one (policy changed, or it launched
    /// before content-addressed descriptors). Adoption can never move a live
    /// worker's plan, so a plain ensure would only be refused on every send;
    /// a reset re-declares the plan and relaunches with it. A prompt queued
    /// after this call waits for the replacement. A busy worker keeps its turn.
    pub fn ensure_current_mcp_plan(&self, session: crate::runtime_wire::StartSession) {
        let stale = self
            .shared
            .workers
            .lock()
            .get(&session.session_id)
            .is_some_and(|worker| {
                worker.state == crate::runtime_wire::WorkerState::Running
                    && worker.current_turn_id.is_none()
                    && worker.has_connected_owner()
                    && worker
                        .launch
                        .as_ref()
                        .is_some_and(|launch| launch.mcp_plan != session.mcp_plan)
            });
        if !stale {
            self.ensure(session);
            return;
        }
        tracing::info!(
            session = %session.session_id,
            "replacing idle worker whose MCP launch plan is stale"
        );
        self.shared
            .hub
            .set_status(&session.session_id, crate::core::Status::Starting, None);
        self.reset(session);
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

#[cfg(test)]
mod tests {
    use crate::runtime_wire::{CoreCommand, StartSession, WorkerSnapshot};
    use serde_json::json;

    fn worker(plan: &str, turn: Option<&str>) -> WorkerSnapshot {
        serde_json::from_value(json!({
            "session_id":"s","worker_epoch":"worker-one","generation":"gen","state":"running",
            "current_turn_id":turn,
            "launch":{"session_id":"s","provider":"codex","cwd":"/w","generation":"gen",
                "mcp_plan":{"schema":1,"command":"/bin/cowboy",
                    "bindings":[{"name":"suger","descriptor":plan}]}}
        }))
        .unwrap()
    }

    fn declaration(plan: &str) -> StartSession {
        let mut session = worker(plan, None).launch.unwrap();
        session.generation.clear();
        session
    }

    fn commands(worker: WorkerSnapshot, plan: &str) -> Vec<CoreCommand> {
        let runtime = super::RemoteRuntime::for_test(crate::core::Hub::new(), vec![worker]);
        runtime.ensure_current_mcp_plan(declaration(plan));
        runtime.pending_for_test()
    }

    fn resets(commands: &[CoreCommand]) -> bool {
        commands.iter().any(|command| {
            matches!(command, CoreCommand::StopSession { command_id, .. }
                if command_id.starts_with("reset-"))
        })
    }

    #[tokio::test]
    async fn an_idle_worker_with_a_stale_mcp_plan_is_replaced_not_adopted() {
        let stale = commands(worker("/old.json", None), "/new.json");
        assert!(resets(&stale));
        assert!(stale.iter().any(|command| matches!(command,
            CoreCommand::EnsureSession { session }
                if session.adopt_only && session.mcp_plan == declaration("/new.json").mcp_plan)));
        assert!(!resets(&commands(worker("/new.json", None), "/new.json")));
        // A running turn is never interrupted to refresh its plan.
        assert!(!resets(&commands(
            worker("/old.json", Some("turn")),
            "/new.json"
        )));
    }
}

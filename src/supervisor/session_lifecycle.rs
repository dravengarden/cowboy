//! Session preparation, cwd migration, restore-failure hold, and worker recycling helpers.

use super::{
    Arc, PathBuf, RemoteRuntime, Status, Supervisor, refuse_trashed, resolve_session_workspace,
};

impl Supervisor {
    /// Validate a session's persisted cwd before any operation that may reuse a
    /// worker. A migrated cwd or crashed worker fences and replaces exactly that
    /// session's worker; the Cowboy id and native agent session id remain
    /// unchanged. Recycling crashed workers is important when a checkout was
    /// replaced at the same pathname: the metadata still looks valid while the
    /// old app-server process holds the deleted directory inode.
    /// Split-runtime retries defer worker preparation to dispatch preflight,
    /// which must verify the target lifetime before launching any replacement.
    pub fn prepare_session(&self, session_id: &str) -> Result<bool, String> {
        let _lifecycle = self.lifecycle.lock();
        if self.should_hold_native_restore_failure(session_id) {
            return Err(crate::provider_behavior::NATIVE_RESTORE_TIMEOUT_HOLD.to_owned());
        }
        refuse_trashed(&self.hub, session_id)?;
        if self
            .hub
            .session_info(session_id)
            .is_some_and(|info| info.meta.execution_binding.is_some())
        {
            // Retry must enqueue before launching a split-runtime worker. The
            // dispatch preflight first asks the target whether its lifetime
            // survived; only target-proven loss permits a new incarnation.
            // Eagerly recycling here launches against the stale binding and
            // can crash before dispatch gets a chance to recover it.
            return Ok(false);
        }
        self.prepare_session_inner(session_id)
    }

    pub(super) fn prepare_session_inner(&self, session_id: &str) -> Result<bool, String> {
        refuse_trashed(&self.hub, session_id)?;
        let migrated = self.resolve_and_persist_cwd(session_id)?;
        let session = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id);
        // A session whose workspace preparation failed or was interrupted
        // still points at the stable source checkout. Recycling would launch
        // a worker there, which the Machine refuses, and leave the session
        // permanently crashed. Workspace preparation owns its first launch.
        if session
            .as_ref()
            .is_some_and(crate::model::SessionMeta::awaits_workspace_preparation)
            && !self.has_worker(session_id)
        {
            return Ok(false);
        }
        let crashed = session
            .as_ref()
            .is_some_and(|meta| meta.status == Status::Crashed);
        if !migrated && !crashed {
            return Ok(false);
        }
        if crashed && !migrated && self.live_recoverable_turn_failure(session_id) {
            tracing::info!(
                session = session_id,
                "reusing live ACP worker after recoverable provider turn failure"
            );
            return Ok(false);
        }
        if crashed && !migrated {
            tracing::warn!(
                session = session_id,
                "recycling crashed worker before session resume"
            );
        }
        self.recycle_session_inner(session_id)?;
        Ok(true)
    }

    pub(super) fn resolve_and_persist_cwd(&self, session_id: &str) -> Result<bool, String> {
        let meta = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        meta.require_runtime_launch()?;
        // Remote paths belong to the selected Machine and are validated by
        // its trusted-workspace boundary. Never reinterpret them against the
        // controller's local workspace layout during resume.
        if meta.machine_id != "local" {
            return Ok(false);
        }
        let stored_cwd = PathBuf::from(&meta.cwd);
        let resolved = resolve_session_workspace(&self.workspace_root, &stored_cwd)?;
        if !resolved.changed {
            return Ok(false);
        }
        self.hub
            .update_session_cwd(session_id, resolved.path.display().to_string())?;
        tracing::warn!(
            session = session_id,
            old_cwd = %meta.cwd,
            new_cwd = %resolved.path.display(),
            project = ?resolved.project,
            resume = ?meta.agent_session_id,
            "retargeting session after workspace migration"
        );
        Ok(true)
    }

    pub(super) fn should_hold_native_restore_failure(&self, session_id: &str) -> bool {
        let Some(meta) = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
        else {
            return false;
        };
        if meta.status != Status::Crashed {
            return false;
        }
        self.hub
            .latest_crash_detail(session_id)
            .as_deref()
            .is_some_and(crate::provider_behavior::is_native_session_restore_timeout)
    }

    pub(super) fn recycle_session_inner(&self, session_id: &str) -> Result<(), String> {
        let runtime = self.runtime_for_session(session_id)?;
        let session = self.start_session(session_id)?;
        self.hub.set_status(session_id, Status::Starting, None);
        runtime.reset(session);
        Ok(())
    }

    pub(super) fn runtime_for_session(
        &self,
        session_id: &str,
    ) -> Result<Arc<RemoteRuntime>, String> {
        let machine_id = self
            .hub
            .session_list()
            .into_iter()
            .find(|meta| meta.id == session_id)
            .map(|meta| meta.machine_id)
            .ok_or_else(|| format!("unknown session {session_id:?}"))?;
        self.router
            .runtime(&machine_id)
            .ok_or_else(|| format!("machine {machine_id:?} is not connected"))
    }
}

use super::{AgentCommand, Supervisor};

impl Supervisor {
    /// Forward a command to a session's agent thread.
    ///
    /// Fast path: a live agent thread owns the session → deliver directly.
    ///
    /// Slow path — **post-restart resume**: after a daemon restart,
    /// [`crate::core::Hub::restore`] brings session metadata + event history back from
    /// postgres, but `senders` starts empty, so a restored session has no
    /// agent process. Without reviving it here the first command returned
    /// `unknown session`, which the WS layer turns into a fire-and-forget
    /// [`Outbound::Error`], so the prompt never reached an agent (the Web UI
    /// stayed empty). We lazily spawn an agent and resume the retained native
    /// thread through `session/resume` or `session/load`. Failed restoration
    /// never falls back to a blank session. Persisted configuration is restored
    /// before the first queued prompt reaches that thread.
    ///
    /// [`Outbound::Error`]: crate::core::Outbound::Error
    ///
    /// # Errors
    /// If the session is unknown to the Hub, its provider is no longer
    /// registered, or the agent thread cannot be spawned.
    pub fn send(&self, session_id: &str, command: AgentCommand) -> Result<(), String> {
        self.send_traced(session_id, command, None)
    }

    /// Send only to an already-live worker. Never repair, ensure, revive or replay.
    pub fn send_superagent_existing(
        &self,
        context: &crate::superagent::DispatchContext,
        text: &str,
        cmid: String,
    ) -> Result<(), String> {
        let _lifecycle = self.lifecycle.lock();
        if !self.hub.superagent_dispatch_current(context, &cmid) {
            return Err("superagent_session_changed".into());
        }
        let runtime = self.runtime_for_session(context.target())?;
        if !runtime.connected()
            || !runtime.has_worker(context.target())
            || runtime.hibernation_pending(context.target())
        {
            return Err("superagent_worker_unavailable".into());
        }
        runtime.prompt_traced(
            context.target(),
            vec![serde_json::json!({"type":"text","text":text})],
            Some(cmid),
            None,
        );
        Ok(())
    }
}

//! Output boundary for one ACP worker.
//!
//! A detached worker implements this contract by emitting versioned runtime
//! frames, keeping ACP's connection/futures out of the Cowboy control plane.

use crate::agent_model::{Event, SessionUsage, Status};

pub trait AgentSink: Send + Sync + 'static {
    /// Explicit execution hooks carry only local correlation, never prompt
    /// content or a Provider credential/environment binding.
    fn prompt_started(&self, _session_id: &str, _cmid: Option<&str>) {}
    fn prompt_completed(&self, _session_id: &str, _cmid: Option<&str>, _outcome: &str) {}
    fn set_status(&self, session_id: &str, status: Status, detail: Option<String>);
    fn push(&self, session_id: &str, event: Event);
    fn push_tagged(&self, session_id: &str, event: Event, cmid: Option<String>);
    fn set_config_options(&self, session_id: &str, options: serde_json::Value);
    fn set_agent_session_id(&self, session_id: &str, agent_session_id: String);
    fn set_session_usage(&self, session_id: &str, usage: SessionUsage);
    /// Live native background tasks that count as activity. A level, not an
    /// edge: every call replaces the previous count.
    fn set_background_tasks(&self, _session_id: &str, _count: u32) {}
    fn schedule_wakeup(&self, session_id: &str, delay_seconds: i64, prompt: String);
    fn session_is_system(&self, session_id: &str) -> bool;
    /// Immutable native profile, independent of the system-session UI flag.
    fn session_is_managed_read_only(&self, _session_id: &str) -> bool {
        false
    }
    /// A managed child accepts only the options its signed presets set, such
    /// as model and reasoning; mode and permission options stay fixed.
    fn managed_config_option_allowed(&self, _session_id: &str, _config_id: &str) -> bool {
        false
    }
    /// Native turn metadata for a managed child prompt. `Err` refuses the
    /// turn: a managed child never runs without its Machine-written round.
    fn managed_prompt_meta(
        &self,
        _session_id: &str,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
        Ok(None)
    }
    /// Completes once `managed_prompt_meta` can answer: a remote managed
    /// child reads its round from the execution target before its prompt.
    fn managed_round_fence(
        &self,
        _session_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<Option<Result<(), String>>>> {
        None
    }
    fn broadcast_error(&self, session_id: Option<String>, message: String);
    fn requeue_prompt(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    );
}

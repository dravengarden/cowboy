//! Output boundary for one ACP worker.
//!
//! A detached worker implements this contract by emitting versioned runtime
//! frames, keeping ACP's connection/futures out of the Cowboy control plane.

use crate::agent_model::{Event, SessionUsage, Status};

pub trait AgentSink: Send + Sync + 'static {
    fn set_status(&self, session_id: &str, status: Status, detail: Option<String>);
    fn push(&self, session_id: &str, event: Event);
    fn push_tagged(&self, session_id: &str, event: Event, cmid: Option<String>);
    fn set_config_options(&self, session_id: &str, options: serde_json::Value);
    fn set_agent_session_id(&self, session_id: &str, agent_session_id: String);
    fn set_session_usage(&self, session_id: &str, usage: SessionUsage);
    fn schedule_wakeup(&self, session_id: &str, delay_seconds: i64, prompt: String);
    fn session_is_system(&self, session_id: &str) -> bool;
    fn broadcast_error(&self, session_id: Option<String>, message: String);
    fn requeue_prompt(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    );
}

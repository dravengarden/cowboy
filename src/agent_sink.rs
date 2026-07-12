//! Output boundary for one ACP worker.
//!
//! The in-process implementation writes directly to [`Hub`]. A detached
//! worker implements the same contract by emitting versioned runtime frames,
//! keeping ACP's connection/futures out of the restartable Cowboy daemon.

use crate::agent_model::{Event, SessionUsage, Status};
use crate::core::Hub;

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

#[derive(Clone)]
pub struct HubAgentSink {
    hub: Hub,
}

impl HubAgentSink {
    #[must_use]
    pub fn new(hub: Hub) -> Self {
        Self { hub }
    }
}

impl AgentSink for HubAgentSink {
    fn set_status(&self, session_id: &str, status: Status, detail: Option<String>) {
        self.hub.set_status(session_id, status, detail);
    }

    fn push(&self, session_id: &str, event: Event) {
        self.hub.push(session_id, event);
    }

    fn push_tagged(&self, session_id: &str, event: Event, cmid: Option<String>) {
        self.hub.push_tagged(session_id, event, cmid);
    }

    fn set_config_options(&self, session_id: &str, options: serde_json::Value) {
        self.hub.set_config_options(session_id, options);
    }

    fn set_agent_session_id(&self, session_id: &str, agent_session_id: String) {
        self.hub.set_agent_session_id(session_id, agent_session_id);
    }

    fn set_session_usage(&self, session_id: &str, usage: SessionUsage) {
        self.hub.set_session_usage(session_id, usage);
    }

    fn schedule_wakeup(&self, session_id: &str, delay_seconds: i64, prompt: String) {
        self.hub.schedule_wakeup(session_id, delay_seconds, prompt);
    }

    fn session_is_system(&self, session_id: &str) -> bool {
        self.hub.session_is_system(session_id)
    }

    fn broadcast_error(&self, session_id: Option<String>, message: String) {
        self.hub.broadcast_error(session_id, message);
    }

    fn requeue_prompt(
        &self,
        session_id: &str,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
    ) {
        self.hub.requeue_prompt(session_id, text, content, cmid);
    }
}

//! Idle hibernation helpers and connection binding for [`super::RemoteRuntime`].

use std::sync::atomic::Ordering;

use super::RemoteRuntime;
use crate::machine_control::ConnectionToken;
use crate::runtime_wire::CoreCommand;

impl RemoteRuntime {
    /// Release an idle session's worker on its Machine without deleting the session.
    pub fn hibernate(&self, session_id: &str) {
        self.hibernate_epoch(session_id, None);
    }

    pub fn hibernate_epoch(&self, session_id: &str, expected_worker_epoch: Option<String>) {
        if self
            .shared
            .pending
            .lock()
            .has_pending_hibernation(session_id)
        {
            return;
        }
        let command_id = self.next_id("hibernate");
        let cmd = CoreCommand::HibernateSession {
            session_id: session_id.to_owned(),
            command_id: command_id.clone(),
            expected_worker_epoch,
        };
        self.queue(command_id, cmd);
    }

    /// Record whether this runtime's Machine host advertised epoch-bound
    /// `HibernateSession` enforcement on upgrade.
    pub fn set_hibernation_epoch_supported(&self, supported: bool) {
        self.shared
            .hibernation_epoch_supported
            .store(supported, Ordering::Release);
    }

    #[must_use]
    pub fn supports_hibernation_epoch(&self) -> bool {
        self.shared
            .hibernation_epoch_supported
            .load(Ordering::Acquire)
    }

    /// Bind this runtime 1-to-1 with its authenticated `MachineControl`
    /// connection registration before inserting it into `RuntimeRouter`.
    pub fn attach_connection(&self, token: ConnectionToken) {
        *self.shared.connection.lock() = Some(token);
    }

    #[must_use]
    pub fn connection_token(&self) -> Option<ConnectionToken> {
        self.shared.connection.lock().clone()
    }

    pub fn hibernation_pending(&self, session_id: &str) -> bool {
        self.shared
            .pending
            .lock()
            .has_pending_hibernation(session_id)
    }
}

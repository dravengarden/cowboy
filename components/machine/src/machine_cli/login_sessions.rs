//! Login registry of one authenticated Controller connection: the logins a
//! Machine may still cancel or feed a code, and the handles that revoke them
//! when the connection ends.

use anyhow::Result;

use super::login_process;
pub(crate) use crate::login_registry::{LoginSession, LoginSessions};

impl LoginSession {
    /// A login that begins now and expires after `login_process::TIMEOUT`,
    /// with no candidate credential, verification URL or browser bound yet.
    pub(super) fn new(
        cancel: tokio::sync::watch::Sender<bool>,
        input: tokio::sync::mpsc::Sender<String>,
        account: Option<crate::provider_account::ProviderAccountId>,
        provider: String,
    ) -> Self {
        Self {
            cancel,
            input,
            candidate: None,
            account,
            provider,
            expires_at_ms: super::unix_ms()
                .saturating_add(login_process::TIMEOUT.as_millis() as i64),
            verification_url: None,
            browser: None,
        }
    }
}

/// Accept cancellation intent, never report process exit. Keep an active
/// task's reservation until its own cleanup so the request cannot be reused
/// while that task can still remove or mutate its registry entry.
pub(super) fn request_login_cancel(sessions: &LoginSessions, request: &str) -> bool {
    let mut entries = sessions.lock();
    let Some(session) = entries.get_mut(request) else {
        // No live task accepts input for this request. Cancellation is
        // idempotent intent, NOT proof of process exit or credential cleanup.
        // Durable native profiles/candidate receipts still fence redispatch
        // and any new login whose predecessor has not conclusively exited.
        return true;
    };
    if let Some(candidate) = session.candidate.as_mut() {
        session.cancel.send_replace(true);
        // Candidate ownership transfers only after the login process exits.
        // Drop is best-effort only. Preserve the entry if explicit cleanup
        // fails so cancellation can retry rather than acknowledge a lost task.
        if let Err(error) = candidate.discard() {
            tracing::warn!(%error, "login candidate cancellation cleanup is unconfirmed");
            return false;
        }
        entries.remove(request);
        return true;
    }
    session.cancel.send(true).is_ok()
}

pub(super) fn submit_login_code(sessions: &LoginSessions, request: &str, code: &str) -> bool {
    crate::provider_account::login_code(code).is_ok_and(|code| {
        sessions.lock().get(request).is_some_and(|session| {
            !*session.cancel.borrow() && session.input.try_send(code.to_owned()).is_ok()
        })
    })
}

pub(super) struct LoginConnection(pub(super) LoginSessions);

impl Drop for LoginConnection {
    fn drop(&mut self) {
        for (_, login) in self.0.lock().drain() {
            let _ = login.cancel.send(true);
        }
    }
}

pub(super) struct LoginIo {
    pub(super) cancel: tokio::sync::watch::Receiver<bool>,
    pub(super) input: tokio::sync::mpsc::Receiver<String>,
    pub(super) sessions: LoginSessions,
}

impl LoginIo {
    pub(super) fn spawn(
        &self,
        request: &str,
        deadline: tokio::time::Instant,
        command: &mut tokio::process::Command,
    ) -> Result<tokio::process::Child, String> {
        let sessions = self.sessions.lock();
        if !sessions.contains_key(request) {
            return Err(login_process::Failure::Cancelled.detail().into());
        }
        login_process::check(&self.cancel, deadline).map_err(|error| error.detail().to_owned())?;
        // Cancellation signals under this registry lock; disconnect drains it.
        // Keep the guard until spawn returns, so a revoked task cannot launch.
        command.spawn().map_err(|error| error.to_string())
    }
}

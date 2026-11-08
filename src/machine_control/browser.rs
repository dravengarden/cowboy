//! Machine browser commands, leases and observations on the exact
//! authenticated connection.

use std::sync::Arc;

use parking_lot::RwLockWriteGuard;

use super::{ConnectionToken, LiveState, MachineControl, Reply, ReplyKind, RequestBinding};
use crate::machine_protocol::{MachineCommand, machine_browser};

/// Consumer of Machine browser observations (`machine_id`, event). Invoked
/// outside every correlation lock; the events name accounts and never enter
/// the shared diagnostics history.
pub type BrowserObserver = Arc<dyn Fn(&str, &machine_browser::Event) + Send + Sync>;

impl MachineControl {
    /// Install the consumer of Machine browser observations.
    pub fn set_browser_observer(&self, observer: BrowserObserver) {
        *self.browser_observer.write() = Some(observer);
    }

    /// One Machine browser command on the exact authenticated connection,
    /// answered by the browser state the Machine reports for it.
    pub async fn browser_command(
        &self,
        connection: &ConnectionToken,
        request_id: String,
        command: machine_browser::Command,
        timeout: std::time::Duration,
    ) -> Result<machine_browser::StateEvent, String> {
        let (rx, _pending) = self.begin_request(
            &connection.0.machine_id,
            &request_id,
            MachineCommand::Browser {
                request_id: request_id.clone(),
                browser: command,
            },
            ReplyKind::Browser,
            Some(RequestBinding::Connection(connection)),
        )?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Reply::Browser(Ok(state)))) => Ok(*state),
            Ok(Ok(Reply::Browser(Err(detail)))) => Err(detail),
            Ok(Ok(_)) => Err("Machine browser reply kind mismatch".into()),
            Ok(Err(_)) => Err("Machine browser response channel closed".into()),
            Err(_) => Err("Machine browser command timed out".into()),
        }
    }

    /// Fire-and-forget attachment lease renewal; the Machine answers only a
    /// refusal, and that reaches the observer.
    pub fn browser_lease(
        &self,
        connection: &ConnectionToken,
        instance: machine_browser::InstanceRef,
        attachment_id: String,
        expires_at_ms: i64,
    ) -> Result<(), String> {
        let request_id = self.request_id("browser-lease")?;
        self.send_on_connection(
            connection,
            MachineCommand::Browser {
                request_id,
                browser: machine_browser::Command::Lease {
                    instance,
                    attachment_id,
                    expires_at_ms,
                },
            },
        )
    }

    /// Complete the typed waiter a browser state names, then hand the
    /// observation to the observer outside every correlation lock. Browser
    /// observations name accounts: they never enter the shared diagnostics
    /// history.
    pub(super) fn record_browser(
        &self,
        mut live: RwLockWriteGuard<'_, LiveState>,
        token: &ConnectionToken,
        browser: &machine_browser::Event,
    ) {
        if let machine_browser::Event::State(state) = browser
            && let Some(request_id) = state.request_id.as_deref()
        {
            live.complete(
                token,
                request_id,
                Reply::Browser(Ok(Box::new(state.clone()))),
            );
        }
        drop(live);
        let observer = self.browser_observer.read().clone();
        if let Some(observer) = observer {
            observer(&token.0.machine_id, browser);
        }
    }
}

impl LiveState {
    /// A `CommandResult` answering a browser command the Machine refused
    /// outright completes its typed waiter with the detail and nothing else.
    pub(super) fn refuse_browser(
        &mut self,
        token: &ConnectionToken,
        request_id: &str,
        detail: Option<&str>,
    ) -> bool {
        let refused = self.pending.get(request_id).is_some_and(|request| {
            request.connection.same(token) && request.kind == ReplyKind::Browser
        });
        if refused {
            let detail = detail
                .unwrap_or("Machine browser command rejected")
                .to_owned();
            self.complete(token, request_id, Reply::Browser(Err(detail)));
        }
        refused
    }
}

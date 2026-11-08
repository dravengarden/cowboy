//! Process-local fence for background observation. This is not a Controller
//! continuation, project trust, execution binding or authority to deliver.
use super::*;

#[derive(Default)]
pub(super) struct Lifetime {
    changed: Arc<tokio::sync::Notify>,
}

impl Drop for Lifetime {
    fn drop(&mut self) {
        self.changed.notify_waiters();
    }
}

pub struct LiveGrant {
    host: Weak<CallHost>,
    parent: String,
    installed: Weak<Lifetime>,
    connection: Weak<Lifetime>,
    installed_changed: Arc<tokio::sync::Notify>,
    connection_changed: Arc<tokio::sync::Notify>,
    transport: mpsc::UnboundedSender<MachineEvent>,
}

impl LiveGrant {
    /// Wake cleanup immediately without retaining either authority lifetime.
    /// Register before checking so revocation cannot fall between those steps.
    pub async fn invalidated(&self) {
        let installed = self.installed_changed.notified();
        let connection = self.connection_changed.notified();
        tokio::pin!(installed, connection);
        installed.as_mut().enable();
        connection.as_mut().enable();
        if !self.current() {
            return;
        }
        tokio::select! {
            () = installed => {},
            () = connection => {},
            () = self.transport.closed() => {},
        }
    }

    /// Both exact lifetimes must remain installed. Disconnect/reconnect and
    /// revoke/reinstall permanently end this fence, even when IDs are reused.
    pub fn current(&self) -> bool {
        let Some(host) = self.host.upgrade() else {
            return false;
        };
        let installed = host.installed.lock();
        let uplink = host.uplink.lock();
        installed.get(&self.parent).is_some_and(|grant| {
            grant.agent_api && self.installed.ptr_eq(&Arc::downgrade(&grant.lifetime))
        }) && uplink.as_ref().is_some_and(|connection| {
            !connection.events.is_closed()
                && self
                    .connection
                    .ptr_eq(&Arc::downgrade(&connection.lifetime))
        })
    }
}

impl CallHost {
    /// Mint only from an installed Agent API grant and the current connected
    /// transport. The caller must separately validate its standing continuation.
    pub fn live_grant(self: &Arc<Self>, parent: &str, grant_id: &str) -> Option<LiveGrant> {
        let installed = self.installed.lock();
        let grant = installed.get(parent)?;
        if grant.grant_id != grant_id || !grant.agent_api {
            return None;
        }
        let uplink = self.uplink.lock();
        let connection = uplink.as_ref().filter(|value| !value.events.is_closed())?;
        Some(LiveGrant {
            host: Arc::downgrade(self),
            parent: parent.to_owned(),
            installed: Arc::downgrade(&grant.lifetime),
            connection: Arc::downgrade(&connection.lifetime),
            installed_changed: Arc::clone(&grant.lifetime.changed),
            connection_changed: Arc::clone(&connection.lifetime.changed),
            transport: connection.events.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fences_never_resume_after_reinstall_reconnect_or_closed_transport() {
        let root = tempfile::tempdir().unwrap();
        let host = CallHost::new(root.path()).unwrap();
        let grant = Grant {
            grant_id: "observer-grant".into(),
            parent_session_id: "parent".into(),
            agent_api: true,
            agent_actions: false,
            mcp: false,
        };
        host.install(&grant).unwrap();
        assert!(host.live_grant("parent", &grant.grant_id).is_none());
        let (tx, rx) = mpsc::unbounded_channel();
        let connection = host.attach(tx);
        let fence = host.live_grant("parent", &grant.grant_id).unwrap();
        assert!(fence.current());
        assert!(host.live_grant("other", &grant.grant_id).is_none());
        assert!(host.live_grant("parent", "other").is_none());
        host.install(&grant).unwrap();
        assert!(fence.current());
        let mut conflict = grant.clone();
        conflict.agent_api = false;
        assert!(host.install(&conflict).is_err());
        assert!(fence.current());
        host.revoke(&grant);
        assert!(!fence.current());
        tokio::time::timeout(Duration::from_secs(1), fence.invalidated())
            .await
            .unwrap();
        host.install(&grant).unwrap();
        assert!(!fence.current());
        let second = host.live_grant("parent", &grant.grant_id).unwrap();
        let (tx2, rx2) = mpsc::unbounded_channel();
        let replacement = host.attach(tx2);
        assert!(!second.current());
        let third = host.live_grant("parent", &grant.grant_id).unwrap();
        drop(connection);
        assert!(third.current());
        drop(rx2);
        assert!(!third.current());
        assert!(host.live_grant("parent", &grant.grant_id).is_none());
        drop(replacement);
        drop(rx);
        let mut denied = grant.clone();
        denied.grant_id = "without-agent-api".into();
        denied.agent_api = false;
        host.install(&denied).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let _connection = host.attach(tx);
        assert!(host.live_grant("parent", &denied.grant_id).is_none());
    }

    #[tokio::test]
    async fn already_waiting_observers_wake_on_grant_replacement() {
        let root = tempfile::tempdir().unwrap();
        let host = CallHost::new(root.path()).unwrap();
        let mut grant = Grant {
            grant_id: "before".into(),
            parent_session_id: "parent".into(),
            agent_api: true,
            agent_actions: false,
            mcp: false,
        };
        host.install(&grant).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let _connection = host.attach(tx);
        let fence = host.live_grant("parent", "before").unwrap();
        let waiting = fence.invalidated();
        tokio::pin!(waiting);
        assert!(futures::poll!(&mut waiting).is_pending());
        grant.grant_id = "after".into();
        host.install(&grant).unwrap();
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap();
        assert!(!fence.current());
    }
}

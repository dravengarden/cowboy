//! A live enrolled connection owns a bounded MCP process lifetime. Wire
//! profiles are declarations; they cannot reconstruct this lease or owner.
use super::{Arc, AtomicBool, Ordering, PluginExecutionScope};
use crate::mcp_binding::ProcessProfile;
use anyhow::{Result, ensure};
use tokio::sync::watch;
use tokio::time::{Duration, Instant};

pub struct McpInvocation {
    profile: ProcessProfile,
    connected: Arc<AtomicBool>,
    deadline: watch::Receiver<Option<Instant>>,
}

pub struct McpOwner {
    deadline: watch::Sender<Option<Instant>>,
    profile: ProcessProfile,
    connected: Arc<AtomicBool>,
}

impl McpOwner {
    /// Renewal preserves the original scope and never restores an ended lease.
    pub fn renew(&self, profile: &ProcessProfile) -> Result<()> {
        ensure!(profile == &self.profile, "MCP renewal profile changed");
        ensure!(!self.deadline.is_closed(), "MCP process ended");
        let renewed = self.deadline.send_if_modified(|deadline| {
            let now = Instant::now();
            if self.connected.load(Ordering::Acquire)
                && deadline.is_some_and(|expires| now < expires)
            {
                *deadline = Some(now + Duration::from_secs(30));
                true
            } else {
                false
            }
        });
        ensure!(renewed, "MCP authorization ended");
        Ok(())
    }

    pub fn revoke(&self) {
        self.deadline.send_replace(None);
    }
}

impl PluginExecutionScope {
    pub fn mcp(&self, profile: ProcessProfile) -> Result<(McpInvocation, McpOwner)> {
        profile.validate()?;
        ensure!(
            self.connected.load(Ordering::Acquire)
                && self.service.as_deref() == Some(profile.service_id.as_str())
                && self.machine == profile.execution_machine,
            "MCP connection owner changed"
        );
        let (sender, deadline) = watch::channel(Some(Instant::now() + Duration::from_secs(30)));
        Ok((
            McpInvocation {
                profile: profile.clone(),
                connected: Arc::clone(&self.connected),
                deadline,
            },
            McpOwner {
                deadline: sender,
                profile,
                connected: Arc::clone(&self.connected),
            },
        ))
    }
}

impl McpInvocation {
    pub fn profile(&self) -> &ProcessProfile {
        &self.profile
    }

    pub fn check(&self) -> Result<()> {
        ensure!(
            self.connected.load(Ordering::Acquire)
                && self.deadline.has_changed().is_ok()
                && self
                    .deadline
                    .borrow()
                    .is_some_and(|deadline| Instant::now() < deadline),
            "MCP authorization ended"
        );
        Ok(())
    }

    pub async fn ended(&self) {
        let mut deadline = self.deadline.clone();
        let mut connection = tokio::time::interval(Duration::from_millis(25));
        loop {
            if self.check().is_err() {
                return;
            }
            let Some(expires) = *deadline.borrow_and_update() else {
                return;
            };
            tokio::select! {
                biased;
                _ = deadline.changed() => {},
                () = tokio::time::sleep_until(expires) => {},
                _ = connection.tick() => {},
            }
        }
    }
}

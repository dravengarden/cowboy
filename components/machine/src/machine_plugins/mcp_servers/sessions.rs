//! One connection owns every opaque stdio handle. Handles are neither durable
//! nor transferable to a replacement connection; losing an owner revokes reads.
use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use serde_json::Value;

use super::super::{MachinePluginStore, McpOwner, PluginExecutionScope};
use super::McpProcess;
use crate::mcp_binding::ProcessProfile;

struct Entry {
    owner: McpOwner,
    process: Option<Arc<McpProcess>>,
}

pub struct McpSessions {
    scope: std::sync::Weak<PluginExecutionScope>,
    entries: Mutex<HashMap<String, Entry>>,
    descriptors: super::descriptors::Descriptors,
}

/// Cancellation during launch removes the reservation and drops its owner.
struct Opening<'a> {
    sessions: &'a McpSessions,
    handle: Option<String>,
}

impl Drop for Opening<'_> {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            self.sessions.close(handle);
        }
    }
}

impl McpSessions {
    pub fn new(scope: &Arc<PluginExecutionScope>) -> Self {
        Self {
            scope: Arc::downgrade(scope),
            entries: Mutex::new(HashMap::new()),
            descriptors: super::descriptors::Descriptors::default(),
        }
    }

    pub fn confirm_native(&self, handle: &str) -> bool {
        self.scope.upgrade().is_some() && self.descriptors.confirm(handle)
    }

    pub fn release_native(&self, handle: &str) -> bool {
        self.descriptors.release(handle)
    }

    pub async fn prepare_native(
        &self,
        store: &MachinePluginStore,
        profile: ProcessProfile,
    ) -> Result<Value> {
        let (invocation, _owner) = self
            .scope
            .upgrade()
            .context("MCP connection ended")?
            .mcp(profile)?;
        self.descriptors.prepare(store, invocation).await
    }

    pub async fn open(
        &self,
        store: &MachinePluginStore,
        profile: ProcessProfile,
    ) -> Result<String> {
        let (invocation, owner) = self
            .scope
            .upgrade()
            .context("MCP connection ended")?
            .mcp(profile)?;
        let handle = format!("mcp-{:032x}", rand::random::<u128>());
        {
            let mut entries = self.entries.lock();
            entries.retain(|_, entry| {
                entry
                    .process
                    .as_ref()
                    .is_none_or(|process| !process.closed())
            });
            ensure!(entries.len() < 8, "MCP connection capacity");
            ensure!(!entries.contains_key(&handle), "MCP handle collision");
            entries.insert(
                handle.clone(),
                Entry {
                    owner,
                    process: None,
                },
            );
        }
        let mut opening = Opening {
            sessions: self,
            handle: Some(handle.clone()),
        };
        let process = store.open_mcp(invocation).await?;
        let mut entries = self.entries.lock();
        let entry = entries.get_mut(&handle).context("MCP opening cancelled")?;
        entry.process = Some(Arc::new(process));
        opening.handle = None;
        Ok(handle)
    }

    pub async fn request(&self, handle: &str, message: Value) -> Result<Value> {
        let process = self
            .entries
            .lock()
            .get(handle)
            .and_then(|entry| entry.process.clone())
            .context("MCP handle unavailable")?;
        process.request(message).await
    }

    pub fn renew(&self, handle: &str, profile: &ProcessProfile) -> Result<()> {
        let entries = self.entries.lock();
        let entry = entries.get(handle).context("MCP handle unavailable")?;
        ensure!(
            entry
                .process
                .as_ref()
                .is_some_and(|process| !process.closed()),
            "MCP process ended"
        );
        entry.owner.renew(profile)
    }

    /// Removing the owner cancels in-flight calls even while they retain I/O.
    pub fn close(&self, handle: &str) {
        if let Some(entry) = self.entries.lock().remove(handle) {
            entry.owner.revoke();
        }
    }
}

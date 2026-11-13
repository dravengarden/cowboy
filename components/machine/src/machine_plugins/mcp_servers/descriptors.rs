//! Private, connection-owned startup metadata. Files carry no execution authority
//! and disappear when their last receipt on any connection releases them. Paths
//! are content-addressed so a Controller restart re-prepares the same plan.
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use anyhow::{Result, ensure};
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::super::{MachinePluginStore, McpInvocation};
use crate::mcp_binding::{NativeDescriptor, ProcessProfile};

struct Entry {
    profile: ProcessProfile,
    descriptor: NativeDescriptor,
    file: Arc<super::descriptor_files::DescriptorFile>,
}

#[derive(Default)]
struct State {
    receipts: HashMap<String, Arc<Entry>>,
    pending: HashMap<String, tokio::time::Instant>,
    sweeping: bool,
    reusable: HashMap<(String, String), Weak<Entry>>,
}

#[derive(Default)]
pub(super) struct Descriptors(Arc<Mutex<State>>);

impl Descriptors {
    pub(super) async fn prepare(
        &self,
        store: &MachinePluginStore,
        invocation: McpInvocation,
    ) -> Result<Value> {
        invocation.check()?;
        let _lifecycle = tokio::select! {
            biased;
            () = invocation.ended() => anyhow::bail!("MCP authorization ended"),
            guard = store.lifecycle.lock() => guard,
        };
        let profile = invocation.profile();
        let descriptor = store
            .prepare_mcp_binding(&profile.binding)?
            .native_descriptor(&profile.epoch)?;
        let key = (profile.session_id.clone(), profile.binding.name.clone());
        let mut state = self.0.lock();
        invocation.check()?;
        expire(&mut state);
        state.reusable.retain(|_, entry| entry.strong_count() > 0);
        ensure!(state.receipts.len() < 256, "MCP descriptor capacity");
        let handle = format!("native-{:032x}", rand::random::<u128>());
        ensure!(
            !state.receipts.contains_key(&handle),
            "MCP descriptor handle collision"
        );
        let existing = state
            .reusable
            .get(&key)
            .and_then(Weak::upgrade)
            .filter(|entry| entry.profile == *profile && entry.descriptor == descriptor);
        let entry = if let Some(entry) = existing {
            entry
        } else {
            let file = super::descriptor_files::DescriptorFile::acquire(
                &store.root,
                profile,
                &descriptor,
            )?;
            Arc::new(Entry {
                profile: profile.clone(),
                descriptor,
                file,
            })
        };
        invocation.check()?;
        let response =
            json!({"handle":handle, "path":entry.file.path(), "descriptor":entry.descriptor});
        state.reusable.insert(key, Arc::downgrade(&entry));
        state.pending.insert(
            handle.clone(),
            tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        );
        state.receipts.insert(handle, entry);
        if !state.sweeping {
            state.sweeping = true;
            let weak = Arc::downgrade(&self.0);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let Some(state) = weak.upgrade() else { break };
                    let mut state = state.lock();
                    expire(&mut state);
                    if state.pending.is_empty() {
                        state.sweeping = false;
                        break;
                    }
                }
            });
        }
        Ok(response)
    }

    pub(super) fn confirm(&self, handle: &str) -> bool {
        let mut state = self.0.lock();
        expire(&mut state);
        state.pending.remove(handle);
        state.receipts.contains_key(handle)
    }

    pub(super) fn release(&self, handle: &str) -> bool {
        let mut state = self.0.lock();
        state.pending.remove(handle);
        let removed = state.receipts.remove(handle).is_some();
        state.reusable.retain(|_, entry| entry.strong_count() > 0);
        removed
    }
}

fn expire(state: &mut State) {
    let now = tokio::time::Instant::now();
    state.pending.retain(|handle, deadline| {
        if *deadline <= now {
            state.receipts.remove(handle);
            false
        } else {
            true
        }
    });
    state.reusable.retain(|_, entry| entry.strong_count() > 0);
}

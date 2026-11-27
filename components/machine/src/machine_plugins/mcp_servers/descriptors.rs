//! Private, connection-owned startup metadata. Files carry no execution authority
//! and disappear when their last receipt on any connection releases them. Paths
//! are content-addressed so a Controller restart re-prepares the same plan.
use std::collections::HashMap;
use std::sync::{Arc, Weak};

use anyhow::{Context as _, Result};
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
        let lifecycle = tokio::select! {
            biased;
            () = invocation.ended() => anyhow::bail!("MCP authorization ended"),
            guard = Arc::clone(&store.lifecycle).lock_owned() => guard,
        };
        let profile = invocation.profile();
        let (descriptor, _lifecycle) = {
            let store = store.clone();
            let binding = profile.binding.clone();
            let epoch = profile.epoch.clone();
            let (result, lifecycle) = tokio::task::spawn_blocking(move || {
                let result = store
                    .prepare_mcp_binding(&binding)
                    .and_then(|prepared| prepared.native_descriptor(&epoch));
                (result, lifecycle)
            })
            .await
            .context("MCP native descriptor preparation task failed")?;
            (result?, lifecycle)
        };
        let key = (profile.session_id.clone(), profile.binding.name.clone());
        let (existing, expired, pre_err) = {
            let mut state = self.0.lock();
            let expired = expire(&mut state);
            if let Err(error) = invocation.check() {
                (None, expired, Some(error))
            } else if state.receipts.len() >= 256 {
                (
                    None,
                    expired,
                    Some(anyhow::anyhow!("MCP descriptor capacity")),
                )
            } else {
                let existing = state
                    .reusable
                    .get(&key)
                    .and_then(Weak::upgrade)
                    .filter(|entry| entry.profile == *profile && entry.descriptor == descriptor);
                (existing, expired, None)
            }
        };
        drop_entries_async(expired).await;
        if let Some(error) = pre_err {
            return Err(error);
        }
        let candidate = if let Some(entry) = existing {
            entry
        } else {
            let scope = store.root.clone();
            let profile_owned = profile.clone();
            let descriptor_owned = descriptor.clone();
            let file = tokio::task::spawn_blocking(move || {
                super::descriptor_files::DescriptorFile::acquire(
                    &scope,
                    &profile_owned,
                    &descriptor_owned,
                )
            })
            .await
            .context("MCP descriptor file acquisition task failed")??;
            Arc::new(Entry {
                profile: profile.clone(),
                descriptor: descriptor.clone(),
                file,
            })
        };
        let (result, to_drop) =
            self.register_candidate(key, profile, &descriptor, candidate, &invocation);
        drop_entries_async(to_drop).await;
        result
    }

    fn register_candidate(
        &self,
        key: (String, String),
        profile: &ProcessProfile,
        descriptor: &NativeDescriptor,
        candidate: Arc<Entry>,
        invocation: &McpInvocation,
    ) -> (Result<Value>, Vec<Arc<Entry>>) {
        let mut state = self.0.lock();
        let mut to_drop = expire(&mut state);
        if let Err(error) = invocation.check() {
            to_drop.push(candidate);
            return (Err(error), to_drop);
        }
        if state.receipts.len() >= 256 {
            to_drop.push(candidate);
            return (Err(anyhow::anyhow!("MCP descriptor capacity")), to_drop);
        }
        let handle = format!("native-{:032x}", rand::random::<u128>());
        if state.receipts.contains_key(&handle) {
            to_drop.push(candidate);
            return (
                Err(anyhow::anyhow!("MCP descriptor handle collision")),
                to_drop,
            );
        }
        let entry = if let Some(live) = state
            .reusable
            .get(&key)
            .and_then(Weak::upgrade)
            .filter(|entry| entry.profile == *profile && entry.descriptor == *descriptor)
        {
            if !Arc::ptr_eq(&live, &candidate) {
                to_drop.push(candidate);
            }
            live
        } else {
            candidate
        };
        let response = json!({
            "handle": handle,
            "path": entry.file.path(),
            "descriptor": entry.descriptor,
        });
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
                    let (expired, done) = {
                        let mut state = state.lock();
                        let expired = expire(&mut state);
                        let done = state.pending.is_empty();
                        if done {
                            state.sweeping = false;
                        }
                        (expired, done)
                    };
                    drop_entries_async(expired).await;
                    if done {
                        break;
                    }
                }
            });
        }
        (Ok(response), to_drop)
    }

    pub(super) async fn confirm_async(&self, handle: &str) -> bool {
        let (confirmed, expired) = {
            let mut state = self.0.lock();
            let expired = expire(&mut state);
            state.pending.remove(handle);
            (state.receipts.contains_key(handle), expired)
        };
        drop_entries_async(expired).await;
        confirmed
    }

    pub(super) async fn release_async(&self, handle: &str) -> bool {
        let (removed, dropped) = {
            let mut state = self.0.lock();
            state.pending.remove(handle);
            let dropped = state.receipts.remove(handle);
            state.reusable.retain(|_, entry| entry.strong_count() > 0);
            (dropped.is_some(), dropped)
        };
        if let Some(entry) = dropped {
            drop_entries_async(vec![entry]).await;
        }
        removed
    }

    #[cfg(test)]
    pub(super) fn confirm(&self, handle: &str) -> bool {
        let (confirmed, expired) = {
            let mut state = self.0.lock();
            let expired = expire(&mut state);
            state.pending.remove(handle);
            (state.receipts.contains_key(handle), expired)
        };
        drop(expired);
        confirmed
    }

    #[cfg(test)]
    pub(super) fn release(&self, handle: &str) -> bool {
        let (removed, dropped) = {
            let mut state = self.0.lock();
            state.pending.remove(handle);
            let dropped = state.receipts.remove(handle);
            state.reusable.retain(|_, entry| entry.strong_count() > 0);
            (dropped.is_some(), dropped)
        };
        drop(dropped);
        removed
    }
}

fn expire(state: &mut State) -> Vec<Arc<Entry>> {
    let now = tokio::time::Instant::now();
    let mut dropped = Vec::new();
    state.pending.retain(|handle, deadline| {
        if *deadline <= now {
            if let Some(entry) = state.receipts.remove(handle) {
                dropped.push(entry);
            }
            false
        } else {
            true
        }
    });
    state.reusable.retain(|_, entry| entry.strong_count() > 0);
    dropped
}

async fn drop_entries_async(entries: Vec<Arc<Entry>>) {
    if entries.is_empty() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || drop(entries)).await;
}

impl Drop for Descriptors {
    fn drop(&mut self) {
        let drained: Vec<Arc<Entry>> = {
            let mut state = self.0.lock();
            state.pending.clear();
            state.reusable.clear();
            state.receipts.drain().map(|(_, entry)| entry).collect()
        };
        if !drained.is_empty() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn_blocking(move || drop(drained));
            } else {
                drop(drained);
            }
        }
    }
}

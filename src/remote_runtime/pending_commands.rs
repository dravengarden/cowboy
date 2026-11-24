//! Versioned pending-command map for `RemoteRuntime`.
//!
//! Storing each command's monotonic generation alongside the `CoreCommand`
//! ensures removal (`remove`, `retain`, `clear`, `drain_commands`) frees its
//! version entry automatically with zero leak and zero lock-order overhead.

use std::collections::HashMap;
use std::ops::Index;

use crate::runtime_wire::CoreCommand;

#[derive(Default)]
pub(in crate::remote_runtime) struct PendingCommands {
    next_version: u64,
    entries: HashMap<String, (u64, CoreCommand)>,
}

impl PendingCommands {
    pub(in crate::remote_runtime) fn insert(&mut self, key: String, command: CoreCommand) -> u64 {
        self.next_version = self.next_version.saturating_add(1);
        let version = self.next_version;
        self.entries.insert(key, (version, command));
        version
    }

    /// Update an existing command in place ONLY when its version still matches
    /// `expected_version` (used when `reset_admission::current` refreshes an
    /// `EnsureSession`'s MCP launch plan right before writing the current
    /// snapshot). Returns `false` without mutating if the command was removed
    /// or superseded by a newer insert.
    pub(in crate::remote_runtime) fn update_if_version(
        &mut self,
        key: &str,
        expected_version: u64,
        command: CoreCommand,
    ) -> bool {
        let Some((current_version, existing)) = self.entries.get_mut(key) else {
            return false;
        };
        if *current_version != expected_version {
            return false;
        }
        *existing = command;
        true
    }

    pub(in crate::remote_runtime) fn remove(&mut self, key: &str) -> Option<CoreCommand> {
        self.entries.remove(key).map(|(_, command)| command)
    }

    pub(in crate::remote_runtime) fn remove_if_version(
        &mut self,
        key: &str,
        expected_version: u64,
    ) -> Option<CoreCommand> {
        if self
            .entries
            .get(key)
            .is_some_and(|(current_version, _)| *current_version == expected_version)
        {
            self.entries.remove(key).map(|(_, command)| command)
        } else {
            None
        }
    }

    pub(in crate::remote_runtime) fn get(&self, key: &str) -> Option<&CoreCommand> {
        self.entries.get(key).map(|(_, command)| command)
    }

    pub(in crate::remote_runtime) fn version(&self, key: &str) -> Option<u64> {
        self.entries.get(key).map(|(version, _)| *version)
    }

    pub(in crate::remote_runtime) fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::remote_runtime) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(in crate::remote_runtime) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(in crate::remote_runtime) fn values(&self) -> impl Iterator<Item = &CoreCommand> {
        self.entries.values().map(|(_, command)| command)
    }

    pub(in crate::remote_runtime) fn iter(&self) -> impl Iterator<Item = (&String, &CoreCommand)> {
        self.entries
            .iter()
            .map(|(key, (_, command))| (key, command))
    }

    pub(in crate::remote_runtime) fn iter_with_versions(
        &self,
    ) -> impl Iterator<Item = (&String, u64, &CoreCommand)> {
        self.entries
            .iter()
            .map(|(key, (version, command))| (key, *version, command))
    }

    pub(in crate::remote_runtime) fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&String, &mut CoreCommand) -> bool,
    {
        self.entries.retain(|key, (_, command)| f(key, command));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(in crate::remote_runtime) fn drain_commands(&mut self) -> HashMap<String, CoreCommand> {
        self.entries
            .drain()
            .map(|(key, (_, command))| (key, command))
            .collect()
    }

    #[cfg(test)]
    pub(in crate::remote_runtime) fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Index<&String> for PendingCommands {
    type Output = CoreCommand;

    fn index(&self, key: &String) -> &Self::Output {
        &self.entries[key].1
    }
}

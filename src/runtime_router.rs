//! Machine-aware registry for detached ACP runtimes.

#![warn(clippy::pedantic)]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::remote_runtime::RemoteRuntime;

/// Routes immutable session placement to the latest authenticated connection
/// for that machine.
pub struct RuntimeRouter {
    runtimes: RwLock<HashMap<String, Arc<RemoteRuntime>>>,
}

impl RuntimeRouter {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            runtimes: RwLock::new(HashMap::new()),
        })
    }

    #[must_use]
    pub fn runtime(&self, machine_id: &str) -> Option<Arc<RemoteRuntime>> {
        self.runtimes.read().get(machine_id).cloned()
    }

    #[must_use]
    pub fn connected(&self, machine_id: &str) -> bool {
        self.runtime(machine_id)
            .is_some_and(|runtime| runtime.connected())
    }

    #[must_use]
    pub fn has_connected_runtime(&self) -> bool {
        self.runtimes
            .read()
            .values()
            .any(|runtime| runtime.connected())
    }

    #[must_use]
    pub fn stats(&self) -> crate::remote_runtime::RemoteRuntimeStats {
        self.runtimes
            .read()
            .values()
            .map(|runtime| runtime.stats())
            .fold(
                crate::remote_runtime::RemoteRuntimeStats::default(),
                |mut total, stats| {
                    total.workers += stats.workers;
                    total.busy_workers += stats.busy_workers;
                    total.draining_workers += stats.draining_workers;
                    total.handoff_workers += stats.handoff_workers;
                    total.pending_commands += stats.pending_commands;
                    total
                },
            )
    }

    pub fn install(&self, machine_id: String, runtime: Arc<RemoteRuntime>) {
        if let Some(previous) = self.runtimes.write().insert(machine_id, runtime) {
            previous.disconnect();
        }
    }

    /// Fence and remove whichever runtime currently owns this Machine id.
    /// Administrative identity revocation uses this form because it is not
    /// tied to one connection epoch.
    pub fn remove(&self, machine_id: &str) {
        if let Some(runtime) = self.runtimes.write().remove(machine_id) {
            runtime.disconnect();
        }
    }

    pub fn remove_if_current(&self, machine_id: &str, runtime: &Arc<RemoteRuntime>) {
        let mut runtimes = self.runtimes.write();
        if runtimes
            .get(machine_id)
            .is_some_and(|current| Arc::ptr_eq(current, runtime))
        {
            runtimes.remove(machine_id);
            runtime.disconnect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Hub;

    #[test]
    fn empty_router_does_not_synthesize_a_local_runtime() {
        let router = RuntimeRouter::new();
        assert!(router.runtime("local").is_none());
        assert!(router.runtime("hawk").is_none());
    }

    #[tokio::test]
    async fn colocated_machine_uses_the_same_registry_as_remote_machines() {
        let router = RuntimeRouter::new();
        let hawk = RemoteRuntime::for_test(Hub::new(), Vec::new());
        let falcon = RemoteRuntime::for_test(Hub::new(), Vec::new());
        router.install("hawk".to_owned(), Arc::clone(&hawk));
        router.install("falcon".to_owned(), Arc::clone(&falcon));
        assert!(Arc::ptr_eq(&router.runtime("hawk").expect("hawk"), &hawk));
        assert!(Arc::ptr_eq(
            &router.runtime("falcon").expect("falcon"),
            &falcon
        ));
    }

    #[tokio::test]
    async fn stale_disconnect_cannot_remove_replacement_runtime() {
        let router = RuntimeRouter::new();
        let first = RemoteRuntime::for_test(Hub::new(), Vec::new());
        let second = RemoteRuntime::for_test(Hub::new(), Vec::new());
        router.install("falcon".to_owned(), Arc::clone(&first));
        router.install("falcon".to_owned(), Arc::clone(&second));
        router.remove_if_current("falcon", &first);
        assert!(Arc::ptr_eq(
            &router.runtime("falcon").expect("replacement runtime"),
            &second
        ));
        router.remove_if_current("falcon", &second);
        assert!(router.runtime("falcon").is_none());
    }

    #[tokio::test]
    async fn explicit_remove_fences_the_current_runtime() {
        let router = RuntimeRouter::new();
        let runtime = RemoteRuntime::for_test(Hub::new(), Vec::new());
        router.install("macbook-air".to_owned(), Arc::clone(&runtime));

        router.remove("macbook-air");

        assert!(router.runtime("macbook-air").is_none());
        assert!(!runtime.connected());
    }
}

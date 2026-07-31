//! Machine-aware registry for detached ACP runtimes.

#![warn(clippy::pedantic)]

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::remote_runtime::RemoteRuntime;

/// Routes immutable session placement to the latest authenticated connection
/// for that machine.
pub struct RuntimeRouter {
    local: Arc<RemoteRuntime>,
    remote: RwLock<HashMap<String, Arc<RemoteRuntime>>>,
}

impl RuntimeRouter {
    #[must_use]
    pub fn new(local: Arc<RemoteRuntime>) -> Arc<Self> {
        Arc::new(Self {
            local,
            remote: RwLock::new(HashMap::new()),
        })
    }

    #[must_use]
    pub fn runtime(&self, machine_id: &str) -> Option<Arc<RemoteRuntime>> {
        if machine_id == "local" {
            return Some(Arc::clone(&self.local));
        }
        self.remote.read().get(machine_id).cloned()
    }

    #[must_use]
    pub fn connected(&self, machine_id: &str) -> bool {
        self.runtime(machine_id)
            .is_some_and(|runtime| runtime.connected())
    }

    pub fn install(&self, machine_id: String, runtime: Arc<RemoteRuntime>) {
        if machine_id != "local"
            && let Some(previous) = self.remote.write().insert(machine_id, runtime)
        {
            previous.disconnect();
        }
    }

    pub fn remove_if_current(&self, machine_id: &str, runtime: &Arc<RemoteRuntime>) {
        let mut remote = self.remote.write();
        if remote
            .get(machine_id)
            .is_some_and(|current| Arc::ptr_eq(current, runtime))
        {
            remote.remove(machine_id);
            runtime.disconnect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Hub;

    #[tokio::test]
    async fn stale_disconnect_cannot_remove_replacement_runtime() {
        let local = RemoteRuntime::for_test(Hub::new(), Vec::new());
        let router = RuntimeRouter::new(local);
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
}

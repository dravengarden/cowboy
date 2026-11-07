//! Process-local receipts prevent an uncertain Start from creating a second
//! observer. Receipt exhaustion refuses admission; replay fences are not evicted.
use super::*;
use crate::code_observer::worker::{Capacity, Worker};
use crate::machine_protocol::code_observer::{Continuation, Request};

const MAX_RECEIPTS: usize = 1024;

/// Capture before spawning: a queued Start cannot borrow a later reconnect.
pub(super) fn capture(
    payload: &serde_json::Value,
    calls: Option<&Arc<crate::managed_calls::host::CallHost>>,
) -> Option<Arc<crate::managed_calls::host::LiveGrant>> {
    let request: Request = serde_json::from_value(payload.clone()).ok()?;
    if !request.valid() {
        return None;
    }
    let key = request.continuation();
    calls?
        .live_grant(&key.parent_session_id, &key.grant_id)
        .map(Arc::new)
}

struct Entry {
    continuation: Continuation,
    worker: Option<Arc<Worker>>,
}

#[derive(Default)]
pub(super) struct Registry {
    entries: parking_lot::Mutex<BTreeMap<String, Entry>>,
    capacity: Capacity,
}

impl Registry {
    fn access(
        &self,
        key: &Continuation,
        close: bool,
        start: impl FnOnce() -> anyhow::Result<Worker>,
    ) -> anyhow::Result<Option<Arc<Worker>>> {
        let mut entries = self.entries.lock();
        for entry in entries.values_mut() {
            if entry
                .worker
                .as_ref()
                .is_some_and(|worker| worker.finished())
            {
                entry.worker = None;
            }
        }
        if let Some(entry) = entries.get(&key.id) {
            anyhow::ensure!(entry.continuation == *key, "observer identity conflict");
            if close && let Some(worker) = &entry.worker {
                worker.cancel();
            }
            return Ok(entry.worker.clone());
        }
        anyhow::ensure!(
            entries.len() < MAX_RECEIPTS,
            "observer receipt capacity exhausted"
        );
        // Reserve before launch, including failed launches and Close-before-Start.
        let entry = entries.entry(key.id.clone()).or_insert_with(|| Entry {
            continuation: key.clone(),
            worker: None,
        });
        if !close {
            entry.worker = Some(Arc::new(start()?));
        }
        Ok(entry.worker.clone())
    }
}

fn pin(
    key: &Continuation,
    configuration: &WorkspaceConfig,
    worktrees: &crate::session_workspace::WorktreeRoots,
) -> anyhow::Result<workspace_identity::SessionRootPin> {
    let _configuration = configuration.identities.lock();
    // Verify first so even a temporarily missing pathname permanently retires
    // its old identity. Canonicalization failure must not bypass that fence.
    let pin = configuration
        .session_identities
        .lock()
        .pin(&key.root, &key.root_incarnation)?;
    let canonical = PathBuf::from(&key.root).canonicalize()?;
    anyhow::ensure!(
        workspace_path_allowed(&canonical, &configuration.snapshot().workspaces, worktrees),
        "Session root is not trusted"
    );
    Ok(pin)
}

pub(super) async fn handle(
    payload: serde_json::Value,
    configuration: &WorkspaceConfig,
    worktrees: &crate::session_workspace::WorktreeRoots,
    grant: Option<&Arc<crate::managed_calls::host::LiveGrant>>,
) -> anyhow::Result<serde_json::Value> {
    let request: Request = serde_json::from_value(payload).context("invalid observer request")?;
    anyhow::ensure!(request.valid(), "invalid observer request");
    let key = request.continuation();
    let registry = &configuration.observers;
    if matches!(&request, Request::Close { .. }) {
        let worker = registry.access(key, true, || unreachable!())?;
        return Ok(
            serde_json::json!({"state": if worker.is_none_or(|w| w.finished()) { "closed" } else { "closing" }}),
        );
    }
    let grant = grant
        .filter(|grant| grant.current())
        .context("observer grant unavailable")?;
    let root = pin(key, configuration, worktrees)?;
    let starting = matches!(&request, Request::Start { .. });
    let worker = registry
        .access(key, false, || {
            anyhow::ensure!(starting, "observer has not started");
            Worker::start(
                root.file,
                PathBuf::from(&key.root),
                Arc::clone(grant),
                root.current,
                &registry.capacity,
            )
        })?
        .context("observer continuation ended")?;
    anyhow::ensure!(worker.is_current(), "observer continuation ended");
    let result = match &request {
        Request::Read { cursor, .. } => serde_json::to_value(worker.read(cursor.clone()).await?)?,
        Request::Start { .. } => serde_json::json!({"state": "started"}),
        Request::Close { .. } => unreachable!(),
    };
    // Filesystem/configuration and original grant are rechecked after awaiting.
    let _pin = pin(key, configuration, worktrees)?;
    anyhow::ensure!(worker.is_current(), "observer continuation ended");
    Ok(result)
}

#[cfg(test)]
mod tests;

//! Keep prompt admission bound to the original workspace directory.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WorkspaceIdentity {
    device: u64,
    inode: u64,
}

pub(super) fn workspace_identity(path: &std::path::Path) -> Option<WorkspaceIdentity> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(WorkspaceIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

impl Shared {
    /// The workspace was replaced or removed: report the crash it means and
    /// return why a prompt cannot run.
    pub(super) fn workspace_replaced(&self) -> Option<String> {
        if self.workspace_is_current() {
            return None;
        }
        let message = format!(
            "session workspace was replaced or removed: {}; retry after Cowboy recycles this worker",
            self.workspace_path.display()
        );
        self.emit(RuntimeEvent::Status {
            state: WorkerState::Crashed,
            detail: Some(message.clone()),
        });
        Some(message)
    }

    pub(super) fn workspace_is_current(&self) -> bool {
        self.workspace_identity.is_some()
            && workspace_identity(&self.workspace_path) == self.workspace_identity
    }
}

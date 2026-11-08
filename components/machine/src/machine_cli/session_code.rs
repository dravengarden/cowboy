//! Private Session envelopes terminate at the resident Machine.
use super::*;

pub(super) enum Envelope {
    Reply(serde_json::Value),
    Read {
        operation: serde_json::Value,
        root: String,
        incarnation: String,
    },
}

pub(super) fn admit(
    payload: serde_json::Value,
    configuration: &WorkspaceConfig,
    worktrees: &crate::session_workspace::WorktreeRoots,
) -> anyhow::Result<Envelope> {
    use crate::machine_protocol::session_code::Request;
    let request: Request =
        serde_json::from_value(payload).context("invalid Session Code request")?;
    let root = match &request {
        Request::Observe { root } | Request::Verify { root, .. } | Request::Read { root, .. } => {
            root
        }
    };
    anyhow::ensure!(
        root.len() <= 4096 && Path::new(root).is_absolute(),
        "invalid Session root"
    );
    let canonical = PathBuf::from(root).canonicalize()?;
    let _configuration = configuration.identities.lock();
    anyhow::ensure!(
        workspace_path_allowed(&canonical, &configuration.snapshot().workspaces, worktrees),
        "Session root is not trusted"
    );
    match &request {
        Request::Verify { incarnation, .. } | Request::Read { incarnation, .. } => {
            anyhow::ensure!(
                incarnation.len() == 32 && incarnation.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid Session root identity"
            );
        }
        Request::Observe { .. } => {}
    }
    let mut identities = configuration.session_identities.lock();
    match request {
        Request::Observe { root } => Ok(Envelope::Reply(
            serde_json::json!({"incarnation": identities.observe(&root)?}),
        )),
        Request::Verify { root, incarnation } => Ok(Envelope::Reply(
            serde_json::json!({"current": identities.verify(&root, &incarnation)}),
        )),
        Request::Read {
            root,
            incarnation,
            operation,
        } => {
            anyhow::ensure!(
                identities.verify(&root, &incarnation),
                "Session root changed"
            );
            let mut operation = operation
                .as_object()
                .context("invalid Code operation")?
                .clone();
            anyhow::ensure!(
                !operation.contains_key("root"),
                "operation cannot replace Session root"
            );
            operation.insert("root".into(), serde_json::Value::String(root.clone()));
            Ok(Envelope::Read {
                operation: serde_json::Value::Object(operation),
                root,
                incarnation,
            })
        }
    }
}

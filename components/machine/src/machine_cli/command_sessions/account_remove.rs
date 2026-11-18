//! The revocation's removal of an account's home and system user through
//! the Machine's account-sandbox daemon (`AccountSandboxRemove`).
use super::*;
use crate::provider_account::AccountRemoveRequest;

/// Remove a revoked account's home and user; only the daemon's receipt is
/// an observation.
pub(super) async fn remove(
    providers: Arc<MachinePluginStore>,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    request_id: String,
    remove: AccountRemoveRequest,
) {
    let result = tokio::task::spawn_blocking(move || providers.account_sandbox_remove(&remove))
        .await
        .map_err(anyhow::Error::from)
        .and_then(std::convert::identity);
    let (observation, detail) = match result {
        Ok(observation) => (Some(observation), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let _ = events.send(MachineEvent::AccountSandboxRemoveObserved {
        request_id,
        observation,
        detail,
    });
}

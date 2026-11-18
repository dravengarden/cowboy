//! Account-sandbox fences on a Machine (`AccountSandboxFence`): the
//! Controller's kill switch and suspension handling stop an account's units
//! through the Machine's `cowboy-account-sandbox` daemon, and clearing the
//! account admits it again (`docs/antigravity-real-accounts-design.md`).

use super::{MachineControl, Reply, ReplyKind};
use crate::machine_protocol::MachineCommand;
use crate::provider_account::{AccountFenceObservation, AccountFenceRequest};

/// The daemon stops every unit of the account with its own bounded stop
/// timeouts before it answers; allow for them.
const FENCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(125);

impl MachineControl {
    /// Ask `machine` to fence or admit one account in its daemon. Only the
    /// exact typed observation of this request confirms it; a lost reply is
    /// `account_fence_unconfirmed` and the caller retries the same request.
    pub async fn account_sandbox_fence(
        &self,
        machine: &str,
        fence: &AccountFenceRequest,
    ) -> Result<AccountFenceObservation, String> {
        if fence.account_id.is_default() {
            return Err("named_account_required".into());
        }
        let request_id = format!("account-fence-{}", uuid::Uuid::new_v4());
        let (rx, _pending) = self.begin_request(
            machine,
            &request_id,
            MachineCommand::AccountSandboxFence {
                request_id: request_id.clone(),
                fence: fence.clone(),
            },
            ReplyKind::AccountFence,
            None,
        )?;
        match tokio::time::timeout(FENCE_TIMEOUT, rx).await {
            Ok(Ok(Reply::AccountFence(Ok(observation)))) if observation.request == *fence => {
                Ok(observation)
            }
            Ok(Ok(Reply::AccountFence(Err(detail)))) if !detail.is_empty() => {
                Err(format!("account_fence_refused: {detail}"))
            }
            _ => Err("account_fence_unconfirmed".into()),
        }
    }
}

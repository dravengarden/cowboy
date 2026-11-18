//! Account-sandbox fences on a Machine (`AccountSandboxFence`): the
//! Controller's kill switch and suspension handling stop an account's units
//! through the Machine's `cowboy-account-sandbox` daemon, and clearing the
//! account admits it again (`docs/antigravity-real-accounts-design.md`).

use super::{MachineControl, Reply, ReplyKind};
use crate::machine_protocol::MachineCommand;
use crate::provider_account::{
    AccountFenceObservation, AccountFenceRequest, AccountRemoveObservation, AccountRemoveRequest,
    AccountTurnTerminationQuery, TargetTermination,
};

/// The daemon stops every unit of the account with its own bounded stop
/// timeouts before it answers; allow for them.
const FENCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(125);
/// The Machine reads its ledger and asks its daemon for the session's units.
const TERMINATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// `remove_account` stops the account's units first, with the same bounds as
/// a fence.
const REMOVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(125);

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

    /// Ask a session's runtime Machine whether the execution target of one
    /// dispatched turn is proven gone. Anything but the typed observation of
    /// this request is no evidence (`Err`), so the reservation keeps the slot.
    pub async fn account_turn_termination(
        &self,
        machine: &str,
        query: &AccountTurnTerminationQuery,
    ) -> Result<TargetTermination, String> {
        if query.account_id.is_default() {
            return Err("named_account_required".into());
        }
        let request_id = format!("account-turn-{}", uuid::Uuid::new_v4());
        let (rx, _pending) = self.begin_request(
            machine,
            &request_id,
            MachineCommand::QueryAccountTurnTermination {
                request_id: request_id.clone(),
                query: query.clone(),
            },
            ReplyKind::AccountTurnTermination,
            None,
        )?;
        match tokio::time::timeout(TERMINATION_TIMEOUT, rx).await {
            Ok(Ok(Reply::AccountTurnTermination(Ok(observation)))) => Ok(observation),
            Ok(Ok(Reply::AccountTurnTermination(Err(detail)))) if !detail.is_empty() => {
                Err(format!("account_turn_termination_unknown: {detail}"))
            }
            _ => Err("account_turn_termination_unconfirmed".into()),
        }
    }

    /// Ask `machine` to remove a revoked account's home and system user. Only
    /// the exact typed receipt of this request counts.
    pub async fn account_sandbox_remove(
        &self,
        machine: &str,
        remove: &AccountRemoveRequest,
    ) -> Result<AccountRemoveObservation, String> {
        if remove.account_id.is_default() {
            return Err("named_account_required".into());
        }
        let request_id = format!("account-remove-{}", uuid::Uuid::new_v4());
        let (rx, _pending) = self.begin_request(
            machine,
            &request_id,
            MachineCommand::AccountSandboxRemove {
                request_id: request_id.clone(),
                remove: remove.clone(),
            },
            ReplyKind::AccountRemove,
            None,
        )?;
        match tokio::time::timeout(REMOVE_TIMEOUT, rx).await {
            Ok(Ok(Reply::AccountRemove(Ok(observation)))) if observation.request == *remove => {
                Ok(observation)
            }
            Ok(Ok(Reply::AccountRemove(Err(detail)))) if !detail.is_empty() => {
                Err(format!("account_remove_refused: {detail}"))
            }
            _ => Err("account_remove_unconfirmed".into()),
        }
    }
}

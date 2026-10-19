//! Waiting is disposable; the idempotent account stop remains queued after a
//! lost observer. A matching broker acknowledgement confirms its stop contract,
//! not a portable proof that every descendant exited (Direct mode is leader-only).
#![allow(dead_code)] // Account-wide revocation admission remains gated.

use super::{Arc, CoreCommand, Duration, RemoteRuntime, Shared};
use crate::provider_account::ProviderAccountId;
use anyhow::Result;
use tokio::sync::oneshot;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Stopped,
    Refused(String),
    Unconfirmed,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadmissionOutcome {
    Admitted,
    Refused(String),
    Unconfirmed,
}

pub(crate) struct PendingReadmission(PendingStop);

impl PendingReadmission {
    pub(crate) async fn wait(self, timeout: Duration) -> ReadmissionOutcome {
        match self.0.wait(timeout).await {
            Outcome::Stopped => ReadmissionOutcome::Admitted,
            Outcome::Refused(reason) => ReadmissionOutcome::Refused(reason),
            Outcome::Unconfirmed => ReadmissionOutcome::Unconfirmed,
        }
    }
}

pub(super) struct Waiter {
    token: uuid::Uuid,
    sender: oneshot::Sender<Outcome>,
}

pub(crate) struct PendingStop {
    shared: Arc<Shared>,
    command_id: String,
    token: uuid::Uuid,
    receiver: oneshot::Receiver<Outcome>,
}

impl PendingStop {
    pub(crate) async fn wait(mut self, timeout: Duration) -> Outcome {
        match tokio::time::timeout(timeout, &mut self.receiver).await {
            Ok(Ok(outcome)) => outcome,
            _ => Outcome::Unconfirmed,
        }
    }
}

impl Drop for PendingStop {
    fn drop(&mut self) {
        let mut waiters = self.shared.account_stops.lock();
        if waiters
            .get(&self.command_id)
            .is_some_and(|waiter| waiter.token == self.token)
        {
            waiters.remove(&self.command_id);
        }
        // Do not withdraw an already queued stop or claim that it completed.
    }
}

impl RemoteRuntime {
    /// Caller must verify the committed migration and current session authority.
    /// An admitted result means no process was started by this operation.
    pub(crate) fn readmit_account_session(
        &self,
        mut session: crate::runtime_wire::StartSession,
        binding: crate::provider_account::PreparedBinding,
    ) -> Result<PendingReadmission> {
        anyhow::ensure!(
            self.connected() && command_supported_for_readmission(&session, &binding),
            "invalid_readmission_scope"
        );
        session.adopt_only = true;
        if session.generation.is_empty() {
            session
                .generation
                .clone_from(&self.shared.desired_generation);
        }
        let command_id = crate::provider_account::account_stop_command_id(
            &format!("readmission:{}", binding.operation),
            &session.session_id,
            &session.provider_account_id,
        );
        let mut waiters = self.shared.account_stops.lock();
        anyhow::ensure!(
            !waiters.contains_key(&command_id),
            "readmission_observer_exists"
        );
        let (sender, receiver) = oneshot::channel();
        let token = uuid::Uuid::new_v4();
        waiters.insert(command_id.clone(), Waiter { token, sender });
        self.queue(
            command_id.clone(),
            CoreCommand::ReadmitAccountSession {
                session: Box::new(session),
                binding: Box::new(binding),
                command_id: command_id.clone(),
            },
        );
        Ok(PendingReadmission(PendingStop {
            shared: self.shared.clone(),
            command_id,
            token,
            receiver,
        }))
    }
    pub(crate) fn account_stop_connection(&self) -> Option<u64> {
        self.connected().then(|| {
            self.shared
                .account_stop_epoch
                .load(std::sync::atomic::Ordering::Acquire)
        })
    }
    #[cfg(test)]
    pub(crate) fn for_account_stop_test(key: String) -> Arc<Self> {
        let mut runtime = Self::for_test(super::Hub::new(), Vec::new());
        let runtime_mut = Arc::get_mut(&mut runtime).unwrap();
        Arc::get_mut(&mut runtime_mut.shared)
            .unwrap()
            .auth_recipient_key = Some(key);
        runtime.connect_for_test();
        runtime
    }

    pub(crate) fn account_stop_recipient(&self) -> Option<String> {
        self.shared
            .auth_recipient_key
            .as_ref()
            .map(|key| crate::admin::hex_sha256(key.as_bytes()))
    }

    /// The caller must authorize the durable revocation operation and exact
    /// enrolled Machine before invoking this transport primitive. Its result
    /// is process-stop evidence only, never native logout or credential cleanup.
    pub(crate) fn quiesce_account_session(
        &self,
        operation: &str,
        session_id: &str,
        account_id: &ProviderAccountId,
    ) -> Result<PendingStop> {
        anyhow::ensure!(!account_id.is_default(), "named_account_required");
        anyhow::ensure!(
            !operation.is_empty()
                && operation.len() <= 512
                && !session_id.is_empty()
                && session_id.len() <= 512,
            "invalid_stop_scope"
        );
        let command_id =
            crate::provider_account::account_stop_command_id(operation, session_id, account_id);
        let mut waiters = self.shared.account_stops.lock();
        anyhow::ensure!(!waiters.contains_key(&command_id), "stop_observer_exists");
        let (sender, receiver) = oneshot::channel();
        let token = uuid::Uuid::new_v4();
        waiters.insert(command_id.clone(), Waiter { token, sender });
        self.queue(
            command_id.clone(),
            CoreCommand::QuiesceAccountSession {
                session_id: session_id.into(),
                command_id: command_id.clone(),
                account_id: account_id.clone(),
            },
        );
        Ok(PendingStop {
            shared: self.shared.clone(),
            command_id,
            token,
            receiver,
        })
    }
}

fn command_supported_for_readmission(
    session: &crate::runtime_wire::StartSession,
    binding: &crate::provider_account::PreparedBinding,
) -> bool {
    !session.provider_account_id.is_default()
        && crate::managed_calls::valid_id(&binding.operation)
        && session.session_id == binding.stopped.request.session_id
        && session.provider_account_id == binding.stopped.request.account_id
        && session.provider_account_id == binding.target.account_id
        && session.provider == binding.target.provider_id
        && session.provider_version == binding.target.provider_version
        && session.provider_generation_digest == binding.target.generation_digest
        && session.provider_auth_generation == Some(binding.target.generation)
        && session.agent_session_id.is_none()
}

pub(super) fn acknowledge(
    shared: &Shared,
    command_id: &str,
    accepted: bool,
    reason: Option<String>,
) {
    if let Some(waiter) = shared.account_stops.lock().remove(command_id) {
        let outcome = if accepted {
            Outcome::Stopped
        } else {
            Outcome::Refused(reason.unwrap_or_else(|| "account stop refused".into()))
        };
        let _ = waiter.sender.send(outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Hub;
    use crate::runtime_wire::Frame;

    async fn ack(runtime: &RemoteRuntime, command: &str, session: &str, accepted: bool) {
        super::super::handle_frame(
            &runtime.shared,
            Frame::CommandAck {
                session_id: session.into(),
                command_id: command.into(),
                accepted,
                reason: (!accepted).then(|| "still running".into()),
            },
            &mut Vec::new(),
        )
        .await
        .unwrap();
    }

    fn account() -> ProviderAccountId {
        uuid::Uuid::new_v4().to_string().try_into().unwrap()
    }

    #[tokio::test]
    async fn readmission_requires_scoped_ack_and_does_not_queue_ensure() {
        let runtime = RemoteRuntime::for_account_stop_test("fixture".into());
        let id = account();
        let session: crate::runtime_wire::StartSession = serde_json::from_value(serde_json::json!({
            "session_id":"s", "provider":"claude-code", "provider_account_id":id,
            "provider_version":"1", "provider_generation_digest":"digest", "provider_auth_generation":2,
            "cwd":"/fixture", "generation":"worker"
        })).unwrap();
        let binding: crate::provider_account::PreparedBinding = serde_json::from_value(serde_json::json!({
            "operation":"move", "stopped":{"request":{"session_id":"s","account_id":id,"stop_command_id":"stop"},"launch_id":uuid::Uuid::new_v4()},
            "target":{"account_id":id,"provider_id":"claude-code","request_id":"login","generation":2,
                "method":"native","provider_version":"1","generation_digest":"digest","auth_contract_fingerprint":"fixture",
                "identity":{"authority":"fixture","subject":"private","tenant":null}}
        })).unwrap();
        let pending = runtime
            .readmit_account_session(session.clone(), binding.clone())
            .unwrap();
        let command = pending.0.command_id.clone();
        assert!(
            runtime
                .readmit_account_session(session.clone(), binding.clone())
                .is_err()
        );
        ack(&runtime, &command, "wrong", true).await;
        assert!(runtime.shared.pending.lock().contains_key(&command));
        assert!(
            matches!(&runtime.shared.pending.lock()[&command], CoreCommand::ReadmitAccountSession { session, .. } if session.adopt_only)
        );
        ack(&runtime, &command, "s", true).await;
        assert_eq!(
            pending.wait(Duration::ZERO).await,
            ReadmissionOutcome::Admitted
        );
        assert!(runtime.shared.pending.lock().is_empty());
        let pending = runtime.readmit_account_session(session, binding).unwrap();
        assert_eq!(
            pending.wait(Duration::ZERO).await,
            ReadmissionOutcome::Unconfirmed
        );
        assert_eq!(runtime.shared.pending.lock().len(), 1);
    }

    #[tokio::test]
    async fn stop_requires_exact_ack_and_retains_pending_after_wrong_session() {
        let runtime = RemoteRuntime::for_test(Hub::new(), Vec::new());
        let account = account();
        assert!(
            runtime
                .quiesce_account_session("op", "s", &ProviderAccountId::default())
                .is_err()
        );
        let pending = runtime
            .quiesce_account_session("op", "s", &account)
            .unwrap();
        let id = pending.command_id.clone();
        assert!(
            runtime
                .quiesce_account_session("op", "s", &account)
                .is_err()
        );
        ack(&runtime, &id, "other-session", true).await;
        assert!(runtime.shared.pending.lock().contains_key(&id));
        assert!(runtime.shared.account_stops.lock().contains_key(&id));
        ack(&runtime, &id, "s", true).await;
        assert_eq!(pending.wait(Duration::ZERO).await, Outcome::Stopped);
        assert!(!runtime.shared.pending.lock().contains_key(&id));
        assert!(runtime.shared.account_stops.lock().is_empty());
    }

    #[tokio::test]
    async fn observer_timeout_and_drop_retain_one_idempotent_stop() {
        let runtime = RemoteRuntime::for_test(Hub::new(), Vec::new());
        let account = account();
        let pending = runtime
            .quiesce_account_session("op", "s", &account)
            .unwrap();
        let id = pending.command_id.clone();
        assert_eq!(pending.wait(Duration::ZERO).await, Outcome::Unconfirmed);
        assert!(runtime.shared.account_stops.lock().is_empty());
        let retry = runtime
            .quiesce_account_session("op", "s", &account)
            .unwrap();
        assert_eq!(retry.command_id, id);
        assert_eq!(runtime.shared.pending.lock().len(), 1);
        drop(retry);
        assert!(runtime.shared.account_stops.lock().is_empty());
        assert!(runtime.shared.pending.lock().contains_key(&id));
        let retry = runtime
            .quiesce_account_session("op", "s", &account)
            .unwrap();
        ack(&runtime, &id, "s", false).await;
        assert_eq!(
            retry.wait(Duration::ZERO).await,
            Outcome::Refused("still running".into())
        );
    }

    #[tokio::test]
    async fn completed_observer_cannot_remove_a_new_observer_for_the_same_stop() {
        let runtime = RemoteRuntime::for_test(Hub::new(), Vec::new());
        let account = account();
        let first = runtime
            .quiesce_account_session("op", "s", &account)
            .unwrap();
        let id = first.command_id.clone();
        ack(&runtime, &id, "s", true).await;
        let retry = runtime
            .quiesce_account_session("op", "s", &account)
            .unwrap();
        drop(first);
        assert!(runtime.shared.account_stops.lock().contains_key(&id));
        ack(&runtime, &id, "s", true).await;
        assert_eq!(retry.wait(Duration::ZERO).await, Outcome::Stopped);
    }

    #[tokio::test]
    async fn old_broker_refusal_is_not_stop_evidence() {
        let runtime = RemoteRuntime::for_test(Hub::new(), Vec::new());
        runtime
            .shared
            .protocol
            .store(2, std::sync::atomic::Ordering::Release);
        let pending = runtime
            .quiesce_account_session("op", "s", &account())
            .unwrap();
        let mut bytes = Vec::new();
        super::super::send_pending(&runtime.shared, &mut bytes)
            .await
            .unwrap();
        assert!(bytes.is_empty());
        assert!(matches!(
            pending.wait(Duration::ZERO).await,
            Outcome::Refused(_)
        ));
    }
}

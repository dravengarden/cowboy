//! Idle-rollover hibernation admission, dormancy authority, and keepalive handoff proofs.

use std::sync::Arc;

use super::Supervisor;
use crate::core::{KeepaliveInactiveProof, Status};
use crate::machine_control::ConnectionToken;
use crate::remote_runtime::RemoteRuntime;

/// Live keepalive-inactive proof pinned to the exact authenticated
/// [`RemoteRuntime`] and [`ConnectionToken`] registration that owns the
/// session's worker.
#[derive(Clone)]
pub struct KeepaliveRuntimeProof {
    pub proof: KeepaliveInactiveProof,
    runtime: Arc<RemoteRuntime>,
    connection: ConnectionToken,
}

impl KeepaliveRuntimeProof {
    #[must_use]
    pub fn machine_id(&self) -> &str {
        &self.proof.machine_id
    }

    #[must_use]
    pub fn configuration(&self) -> &cowboy_provider_sdk::ConfigurationBehavior {
        &self.proof.configuration
    }

    #[must_use]
    pub fn connection(&self) -> &ConnectionToken {
        &self.connection
    }
}

impl Supervisor {
    /// Shared dormancy predicate used by both [`Self::forget_dormant_context`]
    /// and cache-metrics/rollover status reads: fresh `Status::Exited`, no
    /// live worker on the owning runtime, and no pending hibernation command.
    pub(super) fn is_dormant_locked(
        &self,
        session_id: &str,
        expected_agent_session_id: Option<&str>,
    ) -> bool {
        self.hub.session_info(session_id).is_some_and(|info| {
            info.meta.status == Status::Exited
                && expected_agent_session_id
                    .is_none_or(|expected| info.meta.agent_session_id.as_deref() == Some(expected))
        }) && self.runtime_for_session(session_id).is_ok_and(|runtime| {
            !runtime.has_worker(session_id) && !runtime.hibernation_pending(session_id)
        })
    }

    /// True iff `session_id` is genuinely dormant and eligible for renewal on
    /// its next prompt (no worker present and no hibernation acknowledgement
    /// still in flight).
    #[must_use]
    pub fn is_dormant_for_renewal(&self, session_id: &str) -> bool {
        let _lifecycle = self.lifecycle.lock();
        self.is_dormant_locked(session_id, None)
    }

    /// Capture `(machine_id, configuration, last_activity, worker_epoch)` and
    /// the current [`RemoteRuntime`] + [`ConnectionToken`] atomically under
    /// the lifecycle lock before querying the owning gateway's live keepalive
    /// status. Returns `Ok(None)` when explicit paid keepalive is not
    /// configured on `session_id`, and refuses if the runtime connection lacks
    /// epoch-bound hibernation support or an attached connection token.
    pub fn capture_keepalive_probe(
        &self,
        session_id: &str,
    ) -> Result<Option<KeepaliveRuntimeProof>, String> {
        let _lifecycle = self.lifecycle.lock();
        let Some((machine_id, configuration, last_activity)) =
            self.hub.keepalive_probe_target(session_id)
        else {
            return Ok(None);
        };
        let runtime = self.runtime_for_session(session_id)?;
        if !runtime.supports_hibernation_epoch() {
            return Err("Machine connection predates epoch-bound hibernation".to_owned());
        }
        let Some(connection) = runtime.connection_token() else {
            return Err("Machine runtime has no attached connection token".to_owned());
        };
        if runtime.hibernation_pending(session_id) {
            return Err("session is already hibernating".to_owned());
        }
        let Some(snapshot) = runtime.worker_snapshot(session_id) else {
            return Err("session has no live worker snapshot".to_owned());
        };
        if snapshot.worker_epoch.is_empty() {
            return Err("worker snapshot has no epoch".to_owned());
        }
        Ok(Some(KeepaliveRuntimeProof {
            proof: KeepaliveInactiveProof {
                machine_id,
                configuration,
                last_activity,
                worker_epoch: snapshot.worker_epoch,
            },
            runtime,
            connection,
        }))
    }

    /// Revalidate idle-rollover hibernation admission atomically under the
    /// lifecycle lock: same authenticated runtime connection (`Arc::ptr_eq`),
    /// active connection token (`connection_is_current`), epoch-bound
    /// hibernation support, single-snapshot worker epoch, renewal policy
    /// epoch, activity, queue state, background work, context size, and
    /// explicit `DeepSeek` keepalive proof.
    pub fn hibernate_idle_session<F>(
        &self,
        session_id: &str,
        min_idle: std::time::Duration,
        min_context_tokens: u64,
        expected_policy_epoch: u64,
        keepalive_proof: Option<&KeepaliveRuntimeProof>,
        connection_is_current: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&ConnectionToken) -> bool,
    {
        let _lifecycle = self.lifecycle.lock();
        let runtime = self.runtime_for_session(session_id)?;
        if runtime.hibernation_pending(session_id) {
            return Err("session is already hibernating".to_owned());
        }
        let Some(snapshot) = runtime.worker_snapshot(session_id) else {
            return Err("session is already hibernated".to_owned());
        };
        if snapshot.worker_epoch.is_empty() {
            return Err("worker snapshot has no epoch".to_owned());
        }
        if let Some(bound) = keepalive_proof {
            if !runtime.supports_hibernation_epoch()
                || !Arc::ptr_eq(&runtime, &bound.runtime)
                || !connection_is_current(&bound.connection)
            {
                return Err("Machine runtime connection changed after keepalive check".to_owned());
            }
            if snapshot.worker_epoch != bound.proof.worker_epoch {
                return Err("worker epoch changed after keepalive check".to_owned());
            }
        }
        self.hub.admit_idle_hibernation(
            session_id,
            min_idle,
            min_context_tokens,
            expected_policy_epoch,
            keepalive_proof.map(|bound| &bound.proof),
        )?;
        runtime.hibernate_epoch(session_id, Some(snapshot.worker_epoch));
        Ok(())
    }
}

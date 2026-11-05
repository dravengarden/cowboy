//! Provider generation pins on sessions: auth recovery, dormant and workerless repins, missing generations, native bindings, reloads and release reconciliation.

use super::*;

impl Hub {
    /// Return the native id only when Codex has had a user turn in the current
    /// context generation. An id allocated by `session/new` alone has no rollout
    /// and must not be handed to `session/load` after a restart.
    #[must_use]
    /// See `provider_version_portable`.
    pub fn provider_version_changeable(&self, session_id: &str) -> bool {
        let sessions = self.inner.sessions.lock();
        sessions
            .get(session_id)
            .is_some_and(provider_version_portable)
    }

    pub fn agent_session_id_for_resume(&self, session_id: &str) -> Option<String> {
        let forked = self.native_fork_settled(session_id);
        let sessions = self.inner.sessions.lock();
        let session = sessions.get(session_id)?;
        (forked || current_context_has_user_message(session))
            .then(|| session.meta.agent_session_id.clone())
            .flatten()
    }

    /// Reserve an idle session with an unresolved authentication failure for
    /// recovery after a newer compatible Service-auth generation reaches its
    /// Machine. The session deliberately retains its original auth generation:
    /// that generation owns the Provider runtime home containing the native
    /// session database and rollout. Machine credential reconciliation updates
    /// only the declared credential files in that home.
    pub fn begin_provider_auth_recovery(
        &self,
        session_id: &str,
        expected_status_revision: (Status, u64),
        expected_crash_detail: &str,
        expected_generation: u64,
        next_generation: u64,
    ) -> Result<bool, String> {
        if next_generation <= expected_generation {
            return Err("Provider auth generation must advance".to_owned());
        }
        let reserved = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("unknown session {session_id:?}"))?;
            if (session.meta.status, session.lifecycle_epoch) != expected_status_revision
                || unresolved_crash_detail_for_session(session) != Some(expected_crash_detail)
                || session.meta.provider_auth_generation != Some(expected_generation)
                || session.in_flight
                || !matches!(session.meta.status, Status::Crashed | Status::Running)
            {
                false
            } else {
                session.meta.status = Status::Starting;
                session.lifecycle_epoch = session.lifecycle_epoch.wrapping_add(1);
                true
            }
        };
        if !reserved {
            return Ok(false);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateStatus {
                session_id: session_id.to_owned(),
                status: Status::Starting,
            });
        }
        self.push(
            session_id,
            Event::Lifecycle {
                status: Status::Starting,
                detail: Some("reloading synchronized Provider credentials".to_owned()),
            },
        );
        self.broadcast_sessions();
        Ok(true)
    }

    /// Move a dormant (exited, workerless) session's binding to another
    /// installed Provider generation without starting anything. The next open
    /// launches that generation with the same native resume as an explicit
    /// reload. Refuses unless the session is still exactly `expected` and
    /// still exited, so a racing open or prompt always wins.
    pub fn repin_dormant_provider(
        &self,
        expected: &SessionMeta,
        version: &str,
        digest: &str,
        behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
        carry: Option<&NamedReleaseCarry>,
    ) -> Result<(), String> {
        self.repin_workerless_provider(expected, version, digest, behavior, false, carry)
    }

    /// A Machine rejected this session's launch because its pinned Provider
    /// generation is not installed there any more. Recorded for the Provider
    /// update pass, which moves the session to the installed release through
    /// [`Self::repin_missing_provider`]; an open or prompt cannot help it.
    pub fn note_provider_generation_missing(&self, session_id: &str) {
        let mut sessions = self.inner.sessions.lock();
        if let Some(session) = sessions.get_mut(session_id) {
            session.provider_generation_missing = true;
        }
    }

    /// Sessions whose last launch was rejected for a missing Provider generation.
    #[must_use]
    pub fn provider_generation_missing_sessions(&self) -> Vec<String> {
        self.inner
            .sessions
            .lock()
            .iter()
            .filter(|(_, session)| session.provider_generation_missing)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Forget a missing-generation note nothing can act on, so the pass stops
    /// resolving it; the next rejected launch records it again.
    pub fn clear_provider_generation_missing(&self, session_id: &str) {
        let mut sessions = self.inner.sessions.lock();
        if let Some(session) = sessions.get_mut(session_id) {
            session.provider_generation_missing = false;
        }
    }

    /// Move a session noted by [`Self::note_provider_generation_missing`] to
    /// an installed Provider generation. Like a dormant re-pin, except that a
    /// crashed session qualifies: its launch failed and nothing of it runs.
    /// Clears the note and records the move in the transcript, leaving the
    /// session exited so the next open launches the new generation.
    pub fn repin_missing_provider(
        &self,
        expected: &SessionMeta,
        version: &str,
        digest: &str,
        behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
    ) -> Result<(), String> {
        self.repin_workerless_provider(expected, version, digest, behavior, true, None)?;
        self.set_status(
            &expected.id,
            Status::Exited,
            Some(format!(
                "Provider {} is no longer installed on {}; the session now opens on {version}",
                expected.provider_version, expected.machine_id
            )),
        );
        Ok(())
    }

    /// Project a binding already committed by the guarded Store writer. This
    /// deliberately enqueues no persistence write and never starts a worker.
    pub fn project_native_binding(
        &self,
        expected: &SessionMeta,
        target: &crate::provider_account::NativeProfileReceipt,
        behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
    ) -> Result<(), String> {
        {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(&expected.id)
                .ok_or_else(|| "session no longer exists".to_owned())?;
            let meta = &mut session.meta;
            if expected.provider_account_id.is_default()
                || target.account_id != expected.provider_account_id
                || target.provider_id != expected.provider
                || expected
                    .provider_auth_generation
                    .is_none_or(|old| old >= target.generation)
                || meta.closing
                || session.in_flight
                || meta.status != Status::Exited
                || meta.owner_user_id != expected.owner_user_id
                || meta.provider_account_id != expected.provider_account_id
                || meta.provider != expected.provider
                || meta.machine_id != expected.machine_id
            {
                return Err("native binding projection scope changed".into());
            }
            if meta.provider_auth_generation == Some(target.generation)
                && meta.provider_version == target.provider_version
                && meta.provider_generation_digest == target.generation_digest
                && meta.provider_behavior.as_ref() == Some(behavior)
                && meta.agent_session_id.is_none()
            {
                return Ok(());
            }
            if meta.provider_auth_generation != expected.provider_auth_generation
                || meta.provider_version != expected.provider_version
                || meta.provider_generation_digest != expected.provider_generation_digest
            {
                return Err("native binding projection conflicts with current binding".into());
            }
            meta.provider_version.clone_from(&target.provider_version);
            meta.provider_generation_digest
                .clone_from(&target.generation_digest);
            meta.provider_auth_generation = Some(target.generation);
            meta.provider_behavior = Some(behavior.clone());
            meta.agent_session_id = None;
            session.initial_config_options = None;
            session.provider_generation_missing = false;
            session.lifecycle_epoch = session.lifecycle_epoch.wrapping_add(1);
        }
        self.broadcast_sessions();
        Ok(())
    }

    pub(super) fn repin_workerless_provider(
        &self,
        expected: &SessionMeta,
        version: &str,
        digest: &str,
        behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
        after_rejected_launch: bool,
        carry: Option<&NamedReleaseCarry>,
    ) -> Result<(), String> {
        let meta = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(&expected.id)
                .ok_or_else(|| "session no longer exists".to_owned())?;
            let workerless = if after_rejected_launch {
                session.provider_generation_missing
                    && matches!(session.meta.status, Status::Exited | Status::Crashed)
            } else {
                session.meta.status == Status::Exited
            };
            if session.in_flight || !workerless {
                return Err("session is no longer dormant".to_owned());
            }
            if session.meta.provider != expected.provider
                || session.meta.provider_account_id != expected.provider_account_id
                || session.meta.provider_version != expected.provider_version
                || session.meta.provider_generation_digest != expected.provider_generation_digest
                || session.meta.provider_auth_generation != expected.provider_auth_generation
                || session.meta.agent_session_id != expected.agent_session_id
                || session.meta.machine_id != expected.machine_id
                || session.meta.cwd != expected.cwd
                || session.meta.execution_binding != expected.execution_binding
            {
                return Err("session changed while preparing to re-pin; try again".to_owned());
            }
            if !provider_version_portable(session) {
                return Err(
                    "a saved native session is required to change Provider version".to_owned(),
                );
            }
            session
                .meta
                .require_provider_release_transition(version, digest, carry)?;
            if session.meta.execution_binding.is_some() {
                let mut candidate = session.meta.clone();
                candidate.provider_version = version.to_owned();
                candidate.provider_generation_digest = digest.to_owned();
                candidate.provider_behavior = Some(behavior.clone());
                candidate.require_runtime_launch()?;
            }
            session.meta.provider_version = version.to_owned();
            session.meta.provider_generation_digest = digest.to_owned();
            session.meta.provider_behavior = Some(behavior.clone());
            session.provider_generation_missing = false;
            session.initial_config_options = None;
            session.lifecycle_epoch = session.lifecycle_epoch.wrapping_add(1);
            session.meta.clone()
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ReloadProvider(Box::new(meta)));
        }
        self.broadcast_sessions();
        Ok(())
    }

    /// Reserve an idle session for an explicit Provider reload. The lock also
    /// fences prompt submission: a racing prompt either wins and rejects the
    /// reload, or stays queued until the replacement runtime is ready.
    pub fn begin_provider_reload(
        &self,
        expected: &SessionMeta,
        version: &str,
        digest: &str,
        behavior: &cowboy_provider_sdk::ProviderBehaviorContract,
        automatic: bool,
        carry: Option<&NamedReleaseCarry>,
    ) -> Result<(), String> {
        let meta = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(&expected.id)
                .ok_or_else(|| "session no longer exists".to_owned())?;
            // Every live Provider reload (Web Reload, local Operator, automatic
            // pass) starts a replacement worker here. A managed child serves one
            // call on its parent's binding and has no execution recovery, so a
            // replacement only crashes once its execution lifetime is gone.
            if session
                .meta
                .execution_binding
                .as_ref()
                .is_some_and(|binding| binding.managed().is_some())
            {
                return Err(
                    "managed child sessions keep their Provider release until they are retired"
                        .to_owned(),
                );
            }
            if session.in_flight || matches!(session.meta.status, Status::Busy | Status::Starting) {
                return Err(
                    "wait for the current turn to finish before loading a new Provider version"
                        .to_owned(),
                );
            }
            if session.meta.provider != expected.provider
                || session.meta.provider_account_id != expected.provider_account_id
                || session.meta.status != expected.status
                || session.meta.provider_version != expected.provider_version
                || session.meta.provider_generation_digest != expected.provider_generation_digest
                || session.meta.provider_auth_generation != expected.provider_auth_generation
                || session.meta.agent_session_id != expected.agent_session_id
                || session.meta.machine_id != expected.machine_id
                || session.meta.cwd != expected.cwd
                || session.meta.execution_binding != expected.execution_binding
            {
                return Err("session changed while preparing reload; try again".to_owned());
            }
            if !provider_version_portable(session) {
                return Err(
                    "a saved native session is required to reload a new Provider version"
                        .to_owned(),
                );
            }
            // Background work lives in the worker an unattended update would
            // replace; an explicit Reload is the user's own decision.
            if automatic && session.meta.background_tasks > 0 {
                return Err("session has background work in progress".to_owned());
            }
            session
                .meta
                .require_provider_release_transition(version, digest, carry)?;
            if session.meta.execution_binding.is_some() {
                let mut candidate = session.meta.clone();
                candidate.provider_version = version.to_owned();
                candidate.provider_generation_digest = digest.to_owned();
                candidate.provider_behavior = Some(behavior.clone());
                candidate.require_runtime_launch()?;
            }
            session.meta.provider_update = Some(ProviderUpdate {
                from: session.meta.provider_version.clone(),
                to: version.to_owned(),
                automatic,
                started_at_ms: now_ms(),
            });
            session.meta.provider_version = version.to_owned();
            session.meta.provider_generation_digest = digest.to_owned();
            session.meta.provider_behavior = Some(behavior.clone());
            session.initial_config_options = None;
            // Auth generation owns the native runtime home. Never replace it
            // with the active installation's credential generation here.
            session.meta.status = Status::Starting;
            session.lifecycle_epoch = session.lifecycle_epoch.wrapping_add(1);
            (
                session.meta.clone(),
                current_context_has_user_message(session),
            )
        };
        let (meta, resumes) = meta;
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ReloadProvider(Box::new(meta)));
        }
        self.push(
            &expected.id,
            Event::Lifecycle {
                status: Status::Starting,
                detail: Some(format!(
                    "reloading Provider {} -> {version}; {}",
                    expected.provider_version,
                    if resumes {
                        "preserving native session"
                    } else {
                        "the next turn starts a fresh native session"
                    }
                )),
            },
        );
        self.broadcast_sessions();
        Ok(())
    }

    /// Settle an interrupted Provider reload against the surviving native
    /// worker. The reset and write-behind persistence travel independently;
    /// after reconnect the actual owner, not an unacknowledged intent, defines
    /// the running release. Never use this to migrate native identity or home.
    /// Runs before the snapshot acceptance fence, so it applies its own
    /// placement check: a different execution binding is never adopted.
    pub fn reconcile_provider_release(&self, worker: &WorkerSnapshot) {
        // Only a worker that still runs defines the release. An exited worker's
        // stale launch would undo a dormant re-pin, which the next open then
        // re-pins again, every time the Machine restates its snapshots.
        let Some(launch) = worker.launch.as_ref().filter(|launch| {
            worker.has_connected_owner()
                && !matches!(worker.state, WorkerState::Exited | WorkerState::Crashed)
                && !launch.provider_generation_digest.is_empty()
        }) else {
            return;
        };
        let reconciled = {
            let mut sessions = self.inner.sessions.lock();
            let Some(session) = sessions.get_mut(&worker.session_id) else {
                return;
            };
            if session.meta.provider != launch.provider
                || session.meta.provider_account_id != launch.provider_account_id
                || session
                    .meta
                    .require_provider_release_transition(
                        &launch.provider_version,
                        &launch.provider_generation_digest,
                        None,
                    )
                    .is_err()
                || session.meta.require_runtime_launch().is_err()
                || session.meta.cwd != launch.cwd
                || session.meta.execution_binding != launch.execution_binding
                || session.meta.provider_auth_generation != launch.provider_auth_generation
                || session.meta.agent_session_id.is_none()
                || session.meta.agent_session_id != worker.agent_session_id
                || session.meta.provider_generation_digest.is_empty()
                || (session.meta.provider_generation_digest == launch.provider_generation_digest
                    && session.meta.provider_version == launch.provider_version)
            {
                return;
            }
            session
                .meta
                .provider_version
                .clone_from(&launch.provider_version);
            session
                .meta
                .provider_generation_digest
                .clone_from(&launch.provider_generation_digest);
            session
                .meta
                .provider_behavior
                .clone_from(&launch.provider_behavior);
            session.meta.clone()
        };
        tracing::warn!(session = %worker.session_id, version = %reconciled.provider_version,
            digest = %reconciled.provider_generation_digest,
            "reconciled interrupted Provider reload with surviving native worker");
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::ReloadProvider(Box::new(reconciled)));
        }
        self.broadcast_sessions();
    }
}

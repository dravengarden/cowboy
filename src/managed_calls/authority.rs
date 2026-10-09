//! Controller-derived parent authority. Placement alone is never a launch
//! grant: a surviving native conversation may have several worker incarnations.
//! This value is not deserializable from a CLI or Machine request.

use anyhow::{Result, ensure};

use crate::{
    core::SessionMeta,
    runtime_wire::{WorkerSnapshot, WorkerState},
};

use super::{lifecycle::Placement, service::resolve_placement};

#[derive(Clone, PartialEq, Eq)]
pub struct Authority {
    placement: Placement,
    runtime_machine: String,
    worker_epoch: String,
}

impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedCallAuthority([private])")
    }
}

impl Authority {
    /// Both arguments must be current Controller observations. The runtime
    /// additionally fences its connection and any session reset before calling.
    pub(crate) fn for_worker(
        service: &str,
        parent: &SessionMeta,
        worker: &WorkerSnapshot,
    ) -> Result<Self> {
        let placement = resolve_placement(service, parent)?;
        ensure!(
            worker.session_id == parent.id
                && worker.has_connected_owner()
                && super::valid_id(&worker.worker_epoch)
                // A drain request only schedules replacement at the next safe
                // boundary; the new worker epoch revokes this authority then.
                && matches!(worker.state, WorkerState::Running | WorkerState::Busy),
            "parent has no active worker owner"
        );
        let launch = worker
            .launch
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("parent worker launch is unavailable"))?;
        ensure!(
            launch.session_id == parent.id
                && launch.provider == parent.provider
                && launch.provider_version == parent.provider_version
                && launch.provider_generation_digest == parent.provider_generation_digest
                && launch.provider_behavior == parent.provider_behavior
                && launch.cwd == parent.cwd
                && launch.execution_binding == parent.execution_binding
                && launch.generation == worker.generation,
            "parent worker launch changed"
        );
        Ok(Self {
            placement,
            runtime_machine: parent.machine_id.clone(),
            worker_epoch: worker.worker_epoch.clone(),
        })
    }

    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    /// Compare a previously issued authority against a freshly derived one on
    /// every action, including inspect/wait/result. Never refresh the old grant
    /// in place when a worker is replaced or a parent changes owner.
    pub fn accepts(&self, current: &Self) -> bool {
        self == current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (SessionMeta, WorkerSnapshot) {
        let mut parent: SessionMeta = serde_json::from_value(json!({
            "id":"parent", "provider":"claude-code", "machine_id":"ovh",
            "workspace_id":"cowboy", "cwd":"/runtime/session", "title":"Review",
            "status":"running", "provider_version":"1.0.0",
            "provider_generation_digest":"generation"
        }))
        .unwrap();
        let binding = crate::execution_environment::fixture();
        let mut behavior = crate::provider::legacy_behavior("claude-code");
        behavior.execution = Some(cowboy_provider_sdk::ExecutionBehavior::JsonrpcV1 {
            executor_digests: [binding.decode().unwrap().environment.executor_digest].into(),
        });
        parent.provider_behavior = Some(behavior);
        parent.execution_binding = Some(binding);
        let worker = serde_json::from_value(json!({
            "session_id":parent.id, "worker_epoch":"worker-one", "generation":"worker-build",
            "state":"busy", "agent_session_id":null, "current_turn_id":"turn-one",
            "last_runtime_seq":1, "pending_permissions":[], "pending_prompt_count":0,
            "drain_requested":false,
            "launch":{
                "session_id":parent.id, "provider":parent.provider,
                "provider_version":parent.provider_version,
                "provider_generation_digest":parent.provider_generation_digest,
                "provider_behavior":parent.provider_behavior, "cwd":parent.cwd,
                "system":false, "generation":"worker-build",
                "execution_binding":parent.execution_binding
            }
        }))
        .unwrap();
        (parent, worker)
    }

    #[test]
    fn remote_authority_follows_target_but_is_fenced_by_runtime_incarnation() {
        let (mut parent, mut worker) = fixture();
        let original = Authority::for_worker("service-test", &parent, &worker).unwrap();
        assert_eq!(original.placement.machine_id, "hawk");
        assert_eq!(original.placement.cwd, "/tasks/cowboy");
        assert_eq!(original.runtime_machine, "ovh");
        parent.agent_session_id = Some("native-created-later".into());
        parent.title = "Renamed".into();
        worker.current_turn_id = Some("next-turn".into());
        worker.state = WorkerState::Running;
        assert!(
            original.accepts(&Authority::for_worker("service-test", &parent, &worker).unwrap())
        );
        worker.worker_epoch = "worker-two".into();
        assert!(
            !original.accepts(&Authority::for_worker("service-test", &parent, &worker).unwrap())
        );
        worker.worker_epoch = "worker-one".into();
        parent.owner_user_id = Some("new-owner".into());
        assert!(
            !original.accepts(&Authority::for_worker("service-test", &parent, &worker).unwrap())
        );
        assert!(!format!("{original:?}").contains("ovh"));
    }

    #[test]
    fn placeholders_dead_workers_and_launch_drift_never_authorize_calls() {
        let (parent, worker) = fixture();
        for state in [
            WorkerState::Starting,
            WorkerState::Draining,
            WorkerState::Exited,
            WorkerState::Crashed,
        ] {
            let mut changed = worker.clone();
            changed.state = state;
            assert!(Authority::for_worker("service-test", &parent, &changed).is_err());
        }
        for epoch in ["", "broker-known-session", "../worker"] {
            let mut changed = worker.clone();
            changed.worker_epoch = epoch.into();
            assert!(Authority::for_worker("service-test", &parent, &changed).is_err());
        }
        // A long busy turn after a Provider upgrade keeps its exact worker
        // until the drain boundary, and keeps the same authority until then.
        let mut changed = worker.clone();
        changed.drain_requested = true;
        assert!(
            Authority::for_worker("service-test", &parent, &worker)
                .unwrap()
                .accepts(&Authority::for_worker("service-test", &parent, &changed).unwrap())
        );
        changed = worker.clone();
        changed.launch.as_mut().unwrap().cwd = "/another-worktree".into();
        assert!(Authority::for_worker("service-test", &parent, &changed).is_err());
        changed = worker.clone();
        changed.launch.as_mut().unwrap().provider_generation_digest = "replacement".into();
        assert!(Authority::for_worker("service-test", &parent, &changed).is_err());
        changed = worker.clone();
        changed.launch.as_mut().unwrap().execution_binding = None;
        assert!(Authority::for_worker("service-test", &parent, &changed).is_err());
        let mut closed = parent;
        closed.closing = true;
        assert!(Authority::for_worker("service-test", &closed, &worker).is_err());
    }
}

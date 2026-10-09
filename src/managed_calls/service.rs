//! Controller-owned call ledger. Admission records intent; only a successful
//! claim owns dispatch, and neither a timeout nor reconnect grants a new claim.

use anyhow::{Result, ensure};

use super::{
    Conversation, Request,
    lifecycle::{Placement, Record, State},
};
use crate::{core::SessionMeta, store::Store};

pub struct Ledger {
    store: Store,
    placement: Placement,
    runtime_machine_id: String,
}

impl Ledger {
    /// The parent's own Agent runtime Machine.
    pub(crate) fn runtime_machine_id(&self) -> &str {
        &self.runtime_machine_id
    }

    /// Caller must first authorize the product principal or Machine grant. The
    /// session comes from Hub, never a CLI-supplied parent or filesystem path.
    pub(crate) fn for_parent(store: Store, service: &str, parent: &SessionMeta) -> Result<Self> {
        Ok(Self {
            store,
            placement: resolve_placement(service, parent)?,
            runtime_machine_id: parent.machine_id.clone(),
        })
    }

    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    pub async fn inspect(&self, call: &str) -> Result<Option<Record>> {
        self.store
            .managed_call(&self.placement.parent_session_id, call)
            .await
    }

    pub async fn observe(&self, request: &str) -> Result<Option<Record>> {
        self.store
            .managed_call_request(&self.placement.parent_session_id, request)
            .await
    }

    pub async fn list(&self, before: Option<&str>) -> Result<Vec<Record>> {
        self.store
            .managed_calls(&self.placement.parent_session_id, before)
            .await
    }

    /// Provider/profile readiness and immutable input capture are dispatcher
    /// responsibilities. No native effect occurs from this method.
    pub async fn admit(
        &self,
        provider: &str,
        request: &Request,
        call_id: &str,
        fresh_child_id: &str,
    ) -> Result<Record> {
        request.validate()?;
        if let Some(original) = self.observe(&request.request_id).await? {
            ensure!(
                original.request_digest == request.digest()
                    && original.provider == provider
                    && original.placement == self.placement,
                "managed call request conflict"
            );
            return Ok(original);
        }
        let child = match &request.conversation {
            Conversation::Fresh {} => fresh_child_id,
            Conversation::Continue { child_session_id } => {
                let previous = self
                    .store
                    .managed_child(&self.placement.parent_session_id, child_session_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("managed child not owned by parent"))?;
                ensure!(
                    previous.state.terminal()
                        && previous.provider == provider
                        && previous.placement.same_location(&self.placement)
                        && previous.access == request.access,
                    "managed child unavailable or incompatible"
                );
                child_session_id
            }
        };
        let now = chrono::Utc::now().timestamp_millis();
        let record = Record {
            schema: 1,
            call_id: call_id.into(),
            request_id: request.request_id.clone(),
            request_digest: request.digest(),
            provider: provider.into(),
            purpose: request.purpose,
            access: request.access,
            labels: request.labels.clone(),
            placement: self.placement.clone(),
            child_session_id: child.into(),
            state: State::Queued,
            revision: 1,
            created_at_ms: now,
            updated_at_ms: now,
            input_revision: None,
            result: None,
            runtime_machine_id: Some(self.runtime_machine_id.clone()),
            provider_version: None,
            provider_generation_digest: None,
            child_cursor: None,
            cancel_requested_at_ms: None,
            error: None,
        };
        self.store.admit_managed_call(&record, request).await
    }

    /// Only one claimant receives the original request. Losing a response after
    /// this CAS leaves Starting, which is observable but never claimable again.
    /// The exact child Provider generation is pinned in the same transition.
    pub async fn claim(
        &self,
        call: &str,
        provider_version: &str,
        provider_generation_digest: &str,
    ) -> Result<Option<(Record, Request)>> {
        let Some(previous) = self.inspect(call).await? else {
            return Ok(None);
        };
        if previous.state != State::Queued
            || !previous.placement.same_location(&self.placement)
            || previous.cancel_requested_at_ms.is_some()
        {
            return Ok(None);
        }
        let input = self
            .store
            .managed_call_input(&self.placement.parent_session_id, call)
            .await?
            .ok_or_else(|| anyhow::anyhow!("managed call input missing"))?;
        let mut next = previous.clone();
        next.state = State::Starting;
        next.revision += 1;
        next.updated_at_ms = chrono::Utc::now()
            .timestamp_millis()
            .max(previous.updated_at_ms);
        next.provider_version = Some(provider_version.into());
        next.provider_generation_digest = Some(provider_generation_digest.into());
        if self.store.advance_managed_call(&previous, &next).await? {
            Ok(Some((next, input)))
        } else {
            Ok(None)
        }
    }

    /// The adapter must prove each observation. This method checks durable
    /// ownership and CAS; it cannot manufacture native completion evidence.
    pub async fn advance(&self, previous: &Record, next: &Record) -> Result<bool> {
        ensure!(
            previous.placement == self.placement,
            "managed call scope changed"
        );
        self.store.advance_managed_call(previous, next).await
    }
}

pub(super) fn resolve_placement(service: &str, parent: &SessionMeta) -> Result<Placement> {
    ensure!(!parent.closing, "parent is closing");
    parent
        .require_runtime_launch()
        .map_err(anyhow::Error::msg)?;
    let (machine_id, workspace_id, cwd) = match &parent.execution_binding {
        Some(record) => {
            let binding = record
                .for_runtime(&parent.machine_id, &parent.cwd)
                .map_err(anyhow::Error::msg)?;
            (
                binding.environment.machine_id,
                binding.workspace.id,
                binding.workspace.cwd,
            )
        }
        None => (
            parent.machine_id.clone(),
            parent
                .workspace_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("parent has no registered workspace"))?,
            parent.cwd.clone(),
        ),
    };
    ensure!(
        machine_id != "local",
        "managed calls require an enrolled Machine"
    );
    // Status, title and token usage are deliberately absent: ordinary UI updates
    // cannot change authority. The native conversation id is also absent: it
    // materializes after session/new and is not the Cowboy parent's identity.
    // Owner, Provider generation and execution binding still fence authority.
    // The binding is an opaque JSONB record whose key order changes when the
    // parent is reloaded from PostgreSQL; hash it canonically.
    let identity = serde_json::json!({
        "service":service,"parent":parent.id,"owner":parent.owner_user_id,
        "runtime":parent.machine_id,"cwd":parent.cwd,"workspace":parent.workspace_id,
        "provider":parent.provider,"generation":parent.provider_generation_digest,
        "binding":parent.execution_binding,
    });
    let placement = Placement {
        service_id: service.into(),
        parent_session_id: parent.id.clone(),
        parent_revision: super::canonical_digest(&identity),
        machine_id,
        workspace_id,
        cwd,
    };
    ensure!(placement.validate(), "invalid parent placement");
    Ok(placement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parent() -> SessionMeta {
        serde_json::from_value(json!({"id":"parent-1","provider":"claude-code",
            "machine_id":"hawk","workspace_id":"cowboy","cwd":"/tasks/cowboy",
            "title":"Parent","status":"running"}))
        .unwrap()
    }

    #[test]
    fn native_materialization_preserves_scope_but_owner_and_generation_changes_revoke_it() {
        let mut parent = parent();
        let original = resolve_placement("service-test", &parent).unwrap();
        parent.agent_session_id = Some("native-created-after-startup".into());
        assert_eq!(
            resolve_placement("service-test", &parent).unwrap(),
            original
        );
        parent.provider_generation_digest = "another-installed-generation".into();
        assert_ne!(
            resolve_placement("service-test", &parent).unwrap(),
            original
        );
        parent.provider_generation_digest.clear();
        parent.owner_user_id = Some("another-owner".into());
        assert_ne!(
            resolve_placement("service-test", &parent).unwrap(),
            original
        );
        parent.closing = true;
        assert!(resolve_placement("service-test", &parent).is_err());
    }

    #[test]
    fn remote_parent_uses_target_and_unknown_binding_never_falls_back_to_runtime() {
        let mut parent = parent();
        let local = resolve_placement("service-test", &parent).unwrap();
        assert_eq!(local.machine_id, "hawk");
        parent.machine_id = "ovh".into();
        parent.cwd = "/runtime/session".into();
        parent.provider_version = "1.0.0".into();
        parent.provider_generation_digest = "generation".into();
        let binding = crate::execution_environment::fixture();
        let mut behavior = crate::provider::legacy_behavior("claude-code");
        behavior.execution = Some(cowboy_provider_sdk::ExecutionBehavior::JsonrpcV1 {
            executor_digests: [binding.decode().unwrap().environment.executor_digest].into(),
        });
        parent.provider_behavior = Some(behavior);
        parent.execution_binding = Some(binding);
        let remote = resolve_placement("service-test", &parent).unwrap();
        assert_eq!(remote.machine_id, "hawk");
        assert_eq!(remote.cwd, "/tasks/cowboy");
        parent.title = "Renamed".into();
        assert_eq!(resolve_placement("service-test", &parent).unwrap(), remote);
        // A PostgreSQL JSONB reload reorders binding keys; authority is stable.
        let record = parent.execution_binding.as_ref().unwrap().record().clone();
        let reversed: serde_json::Map<_, _> = record
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        assert_ne!(
            serde_json::to_string(&record).unwrap(),
            serde_json::to_string(&reversed).unwrap()
        );
        parent.execution_binding = Some(
            crate::execution_environment::ExecutionBinding::from_record(reversed.into()),
        );
        assert_eq!(resolve_placement("service-test", &parent).unwrap(), remote);
        parent.execution_binding = Some(
            crate::execution_environment::ExecutionBinding::from_record(json!({"schema":99})),
        );
        assert!(resolve_placement("service-test", &parent).is_err());
        parent.execution_binding = Some(
            crate::execution_environment::ExecutionBinding::from_record(serde_json::Value::Null),
        );
        assert!(resolve_placement("service-test", &parent).is_err());
    }

    #[tokio::test]
    async fn lost_launch_reply_never_grants_a_second_claim() {
        let root = std::env::temp_dir().join(format!(
            "cw-ledger-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let store = Store::connect("sqlite::memory:", root.clone())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let parent = parent();
        store.insert_session(&parent).await.unwrap();
        let ledger = Ledger::for_parent(store, "service-test", &parent).unwrap();
        let request = Request::parse(br#"{"schema":1,"request_id":"review-1","purpose":"review","instruction":"Review this round","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#).unwrap();
        let first = ledger
            .admit("codex", &request, "call-1", "child-1")
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            ledger.claim(&first.call_id, "1.0.0", "generation"),
            ledger.claim(&first.call_id, "1.0.0", "generation")
        );
        assert_ne!(a.unwrap().is_some(), b.unwrap().is_some());
        assert!(
            ledger
                .claim(&first.call_id, "1.0.0", "generation")
                .await
                .unwrap()
                .is_none()
        );
        let observed = ledger
            .admit("codex", &request, "replacement-call", "replacement-child")
            .await
            .unwrap();
        assert_eq!(observed.state, State::Starting);
        assert_eq!(observed.child_session_id, "child-1");
        let mut followup = request.clone();
        followup.request_id = "review-2".into();
        followup.conversation = Conversation::Continue {
            child_session_id: "child-1".into(),
        };
        assert!(
            ledger
                .admit("codex", &followup, "call-2", "unused")
                .await
                .is_err()
        );
        drop(ledger);
        let _ = std::fs::remove_dir_all(root);
    }
}

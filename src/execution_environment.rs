//! Durable execution identity, independent of Agent runtime placement.
//!
//! A stored binding is an identity record, never a transport or execution grant.
//! Retain unrecognized records through restore/save, but refuse to resolve them.
//! In particular, an unsupported record must not become an absent/local binding.

use serde::{Deserialize, Serialize};

pub const EXECUTION_BINDING_SCHEMA: u16 = 1;
pub const EXECUTION_PROTOCOL: u16 = 1;
const MAX_BINDING_BYTES: usize = 16 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecutionBinding(serde_json::Value);

impl std::fmt::Debug for ExecutionBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Unknown future records must not leak their arbitrary fields to logs.
        formatter
            .debug_struct("ExecutionBinding")
            .field("recognized", &self.decode().is_ok())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingV1 {
    pub schema: u16,
    pub id: String,
    pub revision: u64,
    pub runtime: RuntimeLocation,
    pub environment: EnvironmentLocation,
    pub workspace: WorkspaceLocation,
    pub access: ExecutionAccess,
    /// Present only for a managed child whose Agent runtime is on another
    /// Machine. Older readers refuse the field and therefore the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<ManagedBindingV1>,
}

/// The parent-owned constraint of a remotely executed managed child. Its
/// execution Machine enforces `profile`; its runtime applies the turn round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedBindingV1 {
    pub parent_session_id: String,
    pub profile: cowboy_provider_sdk::ManagedRuntimeProfile,
}

impl std::hash::Hash for ManagedBindingV1 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.parent_session_id.hash(state);
        match self.profile {
            cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1 => 1_u8.hash(state),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLocation {
    pub machine_id: String,
    pub cwd: String,
}

/// A target-local managed child. This deliberately does not decode as a V1
/// remote environment: historical launchers retain the record and refuse it.
/// The exact Provider must independently support the selected native profile.
/// The binding is constant for the child's lifetime; each call round's input
/// revision lives in the Machine-owned round marker, never in the binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedChildV1 {
    pub schema: u16,
    pub phase: String,
    pub session_id: String,
    pub parent_session_id: String,
    pub machine_id: String,
    pub workspace_id: String,
    pub cwd: String,
    pub profile: cowboy_provider_sdk::ManagedRuntimeProfile,
}

impl ManagedChildV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != 1
            || self.phase != "managed_child"
            || self.machine_id == "local"
            || self.session_id == self.parent_session_id
            || ![
                &self.session_id,
                &self.parent_session_id,
                &self.machine_id,
                &self.workspace_id,
            ]
            .into_iter()
            .all(|id| crate::managed_calls::valid_id(id))
            || !valid_path(&self.cwd)
        {
            return Err("invalid managed child binding");
        }
        Ok(())
    }

    pub fn accepts(&self, session: &str, machine: &str, cwd: &str) -> bool {
        self.validate().is_ok()
            && self.session_id == session
            && self.machine_id == machine
            && self.cwd == cwd
    }
}

/// A durable creation intent. It deliberately cannot decode as a runnable
/// binding: older readers and every ordinary launch path must fail closed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationV1 {
    pub schema: u16,
    pub phase: String,
    pub runtime: RuntimeLocation,
    pub machine_id: String,
    pub workspace_id: String,
    pub source_path: String,
    pub executor_digest: String,
}

/// Explicit maintenance intent. Ordinary launch and older readers fail closed
/// while the target replaces its process lifetime. Native conversation identity
/// and workspace ownership remain in the existing Session record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryV1 {
    pub schema: u16,
    pub phase: String,
    pub operation_id: String,
    pub previous: BindingV1,
}

impl RecoveryV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != 1
            || self.phase != "recovering"
            || self.operation_id.is_empty()
            || self.operation_id.len() > 128
            || !self
                .operation_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            || self.previous.revision == u64::MAX
        {
            return Err("invalid execution recovery intent");
        }
        self.previous.validate()
    }

    pub fn accepts(&self, next: &BindingV1) -> bool {
        self.validate().is_ok()
            && next.validate().is_ok()
            && next.id == self.previous.id
            && next.revision == self.previous.revision + 1
            && next.runtime == self.previous.runtime
            && next.workspace == self.previous.workspace
            && next.access == self.previous.access
            && next.managed == self.previous.managed
            && next.environment.machine_id == self.previous.environment.machine_id
            && next.environment.executor_digest == self.previous.environment.executor_digest
            && next.environment.protocol == self.previous.environment.protocol
            && next.environment.id != self.previous.environment.id
            && next.environment.incarnation != self.previous.environment.incarnation
    }
}

impl PreparationV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != EXECUTION_BINDING_SCHEMA
            || self.phase != "preparing"
            || ![
                &self.runtime.machine_id,
                &self.machine_id,
                &self.workspace_id,
            ]
            .into_iter()
            .all(|value| valid_id(value))
            || self.runtime.machine_id == "local"
            || self.machine_id == "local"
            || self.runtime.machine_id == self.machine_id
            || !valid_path(&self.runtime.cwd)
            || !valid_path(&self.source_path)
            || !valid_digest(&self.executor_digest)
        {
            return Err("execution environment preparation is invalid");
        }
        Ok(())
    }

    pub fn accepts(&self, binding: &BindingV1) -> bool {
        self.validate().is_ok()
            && binding.validate().is_ok()
            && binding.runtime == self.runtime
            && binding.environment.machine_id == self.machine_id
            && binding.environment.executor_digest == self.executor_digest
            && binding.workspace.id == self.workspace_id
            && binding.workspace.source_path == self.source_path
            && binding.access == ExecutionAccess::Project
            && binding.managed.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentLocation {
    pub machine_id: String,
    /// Target-owned environment and process-lifetime identity, not a path or
    /// the upstream executor's build/provider identifier.
    pub id: String,
    pub incarnation: String,
    pub executor_digest: String,
    pub protocol: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLocation {
    pub id: String,
    pub worktree_id: String,
    pub source_path: String,
    pub cwd: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAccess {
    Project,
    Host,
}

impl ExecutionBinding {
    pub fn managed_child(&self) -> Option<ManagedChildV1> {
        let child = ManagedChildV1::deserialize(&self.0).ok()?;
        child.validate().ok()?;
        Some(child)
    }

    /// The parent and profile of any managed child, local or remote.
    pub fn managed(&self) -> Option<ManagedBindingV1> {
        if let Some(child) = self.managed_child() {
            return Some(ManagedBindingV1 {
                parent_session_id: child.parent_session_id,
                profile: child.profile,
            });
        }
        self.decode().ok()?.managed
    }
    pub fn from_record(record: serde_json::Value) -> Self {
        Self(record)
    }

    pub fn record(&self) -> &serde_json::Value {
        &self.0
    }

    pub fn decode(&self) -> Result<BindingV1, &'static str> {
        let binding = BindingV1::deserialize(&self.0)
            .map_err(|_| "execution environment binding is unsupported or invalid")?;
        binding.validate()?;
        Ok(binding)
    }

    pub fn preparation(&self) -> Option<PreparationV1> {
        let preparation = PreparationV1::deserialize(&self.0).ok()?;
        preparation.validate().ok()?;
        Some(preparation)
    }

    pub fn recovery(&self) -> Option<RecoveryV1> {
        let recovery = RecoveryV1::deserialize(&self.0).ok()?;
        recovery.validate().ok()?;
        Some(recovery)
    }

    pub fn for_runtime(&self, machine_id: &str, cwd: &str) -> Result<BindingV1, &'static str> {
        let binding = self.decode()?;
        if binding.runtime.machine_id != machine_id || binding.runtime.cwd != cwd {
            return Err("execution environment does not match the session runtime");
        }
        Ok(binding)
    }
}

/// Missing fields are legacy. A present JSON null is malformed and must stay
/// present across a metadata round trip rather than becoming runtime-local.
pub fn deserialize_optional_binding<'de, D>(
    deserializer: D,
) -> Result<Option<ExecutionBinding>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer)
        .map(|record| Some(ExecutionBinding::from_record(record)))
}

impl BindingV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != EXECUTION_BINDING_SCHEMA
            || self.environment.protocol != EXECUTION_PROTOCOL
        {
            return Err("execution environment binding requires a newer Cowboy");
        }
        if self.revision == 0
            || ![
                &self.id,
                &self.runtime.machine_id,
                &self.environment.machine_id,
                &self.environment.id,
                &self.environment.incarnation,
                &self.workspace.id,
                &self.workspace.worktree_id,
            ]
            .into_iter()
            .all(|value| valid_id(value))
            || self.environment.machine_id == "local"
            || ![
                &self.runtime.cwd,
                &self.workspace.source_path,
                &self.workspace.cwd,
            ]
            .into_iter()
            .all(|value| valid_path(value))
            || !valid_digest(&self.environment.executor_digest)
            || self.string_bytes() > MAX_BINDING_BYTES
            || self.managed.as_ref().is_some_and(|managed| {
                // A remote managed child executes elsewhere, only in a
                // Machine-owned snapshot it is named after.
                !crate::managed_calls::valid_id(&managed.parent_session_id)
                    || managed.parent_session_id == self.workspace.worktree_id
                    || self.runtime.machine_id == self.environment.machine_id
                    || self.access != ExecutionAccess::Project
            })
        {
            return Err("execution environment binding is invalid");
        }
        Ok(())
    }

    pub fn string_bytes(&self) -> usize {
        [
            &self.id,
            &self.runtime.machine_id,
            &self.runtime.cwd,
            &self.environment.machine_id,
            &self.environment.id,
            &self.environment.incarnation,
            &self.environment.executor_digest,
            &self.workspace.id,
            &self.workspace.worktree_id,
            &self.workspace.source_path,
            &self.workspace.cwd,
        ]
        .into_iter()
        .chain(
            self.managed
                .as_ref()
                .map(|managed| &managed.parent_session_id),
        )
        .map(String::len)
        .sum()
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
}

fn valid_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 4096
        && !value.chars().any(char::is_control)
        && !value.split('/').any(|part| part == "." || part == "..")
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
pub(crate) fn fixture() -> ExecutionBinding {
    ExecutionBinding::from_record(serde_json::json!({
        "schema": 1, "id": "binding-one", "revision": 1,
        "runtime": {"machine_id": "ovh", "cwd": "/runtime/session"},
        "environment": {
            "machine_id": "hawk", "id": "environment-one", "incarnation": "incarnation-one",
            "executor_digest": format!("sha256:{}", "ab".repeat(32)), "protocol": 1,
        },
        "workspace": {
            "id": "cowboy", "worktree_id": "worktree-one", "source_path": "/sources/cowboy",
            "cwd": "/tasks/cowboy",
        },
        "access": "project",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_recovery_fences_launch_and_requires_new_lifetime_same_workspace() {
        let previous = fixture().decode().unwrap();
        let intent = RecoveryV1 {
            schema: 1,
            phase: "recovering".into(),
            operation_id: "repair-1".into(),
            previous: previous.clone(),
        };
        let record = ExecutionBinding::from_record(serde_json::to_value(&intent).unwrap());
        assert!(record.decode().is_err());
        assert!(record.preparation().is_none());
        assert_eq!(record.recovery(), Some(intent.clone()));
        assert!(!intent.accepts(&previous));
        let mut next = previous;
        next.revision += 1;
        next.environment.id = "new-environment".into();
        next.environment.incarnation = "new-incarnation".into();
        assert!(intent.accepts(&next));
        next.workspace.cwd = "/different/worktree".into();
        assert!(!intent.accepts(&next));
        for operation in ["../escape", "a/b", "", "operation.with.dot"] {
            let mut invalid = intent.clone();
            invalid.operation_id = operation.into();
            assert!(invalid.validate().is_err());
        }
        let mut invalid = intent;
        invalid.previous.revision = u64::MAX;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn managed_children_cannot_be_misread_as_unrestricted_local_or_remote_sessions() {
        let child = ManagedChildV1 {
            schema: 1,
            phase: "managed_child".into(),
            session_id: "child-1".into(),
            parent_session_id: "parent-1".into(),
            machine_id: "hawk".into(),
            workspace_id: "cowboy".into(),
            cwd: "/owned/snapshot/workspace".into(),
            profile: cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
        };
        let binding = ExecutionBinding::from_record(serde_json::to_value(&child).unwrap());
        assert_eq!(binding.managed_child(), Some(child.clone()));
        assert!(binding.decode().is_err());
        assert!(binding.for_runtime("hawk", &child.cwd).is_err());
        assert!(child.accepts("child-1", "hawk", &child.cwd));
        assert!(!child.accepts("child-1", "ovh", &child.cwd));
        assert!(!child.accepts("other-child", "hawk", &child.cwd));
        let mut malformed = binding.record().clone();
        malformed["profile"] = serde_json::json!("full_access");
        let malformed = ExecutionBinding::from_record(malformed);
        assert!(malformed.managed_child().is_none());
        assert!(malformed.decode().is_err());
    }

    #[test]
    fn split_managed_children_are_ordinary_remote_bindings_with_a_parent() {
        let mut record = fixture().record().clone();
        record["workspace"]["worktree_id"] = serde_json::json!("child-1");
        record["managed"] =
            serde_json::json!({"parent_session_id": "parent-1", "profile": "read_only_v1"});
        let binding = ExecutionBinding::from_record(record.clone());
        let decoded = binding.decode().unwrap();
        let managed = binding.managed().unwrap();
        assert_eq!(managed.parent_session_id, "parent-1");
        assert_eq!(decoded.managed, Some(managed));
        assert!(binding.managed_child().is_none());
        assert!(fixture().managed().is_none());
        // Same-Machine, self-parented, host-scoped or unknown profiles refuse.
        for (path, value) in [
            ("/environment/machine_id", serde_json::json!("ovh")),
            ("/managed/parent_session_id", serde_json::json!("child-1")),
            ("/access", serde_json::json!("host")),
            ("/managed/profile", serde_json::json!("full_access")),
            ("/managed/parent_session_id", serde_json::json!("../parent")),
        ] {
            let mut invalid = record.clone();
            *invalid.pointer_mut(path).unwrap() = value;
            let invalid = ExecutionBinding::from_record(invalid);
            assert!(invalid.decode().is_err(), "{path}");
            assert!(invalid.managed().is_none(), "{path}");
        }
        // Neither creation nor recovery can add or drop the constraint.
        let ordinary = fixture().decode().unwrap();
        let intent = PreparationV1 {
            schema: 1,
            phase: "preparing".into(),
            runtime: decoded.runtime.clone(),
            machine_id: decoded.environment.machine_id.clone(),
            workspace_id: decoded.workspace.id.clone(),
            source_path: decoded.workspace.source_path.clone(),
            executor_digest: decoded.environment.executor_digest.clone(),
        };
        assert!(intent.accepts(&ordinary));
        assert!(!intent.accepts(&decoded));
        let recovery = RecoveryV1 {
            schema: 1,
            phase: "recovering".into(),
            operation_id: "repair-1".into(),
            previous: decoded.clone(),
        };
        let mut next = decoded;
        next.revision += 1;
        next.environment.id = "new-environment".into();
        next.environment.incarnation = "new-incarnation".into();
        assert!(recovery.accepts(&next));
        next.managed = None;
        assert!(!recovery.accepts(&next));
    }

    #[test]
    fn execution_preparation_never_decodes_as_launchable_and_matches_exact_target() {
        let ready = fixture().decode().unwrap();
        let intent = PreparationV1 {
            schema: 1,
            phase: "preparing".into(),
            runtime: ready.runtime.clone(),
            machine_id: ready.environment.machine_id.clone(),
            workspace_id: ready.workspace.id.clone(),
            source_path: ready.workspace.source_path.clone(),
            executor_digest: ready.environment.executor_digest.clone(),
        };
        let pending = ExecutionBinding::from_record(serde_json::to_value(&intent).unwrap());
        assert_eq!(pending.preparation(), Some(intent.clone()));
        assert!(pending.decode().is_err());
        assert!(intent.accepts(&ready));
        let mut changed = ready.clone();
        changed.environment.executor_digest = format!("sha256:{}", "a".repeat(64));
        assert!(!intent.accepts(&changed));
        changed = ready.clone();
        changed.workspace.source_path = "/different/source".into();
        assert!(!intent.accepts(&changed));
        changed = ready;
        changed.runtime.cwd = "/different/entry".into();
        assert!(!intent.accepts(&changed));
    }

    #[test]
    fn binding_retains_unsupported_records_without_resolving_them() {
        for (field, value) in [
            ("schema", serde_json::json!(2)),
            ("access", serde_json::json!("future-access")),
            ("unknown", serde_json::json!({"future": true})),
        ] {
            let mut record = fixture().0;
            record[field] = value;
            let restored: ExecutionBinding = serde_json::from_value(record.clone()).unwrap();
            assert!(restored.decode().is_err());
            assert_eq!(serde_json::to_value(restored).unwrap(), record);
        }
    }

    #[test]
    fn binding_requires_exact_runtime_and_target_identities() {
        let binding = fixture();
        assert!(binding.for_runtime("ovh", "/runtime/session").is_ok());
        assert!(binding.for_runtime("hawk", "/runtime/session").is_err());
        assert!(binding.for_runtime("ovh", "/tasks/cowboy").is_err());
        for pointer in [
            "/id",
            "/runtime/machine_id",
            "/environment/id",
            "/environment/incarnation",
            "/workspace/id",
            "/workspace/worktree_id",
            "/environment/executor_digest",
            "/workspace/source_path",
            "/workspace/cwd",
        ] {
            let mut record = binding.0.clone();
            *record.pointer_mut(pointer).unwrap() = "".into();
            assert!(
                ExecutionBinding::from_record(record).decode().is_err(),
                "{pointer}"
            );
        }
    }

    #[test]
    fn binding_refuses_new_protocol_invalid_paths_and_unversioned_identity() {
        for (pointer, value) in [
            ("/environment/protocol", serde_json::json!(2)),
            ("/environment/machine_id", serde_json::json!("local")),
            ("/revision", serde_json::json!(0)),
            ("/workspace/cwd", serde_json::json!("../runtime")),
            ("/workspace/cwd", serde_json::json!("/tasks/../other")),
            ("/workspace/cwd", serde_json::json!("/tasks/\u{0}other")),
            (
                "/workspace/cwd",
                serde_json::json!(format!("/{}", "x".repeat(4096))),
            ),
        ] {
            let mut record = fixture().0;
            *record.pointer_mut(pointer).unwrap() = value;
            assert!(
                ExecutionBinding::from_record(record).decode().is_err(),
                "{pointer}"
            );
        }
        let mut record = fixture().0;
        record["workspace"]["cwd"] = "/tasks/带空格的 项目".into();
        assert!(ExecutionBinding::from_record(record).decode().is_ok());
    }
}

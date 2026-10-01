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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLocation {
    pub machine_id: String,
    pub cwd: String,
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

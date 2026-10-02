//! Service-owned runtime policy. Project roots remain owned by their Machines.
use anyhow::ensure;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Disabled,
    Local,
    Remote,
    #[default]
    Either,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Policy {
    pub agent_mode: Mode,
    pub hosts_projects: bool,
    /// None permits any enrolled target; an empty set permits none.
    pub remote_targets: Option<Vec<String>>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            agent_mode: Mode::Either,
            hosts_projects: true,
            remote_targets: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Configuration {
    pub schema: u16,
    pub revision: String,
    pub default_runtime_machine_id: Option<String>,
    pub machines: BTreeMap<String, Policy>,
}

impl Configuration {
    pub(crate) fn policy(&self, machine: &str) -> Policy {
        self.machines.get(machine).cloned().unwrap_or_default()
    }

    pub(crate) fn allows(&self, runtime: &str, target: &str) -> bool {
        let policy = self.policy(runtime);
        if !self.policy(target).hosts_projects {
            return false;
        }
        if runtime == target {
            return matches!(policy.agent_mode, Mode::Local | Mode::Either);
        }
        matches!(policy.agent_mode, Mode::Remote | Mode::Either)
            && policy
                .remote_targets
                .as_ref()
                .is_none_or(|targets| targets.iter().any(|id| id == target))
    }
}

pub(crate) struct Store {
    path: PathBuf,
    value: parking_lot::Mutex<Configuration>,
}

impl Store {
    pub(crate) fn new(
        directory: &Path,
        default_runtime_machine_id: Option<String>,
    ) -> anyhow::Result<Self> {
        let path = directory.join("project-placement.json");
        let value: Configuration = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Configuration {
                schema: 1,
                revision: "bootstrap".into(),
                default_runtime_machine_id,
                machines: BTreeMap::new(),
            },
            Err(error) => return Err(error.into()),
        };
        ensure!(
            value.schema == 1 && !value.revision.is_empty(),
            "invalid project placement configuration"
        );
        Ok(Self {
            path,
            value: parking_lot::Mutex::new(value),
        })
    }

    pub(crate) fn snapshot(&self) -> Configuration {
        self.value.lock().clone()
    }

    pub(crate) fn update(
        &self,
        expected_revision: &str,
        machine: String,
        policy: Policy,
        preferred: bool,
    ) -> anyhow::Result<Configuration> {
        let mut current = self.value.lock();
        ensure!(
            current.revision == expected_revision,
            "Machine policies changed; reload before saving"
        );
        let mut next = current.clone();
        next.machines.insert(machine.clone(), policy);
        if preferred {
            next.default_runtime_machine_id = Some(machine);
        } else if next.default_runtime_machine_id.as_ref() == Some(&machine) {
            next.default_runtime_machine_id = None;
        }
        next.revision = format!("placement-{:032x}", rand::random::<u128>());
        crate::owned_json::write(&self.path, &next)?;
        *current = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_only_policy_is_durable_and_enforces_both_ends() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path(), Some("ovh".into())).unwrap();
        let policy = Policy {
            agent_mode: Mode::Remote,
            hosts_projects: false,
            remote_targets: Some(vec!["hawk".into()]),
        };
        let saved = store
            .update("bootstrap", "ovh".into(), policy.clone(), true)
            .unwrap();
        assert!(saved.allows("ovh", "hawk"));
        assert!(!saved.allows("ovh", "ovh"));
        assert!(!saved.allows("ovh", "falcon"));
        assert!(!saved.allows("hawk", "ovh"));
        assert!(
            store
                .update("bootstrap", "ovh".into(), policy, false)
                .is_err()
        );
        let restart = Store::new(dir.path(), None).unwrap();
        assert_eq!(restart.snapshot().revision, saved.revision);
        assert!(!restart.snapshot().allows("ovh", "ovh"));
    }
}

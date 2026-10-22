//! Machine-owned project registration. IDs and paths are independent of labels.
use serde::{Deserialize, Serialize};

use super::MachineWorkspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Host,
    Cowboy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    List,
    Adopt {
        expected_revision: String,
    },
    Upsert {
        expected_revision: String,
        project: MachineWorkspace,
    },
    Remove {
        expected_revision: String,
        id: String,
    },
    Discover {
        root: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub schema: u16,
    pub revision: String,
    /// Before the first edit, the host workspace file supplies bootstrap roots.
    /// After it, this registry alone owns the Machine's project inventory.
    pub managed: bool,
    pub projects: Vec<MachineWorkspace>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Discovery {
    pub root: String,
    pub paths: Vec<String>,
    pub truncated: bool,
}

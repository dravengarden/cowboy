//! Closed workspace resource wire model. Credentials and raw API JSON stay local.

#[cfg(feature = "machine-host")]
use cowboy_plugin_sdk::WorkspaceExtensionContract;
use cowboy_plugin_sdk::WorkspaceResourceFilter;
use serde::{Deserialize, Serialize};

#[cfg(feature = "machine-host")]
pub(crate) mod runtime;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub root: String,
    pub operation: Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Operation {
    Inventory,
    Read {
        identity: Identity,
        remote: String,
        view: String,
        #[serde(default)]
        item: Option<String>,
        #[serde(default)]
        filter: Option<String>,
        #[serde(default = "first_page")]
        page: u32,
    },
}

fn first_page() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Identity {
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Extension {
    pub identity: Identity,
    pub label: String,
    pub description: String,
    pub views: Vec<View>,
    pub available: bool,
}

impl Extension {
    #[cfg(feature = "machine-host")]
    pub(crate) fn from_contract(
        identity: Identity,
        contract: &WorkspaceExtensionContract,
        available: bool,
    ) -> Self {
        Self {
            identity,
            label: contract.display_name.clone(),
            description: contract.description.clone(),
            views: contract
                .views
                .iter()
                .map(|v| View {
                    id: v.id.clone(),
                    label: v.label.clone(),
                    filters: v.filters.clone(),
                })
                .collect(),
            available,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct View {
    pub id: String,
    pub label: String,
    pub filters: Vec<WorkspaceResourceFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Remote {
    pub name: String,
    pub host: String,
    pub owner: String,
    pub repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Resource {
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub body: Option<String>,
    pub body_truncated: bool,
    pub state: Option<String>,
    pub updated_at: Option<String>,
    pub metadata: Vec<Metadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Response {
    Inventory {
        extensions: Vec<Extension>,
        remotes: Vec<Remote>,
    },
    Page {
        items: Vec<Resource>,
        #[serde(rename = "nextPage")]
        next_page: Option<u32>,
    },
    Detail {
        item: Resource,
    },
    Unavailable {
        code: Failure,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    MachineUnavailable,
    ExtensionChanged,
    DependencyUnavailable,
    RepositoryUnavailable,
    ConnectionUnavailable,
    RequestFailed,
    Busy,
    InvalidRequest,
}

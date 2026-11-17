//! The resolved launch inputs of one Provider package for a worker.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ProviderLaunchContext {
    pub package_path: PathBuf,
    pub command: String,
    pub version: String,
    pub behavior: cowboy_provider_sdk::ProviderBehaviorContract,
    pub environment: BTreeMap<String, String>,
    pub remove_environment: BTreeSet<String>,
    pub remove_environment_prefixes: BTreeSet<String>,
    pub home: Option<PathBuf>,
    pub execution_jsonrpc: bool,
    pub mcp_bindings: bool,
    pub network: Option<cowboy_provider_sdk::RuntimeNetworkPolicy>,
}

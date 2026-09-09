//! Read-only, deterministic structural linking across Service/Machine sites.
//!
//! Nothing here reads a Catalog, credentials, policy, inventory or session state.
//! Input releases and ports are UNTRUSTED claims. A checked proposal is not an
//! AuthorizedPlan/VerifiedRelease; no installer or executor accepts it. Runtime
//! integration must resolve verified packages, authority, fences and leases.

mod json;
mod wire;

use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use wire::{
    Binding, Cardinality, Composition, Execution, Identity, Node, NodeId, Ownership, Scope,
    ScopeId, ScopeKind, ServiceId, Site, Transport, Validate as _, Visibility,
};

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckError {
    InvalidJson,
    InvalidContract,
    DuplicateScope,
    InvalidScopeTree,
    DuplicateNode,
    UnknownScope,
    InvalidPlacement,
    ServiceExecutable,
    GenerationConflict,
    InvalidOwner,
    DuplicatePort,
    PortBudget,
    InvalidEndpoint,
    DuplicateBinding,
    ContractMismatch,
    ScopeNotVisible,
    LocalPortCrossesSite,
    CardinalityMismatch,
    DependencyCycle,
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Bounded codes only: input may contain credentials or private paths.
        let code = serde_json::to_value(self).expect("static error code");
        formatter.write_str(code.as_str().expect("static error code is a string"))
    }
}
impl std::error::Error for CheckError {}

// Serialize only; private construction. This report cannot resurrect authority.
#[derive(Debug, Serialize)]
pub(crate) struct CheckedStructure {
    status: &'static str,
    authorized: bool,
    contract_fingerprint: &'static str,
    proposal_digest: String,
    dependency_order: Vec<NodeId>,
    reverse_dependency_order: Vec<NodeId>,
    sites: Vec<SiteProjection>,
    remote_bindings: Vec<Binding>,
}
#[derive(Debug, Serialize)]
struct SiteProjection {
    site: Site,
    nodes: Vec<NodeId>,
}

pub(crate) fn check_file(path: &std::path::Path) -> anyhow::Result<()> {
    use anyhow::Context as _;
    // Reject known pipes/devices before open (a FIFO could otherwise block).
    anyhow::ensure!(
        std::fs::metadata(path)?.is_file(),
        "composition proposal must be a regular file"
    );
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .context("cannot open composition proposal")?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "composition proposal must be a regular file"
    );
    let mut bytes = Vec::new();
    // Metadata can race; enforce the byte budget while reading too.
    file.take(wire::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let report = check(&bytes)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn decode(bytes: &[u8]) -> Result<Composition, CheckError> {
    let value = json::parse(bytes)?;
    let proposal: Composition =
        serde_json::from_value(value).map_err(|_| CheckError::InvalidContract)?;
    if !proposal.valid() {
        return Err(CheckError::InvalidContract);
    }
    Ok(proposal)
}
fn parent(scope: &ScopeKind) -> Option<&ScopeId> {
    match scope {
        ScopeKind::Service { .. } => None,
        ScopeKind::Machine { parent, .. }
        | ScopeKind::Workspace { parent, .. }
        | ScopeKind::Session { parent, .. }
        | ScopeKind::Operation { parent, .. } => Some(parent),
    }
}
fn ancestry<'a>(
    id: &ScopeId,
    scopes: &BTreeMap<&ScopeId, &'a Scope>,
) -> Result<Vec<&'a Scope>, CheckError> {
    let mut result = Vec::new();
    let mut next = Some(id);
    while let Some(id) = next {
        let scope = *scopes.get(id).ok_or(CheckError::UnknownScope)?;
        if result.contains(&scope) || result.len() >= 32 {
            return Err(CheckError::InvalidScopeTree);
        }
        result.push(scope);
        next = parent(&scope.lifetime);
    }
    Ok(result)
}
fn validate_scopes(scopes: &BTreeMap<&ScopeId, &Scope>) -> Result<ServiceId, CheckError> {
    let mut lifetimes = BTreeSet::new();
    let mut service = None;
    for scope in scopes.values() {
        if !lifetimes.insert(&scope.lifetime) {
            return Err(CheckError::DuplicateScope);
        }
        let chain = ancestry(&scope.id, scopes)?;
        if let ScopeKind::Service { service_id } = &scope.lifetime {
            if service.replace(service_id.clone()).is_some() {
                return Err(CheckError::InvalidScopeTree);
            }
        } else {
            let owner = &chain.get(1).ok_or(CheckError::InvalidScopeTree)?.lifetime;
            let valid = matches!(
                (&scope.lifetime, owner),
                (ScopeKind::Machine { .. }, ScopeKind::Service { .. })
                    | (ScopeKind::Workspace { .. }, ScopeKind::Machine { .. })
                    | (ScopeKind::Session { .. }, ScopeKind::Workspace { .. })
                    | (
                        ScopeKind::Operation { .. },
                        ScopeKind::Service { .. }
                            | ScopeKind::Machine { .. }
                            | ScopeKind::Workspace { .. }
                            | ScopeKind::Session { .. }
                    )
            );
            if !valid {
                return Err(CheckError::InvalidScopeTree);
            }
        }
    }
    service.ok_or(CheckError::InvalidScopeTree)
}
fn site_service(site: &Site) -> &ServiceId {
    match site {
        Site::Service { service_id } | Site::Machine { service_id, .. } => service_id,
    }
}

fn check(bytes: &[u8]) -> Result<CheckedStructure, CheckError> {
    let mut proposal = decode(bytes)?;
    // Canonical graph identity ignores authoring order. Many bindings aggregate
    // by consumer/provider endpoint order, never registration or arrival order.
    proposal.scopes.sort_by(|a, b| a.id.cmp(&b.id));
    proposal.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    proposal.bindings.sort();
    for node in &mut proposal.nodes {
        node.provides.sort_by(|a, b| a.id.cmp(&b.id));
        node.requires.sort_by(|a, b| a.id.cmp(&b.id));
    }
    let scopes: BTreeMap<_, _> = proposal
        .scopes
        .iter()
        .map(|scope| (&scope.id, scope))
        .collect();
    if scopes.len() != proposal.scopes.len() {
        return Err(CheckError::DuplicateScope);
    }
    let service = validate_scopes(&scopes)?;
    let nodes: BTreeMap<_, _> = proposal.nodes.iter().map(|node| (&node.id, node)).collect();
    if nodes.len() != proposal.nodes.len() {
        return Err(CheckError::DuplicateNode);
    }
    let mut dependencies: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    let mut generations = BTreeMap::new();
    let mut port_count = 0;
    for node in nodes.values() {
        let chain = ancestry(&node.scope, &scopes)?;
        if site_service(&node.site) != &service {
            return Err(CheckError::InvalidPlacement);
        }
        if let Some(machine) = chain.iter().find_map(|scope| match &scope.lifetime {
            ScopeKind::Machine { machine_id, .. } => Some(machine_id),
            _ => None,
        }) && !matches!(&node.site, Site::Machine { machine_id, .. } if machine_id == machine)
        {
            return Err(CheckError::InvalidPlacement);
        }
        if let Identity::Plugin {
            release,
            generation,
            execution,
            ..
        } = &node.identity
        {
            if *execution == Execution::Isolated && matches!(node.site, Site::Service { .. }) {
                return Err(CheckError::ServiceExecutable);
            }
            let slot = (&node.site, &release.plugin_id, generation);
            if let Some(previous) = generations.insert(slot, release)
                && previous != release
            {
                return Err(CheckError::GenerationConflict);
            }
        }
        let prerequisites = dependencies.entry(node.id.clone()).or_default();
        if let Ownership::Node { node: owner_id } = &node.owner {
            let owner = nodes.get(owner_id).ok_or(CheckError::InvalidOwner)?;
            if owner.id == node.id
                || owner.site != node.site
                || !chain.iter().any(|s| s.id == owner.scope)
            {
                return Err(CheckError::InvalidOwner);
            }
            prerequisites.insert(owner.id.clone());
        }
        if node
            .provides
            .iter()
            .map(|p| &p.id)
            .collect::<BTreeSet<_>>()
            .len()
            != node.provides.len()
            || node
                .requires
                .iter()
                .map(|p| &p.id)
                .collect::<BTreeSet<_>>()
                .len()
                != node.requires.len()
        {
            return Err(CheckError::DuplicatePort);
        }
        port_count += node.provides.len() + node.requires.len();
        if port_count > 2048 {
            return Err(CheckError::PortBudget);
        }
    }
    let mut seen = BTreeSet::new();
    let mut cardinalities = BTreeMap::new();
    let mut remote_bindings = Vec::new();
    for binding in &proposal.bindings {
        if !seen.insert((&binding.consumer, &binding.provider)) {
            return Err(CheckError::DuplicateBinding);
        }
        let consumer = endpoint_node(&binding.consumer.node, &nodes)?;
        let provider = endpoint_node(&binding.provider.node, &nodes)?;
        let required = consumer
            .requires
            .iter()
            .find(|p| p.id == binding.consumer.port)
            .ok_or(CheckError::InvalidEndpoint)?;
        let provided = provider
            .provides
            .iter()
            .find(|p| p.id == binding.provider.port)
            .ok_or(CheckError::InvalidEndpoint)?;
        if required.contract != provided.contract {
            return Err(CheckError::ContractMismatch);
        }
        let visible = consumer.scope == provider.scope
            || provided.visibility == Visibility::Descendants
                && ancestry(&consumer.scope, &scopes)?
                    .iter()
                    .any(|scope| scope.id == provider.scope);
        if !visible {
            return Err(CheckError::ScopeNotVisible);
        }
        if consumer.site != provider.site {
            if provided.transport == Transport::Local {
                return Err(CheckError::LocalPortCrossesSite);
            }
            remote_bindings.push(binding.clone());
        }
        *cardinalities
            .entry((&consumer.id, &required.id))
            .or_insert(0_usize) += 1;
        dependencies
            .get_mut(&consumer.id)
            .expect("known node")
            .insert(provider.id.clone());
    }
    for node in nodes.values() {
        for required in &node.requires {
            let count = *cardinalities.get(&(&node.id, &required.id)).unwrap_or(&0);
            if matches!(required.cardinality, Cardinality::One) && count != 1
                || matches!(required.cardinality, Cardinality::Optional) && count > 1
            {
                return Err(CheckError::CardinalityMismatch);
            }
        }
    }
    let mut dependency_order = Vec::new();
    while !dependencies.is_empty() {
        let ready = dependencies
            .iter()
            .find(|(_, deps)| deps.is_empty())
            .map(|(id, _)| id.clone())
            .ok_or(CheckError::DependencyCycle)?;
        dependencies.remove(&ready);
        for prerequisites in dependencies.values_mut() {
            prerequisites.remove(&ready);
        }
        dependency_order.push(ready);
    }
    let mut sites: BTreeMap<Site, Vec<NodeId>> = BTreeMap::new();
    for id in &dependency_order {
        sites
            .entry(nodes[id].site.clone())
            .or_default()
            .push(id.clone());
    }
    let mut digest = Sha256::new();
    digest.update(b"cowboy.composition.proposal.v1\n");
    digest.update(wire::CONTRACT_FINGERPRINT.as_bytes());
    digest.update(b"\n");
    digest.update(
        serde_json::to_vec(&canonical_value(
            serde_json::to_value(&proposal).expect("bounded wire structs serialize"),
        ))
        .expect("bounded canonical JSON"),
    );
    Ok(CheckedStructure {
        status: "structurally_valid",
        authorized: false,
        contract_fingerprint: wire::CONTRACT_FINGERPRINT,
        proposal_digest: format!("sha256:{:x}", digest.finalize()),
        reverse_dependency_order: dependency_order.iter().rev().cloned().collect(),
        dependency_order,
        sites: sites
            .into_iter()
            .map(|(site, nodes)| SiteProjection { site, nodes })
            .collect(),
        remote_bindings,
    })
}

// Stable even if serde_json's preserve_order feature is enabled by a dependency
// or a schema editor changes record property order without changing meaning.
fn canonical_value(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .map(|(key, value)| (key, canonical_value(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(canonical_value).collect()),
        value => value,
    }
}
fn endpoint_node<'a>(
    id: &NodeId,
    nodes: &BTreeMap<&NodeId, &'a Node>,
) -> Result<&'a Node, CheckError> {
    nodes.get(id).copied().ok_or(CheckError::InvalidEndpoint)
}
#[cfg(test)]
mod tests;

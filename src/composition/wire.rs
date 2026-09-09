// Generated from contracts/composition-v1.schema.json; do not edit.
use serde::{Deserialize, Serialize};

pub(super) const CONTRACT_FINGERPRINT: &str =
    "sha256:095e34ff676d96f123bb3e2c0adb2aab6f46458e880d46edef3abc557b0a2f39";
pub(super) const MAX_BYTES: usize = 1048576;
pub(super) const MAX_DEPTH: usize = 32;
pub(super) trait Validate {
    fn valid(&self) -> bool;
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct ServiceId(pub(super) String);
impl Validate for ServiceId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct MachineId(pub(super) String);
impl Validate for MachineId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct WorkspaceId(pub(super) String);
impl Validate for WorkspaceId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct SessionId(pub(super) String);
impl Validate for SessionId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct OperationId(pub(super) String);
impl Validate for OperationId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct ScopeId(pub(super) String);
impl Validate for ScopeId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct NodeId(pub(super) String);
impl Validate for NodeId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct PortId(pub(super) String);
impl Validate for PortId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct PluginId(pub(super) String);
impl Validate for PluginId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct PublisherId(pub(super) String);
impl Validate for PublisherId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct ContractId(pub(super) String);
impl Validate for ContractId {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^[a-z0-9][a-z0-9._-]*$").expect("generated contract regex")
        });
        self.0.is_ascii() && !self.0.is_empty() && self.0.len() <= 64 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct ExactVersion(pub(super) String);
impl Validate for ExactVersion {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$")
                .expect("generated contract regex")
        });
        self.0.is_ascii() && self.0.len() >= 5 && self.0.len() <= 32 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct ArtifactDigest(pub(super) String);
impl Validate for ArtifactDigest {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^sha256:[a-f0-9]{64}$").expect("generated contract regex")
        });
        self.0.is_ascii() && self.0.len() >= 71 && self.0.len() <= 71 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct ContractFingerprint(pub(super) String);
impl Validate for ContractFingerprint {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^sha256:[a-f0-9]{64}$").expect("generated contract regex")
        });
        self.0.is_ascii() && self.0.len() >= 71 && self.0.len() <= 71 && PATTERN.is_match(&self.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct InstallationGeneration(pub(super) String);
impl Validate for InstallationGeneration {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^(0|[1-9][0-9]*)$").expect("generated contract regex")
        });
        self.0.is_ascii()
            && !self.0.is_empty()
            && self.0.len() <= 20
            && PATTERN.is_match(&self.0)
            && (self.0.len() < 20 || self.0.as_str() <= "18446744073709551615")
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct InstanceIncarnation(pub(super) String);
impl Validate for InstanceIncarnation {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^(0|[1-9][0-9]*)$").expect("generated contract regex")
        });
        self.0.is_ascii()
            && !self.0.is_empty()
            && self.0.len() <= 20
            && PATTERN.is_match(&self.0)
            && (self.0.len() < 20 || self.0.as_str() <= "18446744073709551615")
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub(super) struct BindingRevision(pub(super) String);
impl Validate for BindingRevision {
    fn valid(&self) -> bool {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new("^(0|[1-9][0-9]*)$").expect("generated contract regex")
        });
        self.0.is_ascii()
            && !self.0.is_empty()
            && self.0.len() <= 20
            && PATTERN.is_match(&self.0)
            && (self.0.len() < 20 || self.0.as_str() <= "18446744073709551615")
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum FormatVersion {
    #[serde(rename = "cowboy.composition.v1")]
    CowboyCompositionV1,
}
impl Validate for FormatVersion {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Visibility {
    #[serde(rename = "scope")]
    Scope,
    #[serde(rename = "descendants")]
    Descendants,
}
impl Validate for Visibility {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Transport {
    #[serde(rename = "local")]
    Local,
    #[serde(rename = "remote")]
    Remote,
}
impl Validate for Transport {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Cardinality {
    #[serde(rename = "one")]
    One,
    #[serde(rename = "optional")]
    Optional,
    #[serde(rename = "many")]
    Many,
}
impl Validate for Cardinality {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Execution {
    #[serde(rename = "data")]
    Data,
    #[serde(rename = "isolated")]
    Isolated,
}
impl Validate for Execution {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CoreAnchor {
    #[serde(rename = "coordination")]
    Coordination,
    #[serde(rename = "telemetry_ingress")]
    TelemetryIngress,
    #[serde(rename = "code_client")]
    CodeClient,
}
impl Validate for CoreAnchor {
    fn valid(&self) -> bool {
        true
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum Site {
    #[serde(rename = "service")]
    Service { service_id: ServiceId },
    #[serde(rename = "machine")]
    Machine {
        service_id: ServiceId,
        machine_id: MachineId,
    },
}
impl Validate for Site {
    fn valid(&self) -> bool {
        match self {
            Self::Service { service_id } => service_id.valid(),
            Self::Machine {
                service_id,
                machine_id,
            } => service_id.valid() && machine_id.valid(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum ScopeKind {
    #[serde(rename = "service")]
    Service { service_id: ServiceId },
    #[serde(rename = "machine")]
    Machine {
        parent: ScopeId,
        machine_id: MachineId,
    },
    #[serde(rename = "workspace")]
    Workspace {
        parent: ScopeId,
        workspace_id: WorkspaceId,
    },
    #[serde(rename = "session")]
    Session {
        parent: ScopeId,
        session_id: SessionId,
    },
    #[serde(rename = "operation")]
    Operation {
        parent: ScopeId,
        operation_id: OperationId,
    },
}
impl Validate for ScopeKind {
    fn valid(&self) -> bool {
        match self {
            Self::Service { service_id } => service_id.valid(),
            Self::Machine { parent, machine_id } => parent.valid() && machine_id.valid(),
            Self::Workspace {
                parent,
                workspace_id,
            } => parent.valid() && workspace_id.valid(),
            Self::Session { parent, session_id } => parent.valid() && session_id.valid(),
            Self::Operation {
                parent,
                operation_id,
            } => parent.valid() && operation_id.valid(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct Scope {
    pub(super) id: ScopeId,
    pub(super) lifetime: ScopeKind,
}
impl Validate for Scope {
    fn valid(&self) -> bool {
        self.id.valid() && self.lifetime.valid()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct ReleaseRef {
    pub(super) plugin_id: PluginId,
    pub(super) version: ExactVersion,
    pub(super) digest: ArtifactDigest,
    pub(super) publisher: PublisherId,
}
impl Validate for ReleaseRef {
    fn valid(&self) -> bool {
        self.plugin_id.valid()
            && self.version.valid()
            && self.digest.valid()
            && self.publisher.valid()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum Identity {
    #[serde(rename = "core")]
    Core { anchor: CoreAnchor },
    #[serde(rename = "plugin")]
    Plugin {
        release: ReleaseRef,
        generation: InstallationGeneration,
        incarnation: InstanceIncarnation,
        execution: Execution,
    },
}
impl Validate for Identity {
    fn valid(&self) -> bool {
        match self {
            Self::Core { anchor } => anchor.valid(),
            Self::Plugin {
                release,
                generation,
                incarnation,
                execution,
            } => release.valid() && generation.valid() && incarnation.valid() && execution.valid(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum Ownership {
    #[serde(rename = "scope")]
    Scope {},
    #[serde(rename = "node")]
    Node { node: NodeId },
}
impl Validate for Ownership {
    fn valid(&self) -> bool {
        match self {
            Self::Scope {} => true,
            Self::Node { node } => node.valid(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct ContractRef {
    pub(super) id: ContractId,
    pub(super) version: ExactVersion,
    pub(super) fingerprint: ContractFingerprint,
}
impl Validate for ContractRef {
    fn valid(&self) -> bool {
        self.id.valid() && self.version.valid() && self.fingerprint.valid()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct ProvidedPort {
    pub(super) id: PortId,
    pub(super) contract: ContractRef,
    pub(super) visibility: Visibility,
    pub(super) transport: Transport,
}
impl Validate for ProvidedPort {
    fn valid(&self) -> bool {
        self.id.valid()
            && self.contract.valid()
            && self.visibility.valid()
            && self.transport.valid()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct RequiredPort {
    pub(super) id: PortId,
    pub(super) contract: ContractRef,
    pub(super) cardinality: Cardinality,
}
impl Validate for RequiredPort {
    fn valid(&self) -> bool {
        self.id.valid() && self.contract.valid() && self.cardinality.valid()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct Node {
    pub(super) id: NodeId,
    pub(super) scope: ScopeId,
    pub(super) site: Site,
    pub(super) identity: Identity,
    pub(super) owner: Ownership,
    pub(super) provides: Vec<ProvidedPort>,
    pub(super) requires: Vec<RequiredPort>,
}
impl Validate for Node {
    fn valid(&self) -> bool {
        self.id.valid()
            && self.scope.valid()
            && self.site.valid()
            && self.identity.valid()
            && self.owner.valid()
            && self.provides.len() <= 32
            && self.provides.iter().all(Validate::valid)
            && self.requires.len() <= 32
            && self.requires.iter().all(Validate::valid)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct Endpoint {
    pub(super) node: NodeId,
    pub(super) port: PortId,
}
impl Validate for Endpoint {
    fn valid(&self) -> bool {
        self.node.valid() && self.port.valid()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub(super) consumer: Endpoint,
    pub(super) provider: Endpoint,
    pub(super) revision: BindingRevision,
}
impl Validate for Binding {
    fn valid(&self) -> bool {
        self.consumer.valid() && self.provider.valid() && self.revision.valid()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(super) struct Composition {
    pub(super) format: FormatVersion,
    pub(super) scopes: Vec<Scope>,
    pub(super) nodes: Vec<Node>,
    pub(super) bindings: Vec<Binding>,
}
impl Validate for Composition {
    fn valid(&self) -> bool {
        self.format.valid()
            && !self.scopes.is_empty()
            && self.scopes.len() <= 256
            && self.scopes.iter().all(Validate::valid)
            && !self.nodes.is_empty()
            && self.nodes.len() <= 256
            && self.nodes.iter().all(Validate::valid)
            && self.bindings.len() <= 2048
            && self.bindings.iter().all(Validate::valid)
    }
}

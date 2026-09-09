// Generated from contracts/composition-v1.schema.json; do not edit.
import { strictJson } from "./strict-json.ts";

export const CONTRACT_FINGERPRINT =
  "sha256:095e34ff676d96f123bb3e2c0adb2aab6f46458e880d46edef3abc557b0a2f39";
export const MAX_BYTES = 1048576;
export const MAX_DEPTH = 32;
declare const identity: unique symbol;

function record(
  value: unknown,
  keys: readonly string[],
): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value) &&
    Object.keys(value).length === keys.length &&
    keys.every((key) => Object.hasOwn(value, key));
}
export type ServiceId = string & { readonly [identity]: "ServiceId" };
function isServiceId(value: unknown): value is ServiceId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type MachineId = string & { readonly [identity]: "MachineId" };
function isMachineId(value: unknown): value is MachineId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type WorkspaceId = string & { readonly [identity]: "WorkspaceId" };
function isWorkspaceId(value: unknown): value is WorkspaceId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type SessionId = string & { readonly [identity]: "SessionId" };
function isSessionId(value: unknown): value is SessionId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type OperationId = string & { readonly [identity]: "OperationId" };
function isOperationId(value: unknown): value is OperationId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type ScopeId = string & { readonly [identity]: "ScopeId" };
function isScopeId(value: unknown): value is ScopeId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type NodeId = string & { readonly [identity]: "NodeId" };
function isNodeId(value: unknown): value is NodeId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type PortId = string & { readonly [identity]: "PortId" };
function isPortId(value: unknown): value is PortId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type PluginId = string & { readonly [identity]: "PluginId" };
function isPluginId(value: unknown): value is PluginId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type PublisherId = string & { readonly [identity]: "PublisherId" };
function isPublisherId(value: unknown): value is PublisherId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type ContractId = string & { readonly [identity]: "ContractId" };
function isContractId(value: unknown): value is ContractId {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 64 &&
    new RegExp("^[a-z0-9][a-z0-9._-]*$").exec(value)?.[0] === value;
}
export type ExactVersion = string & { readonly [identity]: "ExactVersion" };
function isExactVersion(value: unknown): value is ExactVersion {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 5 && value.length <= 32 &&
    new RegExp("^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$").exec(
        value,
      )?.[0] === value;
}
export type ArtifactDigest = string & { readonly [identity]: "ArtifactDigest" };
function isArtifactDigest(value: unknown): value is ArtifactDigest {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 71 && value.length <= 71 &&
    new RegExp("^sha256:[a-f0-9]{64}$").exec(value)?.[0] === value;
}
export type ContractFingerprint = string & {
  readonly [identity]: "ContractFingerprint";
};
function isContractFingerprint(value: unknown): value is ContractFingerprint {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 71 && value.length <= 71 &&
    new RegExp("^sha256:[a-f0-9]{64}$").exec(value)?.[0] === value;
}
export type InstallationGeneration = string & {
  readonly [identity]: "InstallationGeneration";
};
function isInstallationGeneration(
  value: unknown,
): value is InstallationGeneration {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 20 &&
    new RegExp("^(0|[1-9][0-9]*)$").exec(value)?.[0] === value &&
    (value.length < 20 || value <= "18446744073709551615");
}
export type InstanceIncarnation = string & {
  readonly [identity]: "InstanceIncarnation";
};
function isInstanceIncarnation(value: unknown): value is InstanceIncarnation {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 20 &&
    new RegExp("^(0|[1-9][0-9]*)$").exec(value)?.[0] === value &&
    (value.length < 20 || value <= "18446744073709551615");
}
export type BindingRevision = string & {
  readonly [identity]: "BindingRevision";
};
function isBindingRevision(value: unknown): value is BindingRevision {
  return typeof value === "string" && /^[\x00-\x7f]*$/.test(value) &&
    value.length >= 1 && value.length <= 20 &&
    new RegExp("^(0|[1-9][0-9]*)$").exec(value)?.[0] === value &&
    (value.length < 20 || value <= "18446744073709551615");
}
export type FormatVersion = "cowboy.composition.v1";
function isFormatVersion(value: unknown): value is FormatVersion {
  return value === "cowboy.composition.v1";
}
export type Visibility = "scope" | "descendants";
function isVisibility(value: unknown): value is Visibility {
  return value === "scope" || value === "descendants";
}
export type Transport = "local" | "remote";
function isTransport(value: unknown): value is Transport {
  return value === "local" || value === "remote";
}
export type Cardinality = "one" | "optional" | "many";
function isCardinality(value: unknown): value is Cardinality {
  return value === "one" || value === "optional" || value === "many";
}
export type Execution = "data" | "isolated";
function isExecution(value: unknown): value is Execution {
  return value === "data" || value === "isolated";
}
export type CoreAnchor = "coordination" | "telemetry_ingress" | "code_client";
function isCoreAnchor(value: unknown): value is CoreAnchor {
  return value === "coordination" || value === "telemetry_ingress" ||
    value === "code_client";
}
export type Site =
  | { readonly kind: "service"; readonly service_id: ServiceId }
  | {
    readonly kind: "machine";
    readonly service_id: ServiceId;
    readonly machine_id: MachineId;
  };
function isSite(value: unknown): value is Site {
  return (record(value, ["kind", "service_id"]) && value.kind === "service" &&
    (isServiceId(value.service_id))) ||
    (record(value, ["kind", "service_id", "machine_id"]) &&
      value.kind === "machine" && (isServiceId(value.service_id)) &&
      (isMachineId(value.machine_id)));
}
export type ScopeKind = {
  readonly kind: "service";
  readonly service_id: ServiceId;
} | {
  readonly kind: "machine";
  readonly parent: ScopeId;
  readonly machine_id: MachineId;
} | {
  readonly kind: "workspace";
  readonly parent: ScopeId;
  readonly workspace_id: WorkspaceId;
} | {
  readonly kind: "session";
  readonly parent: ScopeId;
  readonly session_id: SessionId;
} | {
  readonly kind: "operation";
  readonly parent: ScopeId;
  readonly operation_id: OperationId;
};
function isScopeKind(value: unknown): value is ScopeKind {
  return (record(value, ["kind", "service_id"]) && value.kind === "service" &&
    (isServiceId(value.service_id))) ||
    (record(value, ["kind", "parent", "machine_id"]) &&
      value.kind === "machine" && (isScopeId(value.parent)) &&
      (isMachineId(value.machine_id))) ||
    (record(value, ["kind", "parent", "workspace_id"]) &&
      value.kind === "workspace" && (isScopeId(value.parent)) &&
      (isWorkspaceId(value.workspace_id))) ||
    (record(value, ["kind", "parent", "session_id"]) &&
      value.kind === "session" && (isScopeId(value.parent)) &&
      (isSessionId(value.session_id))) ||
    (record(value, ["kind", "parent", "operation_id"]) &&
      value.kind === "operation" && (isScopeId(value.parent)) &&
      (isOperationId(value.operation_id)));
}
export type Scope = { readonly id: ScopeId; readonly lifetime: ScopeKind };
function isScope(value: unknown): value is Scope {
  return (record(value, ["id", "lifetime"]) && (isScopeId(value.id)) &&
    (isScopeKind(value.lifetime)));
}
export type ReleaseRef = {
  readonly plugin_id: PluginId;
  readonly version: ExactVersion;
  readonly digest: ArtifactDigest;
  readonly publisher: PublisherId;
};
function isReleaseRef(value: unknown): value is ReleaseRef {
  return (record(value, ["plugin_id", "version", "digest", "publisher"]) &&
    (isPluginId(value.plugin_id)) && (isExactVersion(value.version)) &&
    (isArtifactDigest(value.digest)) && (isPublisherId(value.publisher)));
}
export type Identity =
  | { readonly kind: "core"; readonly anchor: CoreAnchor }
  | {
    readonly kind: "plugin";
    readonly release: ReleaseRef;
    readonly generation: InstallationGeneration;
    readonly incarnation: InstanceIncarnation;
    readonly execution: Execution;
  };
function isIdentity(value: unknown): value is Identity {
  return (record(value, ["kind", "anchor"]) && value.kind === "core" &&
    (isCoreAnchor(value.anchor))) ||
    (record(value, [
      "kind",
      "release",
      "generation",
      "incarnation",
      "execution",
    ]) && value.kind === "plugin" && (isReleaseRef(value.release)) &&
      (isInstallationGeneration(value.generation)) &&
      (isInstanceIncarnation(value.incarnation)) &&
      (isExecution(value.execution)));
}
export type Ownership = { readonly kind: "scope" } | {
  readonly kind: "node";
  readonly node: NodeId;
};
function isOwnership(value: unknown): value is Ownership {
  return (record(value, ["kind"]) && value.kind === "scope") ||
    (record(value, ["kind", "node"]) && value.kind === "node" &&
      (isNodeId(value.node)));
}
export type ContractRef = {
  readonly id: ContractId;
  readonly version: ExactVersion;
  readonly fingerprint: ContractFingerprint;
};
function isContractRef(value: unknown): value is ContractRef {
  return (record(value, ["id", "version", "fingerprint"]) &&
    (isContractId(value.id)) && (isExactVersion(value.version)) &&
    (isContractFingerprint(value.fingerprint)));
}
export type ProvidedPort = {
  readonly id: PortId;
  readonly contract: ContractRef;
  readonly visibility: Visibility;
  readonly transport: Transport;
};
function isProvidedPort(value: unknown): value is ProvidedPort {
  return (record(value, ["id", "contract", "visibility", "transport"]) &&
    (isPortId(value.id)) && (isContractRef(value.contract)) &&
    (isVisibility(value.visibility)) && (isTransport(value.transport)));
}
export type RequiredPort = {
  readonly id: PortId;
  readonly contract: ContractRef;
  readonly cardinality: Cardinality;
};
function isRequiredPort(value: unknown): value is RequiredPort {
  return (record(value, ["id", "contract", "cardinality"]) &&
    (isPortId(value.id)) && (isContractRef(value.contract)) &&
    (isCardinality(value.cardinality)));
}
export type Node = {
  readonly id: NodeId;
  readonly scope: ScopeId;
  readonly site: Site;
  readonly identity: Identity;
  readonly owner: Ownership;
  readonly provides: readonly ProvidedPort[];
  readonly requires: readonly RequiredPort[];
};
function isNode(value: unknown): value is Node {
  return (record(value, [
    "id",
    "scope",
    "site",
    "identity",
    "owner",
    "provides",
    "requires",
  ]) && (isNodeId(value.id)) && (isScopeId(value.scope)) &&
    (isSite(value.site)) && (isIdentity(value.identity)) &&
    (isOwnership(value.owner)) &&
    (Array.isArray(value.provides) && value.provides.length >= 0 &&
      value.provides.length <= 32 && value.provides.every(isProvidedPort)) &&
    (Array.isArray(value.requires) && value.requires.length >= 0 &&
      value.requires.length <= 32 && value.requires.every(isRequiredPort)));
}
export type Endpoint = { readonly node: NodeId; readonly port: PortId };
function isEndpoint(value: unknown): value is Endpoint {
  return (record(value, ["node", "port"]) && (isNodeId(value.node)) &&
    (isPortId(value.port)));
}
export type Binding = {
  readonly consumer: Endpoint;
  readonly provider: Endpoint;
  readonly revision: BindingRevision;
};
function isBinding(value: unknown): value is Binding {
  return (record(value, ["consumer", "provider", "revision"]) &&
    (isEndpoint(value.consumer)) && (isEndpoint(value.provider)) &&
    (isBindingRevision(value.revision)));
}
export type Composition = {
  readonly format: FormatVersion;
  readonly scopes: readonly Scope[];
  readonly nodes: readonly Node[];
  readonly bindings: readonly Binding[];
};
function isComposition(value: unknown): value is Composition {
  return (record(value, ["format", "scopes", "nodes", "bindings"]) &&
    (isFormatVersion(value.format)) &&
    (Array.isArray(value.scopes) && value.scopes.length >= 1 &&
      value.scopes.length <= 256 && value.scopes.every(isScope)) &&
    (Array.isArray(value.nodes) && value.nodes.length >= 1 &&
      value.nodes.length <= 256 && value.nodes.every(isNode)) &&
    (Array.isArray(value.bindings) && value.bindings.length >= 0 &&
      value.bindings.length <= 2048 && value.bindings.every(isBinding)));
}

/** Decodes untrusted data, never an authorization or a verified release. */
export function decodeComposition(raw: string): Composition {
  const value: unknown = strictJson(raw, MAX_BYTES, MAX_DEPTH);
  if (!isComposition(value)) throw new Error("invalid_contract");
  return value;
}

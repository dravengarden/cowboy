/** Core-only structural linking, shared with src/composition. No Catalog,
 * policy, enrollment, state lease or execution authority is obtained here.
 * Accept text only: a decoded/serialized report cannot become a live plan.
 */
import {
  type Binding,
  type Composition,
  CONTRACT_FINGERPRINT,
  decodeComposition,
  type Node,
  type NodeId,
  type Scope,
  type ScopeId,
  type ServiceId,
  type Site,
} from "./composition.generated.ts";

export type CompositionCheckCode =
  | "invalid_json"
  | "invalid_contract"
  | "duplicate_scope"
  | "invalid_scope_tree"
  | "duplicate_node"
  | "unknown_scope"
  | "invalid_placement"
  | "service_executable"
  | "generation_conflict"
  | "invalid_owner"
  | "duplicate_port"
  | "port_budget"
  | "invalid_endpoint"
  | "duplicate_binding"
  | "contract_mismatch"
  | "scope_not_visible"
  | "local_port_crosses_site"
  | "cardinality_mismatch"
  | "dependency_cycle";

/** Closed errors never carry input text, identifiers or native exceptions. */
export class CompositionCheckError extends Error {
  constructor(readonly code: CompositionCheckCode) {
    super(code);
    this.name = "CompositionCheckError";
  }
}

const checked = Symbol("cowboy.checked-structure");
export type CheckedStructure = {
  readonly [checked]: true;
  readonly status: "structurally_valid";
  readonly authorized: false;
  readonly contract_fingerprint: typeof CONTRACT_FINGERPRINT;
  readonly proposal_digest: string;
  readonly dependency_order: readonly NodeId[];
  readonly reverse_dependency_order: readonly NodeId[];
  readonly sites: readonly {
    readonly site: Site;
    readonly nodes: readonly NodeId[];
  }[];
  readonly remote_bindings: readonly Binding[];
};

function fail(code: CompositionCheckCode): never {
  throw new CompositionCheckError(code);
}

// The wire profile permits ASCII identities only. Never use locale collation:
// Rust's string Ord and JSON canonicalization use exact code-point ordering.
function compare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${
      Object.entries(value).sort(([a], [b]) => compare(a, b)).map(
        ([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`,
      ).join(",")
    }}`;
  }
  return JSON.stringify(value) ?? fail("invalid_contract");
}

function compareBindings(a: Binding, b: Binding): number {
  return compare(a.consumer.node, b.consumer.node) ||
    compare(a.consumer.port, b.consumer.port) ||
    compare(a.provider.node, b.provider.node) ||
    compare(a.provider.port, b.provider.port) ||
    compare(a.revision, b.revision);
}

function compareSites(a: Site, b: Site): number {
  if (a.kind !== b.kind) return a.kind === "service" ? -1 : 1;
  return compare(a.service_id, b.service_id) ||
    (a.kind === "machine" && b.kind === "machine"
      ? compare(a.machine_id, b.machine_id)
      : 0);
}

function projectSite(site: Site): Site {
  return site.kind === "service"
    ? { kind: "service", service_id: site.service_id }
    : {
      kind: "machine",
      service_id: site.service_id,
      machine_id: site.machine_id,
    };
}

function projectBinding(binding: Binding): Binding {
  return {
    consumer: { node: binding.consumer.node, port: binding.consumer.port },
    provider: { node: binding.provider.node, port: binding.provider.port },
    revision: binding.revision,
  };
}

function ancestry(id: ScopeId, scopes: ReadonlyMap<ScopeId, Scope>): Scope[] {
  const result: Scope[] = [];
  let next: ScopeId | undefined = id;
  while (next !== undefined) {
    const scope: Scope = scopes.get(next) ?? fail("unknown_scope");
    if (result.includes(scope) || result.length >= 32) {
      fail("invalid_scope_tree");
    }
    result.push(scope);
    next = scope.lifetime.kind === "service"
      ? undefined
      : scope.lifetime.parent;
  }
  return result;
}

function validateScopes(scopes: ReadonlyMap<ScopeId, Scope>): ServiceId {
  const lifetimes = new Set<string>();
  let service: ServiceId | undefined;
  for (const scope of scopes.values()) {
    const identity = canonical(scope.lifetime);
    if (lifetimes.has(identity)) fail("duplicate_scope");
    lifetimes.add(identity);
    const chain = ancestry(scope.id, scopes);
    const kind = scope.lifetime.kind;
    if (kind === "service") {
      if (service !== undefined) fail("invalid_scope_tree");
      service = scope.lifetime.service_id;
    } else {
      const owner = chain[1]?.lifetime.kind;
      const valid = kind === "machine" && owner === "service" ||
        kind === "workspace" && owner === "machine" ||
        kind === "session" && owner === "workspace" ||
        kind === "operation" && owner !== undefined && owner !== "operation";
      if (!valid) fail("invalid_scope_tree");
    }
  }
  return service ?? fail("invalid_scope_tree");
}

function freeze<T>(value: T): T {
  if (value !== null && typeof value === "object") {
    for (const nested of Object.values(value)) freeze(nested);
    Object.freeze(value);
  }
  return value;
}

/** A deeply frozen diagnostic, never a VerifiedRelease/AuthorizedPlan. */
export async function checkComposition(raw: string): Promise<CheckedStructure> {
  let decoded: Composition;
  try {
    decoded = decodeComposition(raw);
  } catch (error) {
    return fail(
      error instanceof Error && error.message === "invalid_json"
        ? "invalid_json"
        : "invalid_contract",
    );
  }
  // Copy readonly arrays; no input object can change while SHA-256 is pending.
  const proposal: Composition = {
    ...decoded,
    scopes: [...decoded.scopes].sort((a, b) => compare(a.id, b.id)),
    nodes: decoded.nodes.map((node) => ({
      ...node,
      provides: [...node.provides].sort((a, b) => compare(a.id, b.id)),
      requires: [...node.requires].sort((a, b) => compare(a.id, b.id)),
    })).sort((a, b) => compare(a.id, b.id)),
    bindings: [...decoded.bindings].sort(compareBindings),
  };
  const scopes = new Map(proposal.scopes.map((scope) => [scope.id, scope]));
  if (scopes.size !== proposal.scopes.length) fail("duplicate_scope");
  const service = validateScopes(scopes);
  const nodes = new Map(proposal.nodes.map((node) => [node.id, node]));
  if (nodes.size !== proposal.nodes.length) fail("duplicate_node");
  const dependencies = new Map<NodeId, Set<NodeId>>();
  const generations = new Map<string, string>();
  let ports = 0;
  for (const node of nodes.values()) {
    const chain = ancestry(node.scope, scopes);
    if (node.site.service_id !== service) fail("invalid_placement");
    const machine = chain.find((scope) => scope.lifetime.kind === "machine");
    if (
      machine?.lifetime.kind === "machine" &&
      (node.site.kind !== "machine" ||
        node.site.machine_id !== machine.lifetime.machine_id)
    ) fail("invalid_placement");
    if (node.identity.kind === "plugin") {
      const { release, generation, execution } = node.identity;
      if (execution === "isolated" && node.site.kind === "service") {
        fail("service_executable");
      }
      const slot = canonical([node.site, release.plugin_id, generation]);
      const exact = canonical(release);
      const previous = generations.get(slot);
      if (previous !== undefined && previous !== exact) {
        fail("generation_conflict");
      }
      generations.set(slot, exact);
    }
    const prerequisites = new Set<NodeId>();
    dependencies.set(node.id, prerequisites);
    if (node.owner.kind === "node") {
      const owner = nodes.get(node.owner.node) ?? fail("invalid_owner");
      if (
        owner.id === node.id || compareSites(owner.site, node.site) !== 0 ||
        !chain.some((scope) => scope.id === owner.scope)
      ) fail("invalid_owner");
      prerequisites.add(owner.id);
    }
    if (
      new Set(node.provides.map((port) => port.id)).size !==
        node.provides.length ||
      new Set(node.requires.map((port) => port.id)).size !==
        node.requires.length
    ) fail("duplicate_port");
    ports += node.provides.length + node.requires.length;
    if (ports > 2048) fail("port_budget");
  }
  const seen = new Set<string>();
  const cardinalities = new Map<string, number>();
  const remoteBindings: Binding[] = [];
  const endpointNode = (id: NodeId): Node =>
    nodes.get(id) ?? fail("invalid_endpoint");
  for (const binding of proposal.bindings) {
    const edge = canonical([binding.consumer, binding.provider]);
    if (seen.has(edge)) fail("duplicate_binding");
    seen.add(edge);
    const consumer = endpointNode(binding.consumer.node);
    const provider = endpointNode(binding.provider.node);
    const required = consumer.requires.find((p) =>
      p.id === binding.consumer.port
    ) ??
      fail("invalid_endpoint");
    const provided =
      provider.provides.find((p) => p.id === binding.provider.port) ??
        fail("invalid_endpoint");
    if (canonical(required.contract) !== canonical(provided.contract)) {
      fail("contract_mismatch");
    }
    const visible = consumer.scope === provider.scope ||
      provided.visibility === "descendants" &&
        ancestry(consumer.scope, scopes).some((scope) =>
          scope.id === provider.scope
        );
    if (!visible) fail("scope_not_visible");
    if (compareSites(consumer.site, provider.site) !== 0) {
      if (provided.transport === "local") fail("local_port_crosses_site");
      remoteBindings.push(projectBinding(binding));
    }
    const port = canonical(binding.consumer);
    cardinalities.set(port, (cardinalities.get(port) ?? 0) + 1);
    dependencies.get(consumer.id)!.add(provider.id);
  }
  for (const node of nodes.values()) {
    for (const required of node.requires) {
      const count =
        cardinalities.get(canonical({ node: node.id, port: required.id })) ?? 0;
      if (
        required.cardinality === "one" && count !== 1 ||
        required.cardinality === "optional" && count > 1
      ) fail("cardinality_mismatch");
    }
  }
  const dependencyOrder: NodeId[] = [];
  while (dependencies.size) {
    // Map insertion order is sorted NodeId order, including after deletions.
    const ready = [...dependencies].find(([, deps]) => deps.size === 0)?.[0] ??
      fail("dependency_cycle");
    dependencies.delete(ready);
    for (const prerequisites of dependencies.values()) {
      prerequisites.delete(ready);
    }
    dependencyOrder.push(ready);
  }
  const sites = new Map<string, { site: Site; nodes: NodeId[] }>();
  for (const id of dependencyOrder) {
    const site = projectSite(nodes.get(id)!.site);
    const key = canonical(site);
    const projection = sites.get(key) ?? { site, nodes: [] };
    projection.nodes.push(id);
    sites.set(key, projection);
  }
  const bytes = new TextEncoder().encode(
    `cowboy.composition.proposal.v1\n${CONTRACT_FINGERPRINT}\n${
      canonical(proposal)
    }`,
  );
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  // The sole branding boundary is after full decoding, linking and digesting.
  // JSON.parse never creates this nominal type or any execution authority.
  return freeze<CheckedStructure>({
    [checked]: true,
    status: "structurally_valid",
    authorized: false,
    contract_fingerprint: CONTRACT_FINGERPRINT,
    proposal_digest: `sha256:${
      Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("")
    }`,
    dependency_order: dependencyOrder,
    reverse_dependency_order: [...dependencyOrder].reverse(),
    sites: [...sites.values()].sort((a, b) => compareSites(a.site, b.site)),
    remote_bindings: remoteBindings,
  });
}

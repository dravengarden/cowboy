// Pure release-policy checks. The registry is build metadata, NOT execution
// authority or a replacement for signed Plugin packages and runtime bindings.
export interface ComponentRecord {
  id: string;
  version: string;
  publisher: string;
  sources: string[];
  digest: string;
  package?: { kind: "cargo" | "npm"; name: string; manifest: string };
}

export interface ComponentPin {
  id: string;
  version: string;
}

export interface PluginSourceSnapshot {
  component_release: string;
  components: ComponentPin[];
  source_digest: string;
}

export interface ComponentClosure {
  // All declared internal package edges are conservatively release-causing,
  // including peer/build/contract dependencies. No implicit "test-only" edges.
  component_dependencies: Record<string, string[]>;
  plugins: Record<string, PluginSourceSnapshot>;
}

export interface ComponentRelease {
  version: string;
  components: ComponentRecord[];
  plugins: Record<string, string>;
  // Absent only in immutable schema-2 history (the coordinated release train).
  closure?: ComponentClosure;
  // Explicit schema-3 additive migration. Never authorizes removal, identity
  // reuse, hidden dependencies or relabeling an existing Plugin's closure.
  component_additions?: string[];
}

export function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

export function exactVersion(value: string): boolean {
  return /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(value);
}

export function compareVersion(left: string, right: string): number {
  assert(exactVersion(left) && exactVersion(right), "invalid SemVer");
  const a = left.split(".").map(BigInt);
  const b = right.split(".").map(BigInt);
  for (let index = 0; index < 3; index++) {
    if (a[index]! !== b[index]!) return a[index]! > b[index]! ? 1 : -1;
  }
  return 0;
}

export function same(left: unknown, right: unknown): boolean {
  return JSON.stringify(canonical(left)) === JSON.stringify(canonical(right));
}

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map((
        [key, v],
      ) => [key, canonical(v)]),
    );
  }
  return value;
}

export function assertSameSet(
  left: string[],
  right: string[],
  label: string,
): void {
  assert(
    new Set(left).size === left.length && new Set(right).size === right.length,
    `${label}: duplicate identity`,
  );
  assert(
    same([...left].sort(), [...right].sort()),
    `${label}: identity set mismatch`,
  );
}

export function dependencyClosure(
  roots: string[],
  graph: Record<string, string[]>,
): string[] {
  const visited = new Set<string>();
  const visiting = new Set<string>();
  const visit = (id: string): void => {
    assert(!visiting.has(id), `${id}: component dependency cycle`);
    if (visited.has(id)) return;
    assert(
      Object.hasOwn(graph, id),
      `${id}: missing component dependency node`,
    );
    visiting.add(id);
    const edges = graph[id]!;
    assert(
      new Set(edges).size === edges.length,
      `${id}: duplicate component dependency`,
    );
    for (const edge of edges) visit(edge);
    visiting.delete(id);
    visited.add(id);
  };
  for (const root of roots) visit(root);
  return [...visited].sort();
}

// A historical component_release is usable only if EVERY exact input in its
// transitive closure still has the same bytes and package identity today.
export function validatePluginComponentClosure(
  releases: ComponentRelease[],
  pluginId: string,
  snapshot: Pick<PluginSourceSnapshot, "component_release" | "components">,
): void {
  const active = releases.at(-1)!;
  const pinned = releases.find((release) =>
    release.version === snapshot.component_release
  );
  assert(pinned !== undefined, `${pluginId}: unknown component release`);
  if (!active.closure) {
    assert(pinned === active, `${pluginId}: component release mismatch`);
  }
  const pins = new Map(snapshot.components.map((pin) => [pin.id, pin.version]));
  assert(
    pins.size === snapshot.components.length,
    `${pluginId}: duplicate component dependency`,
  );
  const activeById = new Map(
    active.components.map((component) => [component.id, component]),
  );
  const pinnedById = new Map(
    pinned.components.map((component) => [component.id, component]),
  );
  for (const [id, version] of pins) {
    assert(exactVersion(version), `${pluginId}: invalid component pin`);
    assert(
      pinnedById.get(id)?.version === version,
      `${pluginId}: component pin differs from declared release`,
    );
  }
  const closure = active.closure
    ? dependencyClosure([...pins.keys()], active.closure.component_dependencies)
    : [...pins.keys()];
  for (const id of closure) {
    assert(
      activeById.has(id) && pinnedById.has(id),
      `${pluginId}: unknown component ${id}`,
    );
    assert(
      same(activeById.get(id), pinnedById.get(id)),
      `${pluginId}: changed component closure: ${id}`,
    );
  }
}

export function validateReleaseHistory(releases: ComponentRelease[]): void {
  for (let index = 0; index < releases.length; index++) {
    const release = releases[index]!;
    assert(
      exactVersion(release.version),
      `${release.version}: invalid component release version`,
    );
    assertSameSet(
      release.components.map((c) => c.id),
      release.components.map((c) => c.id),
      `${release.version}: component`,
    );
    for (const [id, version] of Object.entries(release.plugins)) {
      assert(/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(id), `${id}: invalid plugin id`);
      assert(exactVersion(version), `${id}: invalid plugin version ${version}`);
    }
    const previous = releases[index - 1];
    if (release.component_additions !== undefined) {
      assert(
        previous?.closure && release.closure &&
          Array.isArray(release.component_additions) &&
          release.component_additions.length > 0 &&
          release.component_additions.every((id) =>
            typeof id === "string" &&
            /^cowboy\.[a-z0-9]+(?:-[a-z0-9]+)*$/.test(id)
          ),
        "component additions require an explicit nonempty post-baseline migration",
      );
    }
    if (previous) {
      assert(
        compareVersion(release.version, previous.version) > 0,
        `${release.version}: component releases must append in SemVer order`,
      );
      assertSameSet(
        Object.keys(release.plugins),
        Object.keys(previous.plugins),
        `${release.version}: plugin set changed; add/remove requires a new plugin-contract schema`,
      );
    }
    if (!release.closure) {
      assert(
        !previous?.closure,
        "cannot return to coordinated policy after closure migration",
      );
      if (previous) {
        for (const id of Object.keys(release.plugins)) {
          assert(
            compareVersion(release.plugins[id]!, previous.plugins[id]!) > 0,
            `${release.version}: ${id} must increase version with the component release`,
          );
        }
      }
      continue;
    }
    const graph = release.closure.component_dependencies;
    assertSameSet(
      Object.keys(graph),
      release.components.map((c) => c.id),
      "component closure nodes",
    );
    dependencyClosure(Object.keys(graph), graph);
    assertSameSet(
      Object.keys(release.closure.plugins),
      Object.keys(release.plugins),
      "Plugin closure nodes",
    );
    for (const component of release.components) {
      assert(
        exactVersion(component.version),
        `${component.id}: invalid component version`,
      );
      assert(
        /^sha256:[0-9a-f]{64}$/.test(component.digest),
        `${component.id}: invalid source digest`,
      );
    }
    if (previous && !previous.closure) {
      // A separate no-change baseline makes the policy cutover auditable; it
      // cannot hide a component upgrade under the legacy minimum versions.
      assert(
        same(release.components, previous.components),
        "closure migration baseline must not change components",
      );
    }
    if (previous?.closure) {
      assertSameSet(
        release.components.map((c) => c.id),
        [
          ...previous.components.map((c) => c.id),
          ...(release.component_additions ?? []),
        ],
        "component identities (addition/removal needs an explicit migration)",
      );
      for (const component of release.components) {
        const old = previous.components.find((c) => c.id === component.id);
        if (!old) continue; // Exact declared additions were checked above.
        const changed = !same(component, old) ||
          !same(
            graph[component.id],
            previous.closure.component_dependencies[component.id],
          );
        const inputsChanged = dependencyClosure(graph[component.id]!, graph)
          .some((id) =>
            !same(
              release.components.find((candidate) => candidate.id === id),
              previous.components.find((candidate) => candidate.id === id),
            )
          );
        assert(
          compareVersion(component.version, old.version) >= 0,
          `${component.id}: component version regressed`,
        );
        if (changed || inputsChanged) {
          assert(
            compareVersion(component.version, old.version) > 0,
            `${component.id}: changed component must increase version`,
          );
        }
      }
    }
    for (const [id, snapshot] of Object.entries(release.closure.plugins)) {
      assert(
        /^sha256:[0-9a-f]{64}$/.test(snapshot.source_digest),
        `${id}: invalid Plugin source digest`,
      );
      validatePluginComponentClosure(
        releases.slice(0, index + 1),
        id,
        snapshot,
      );
      if (!previous) continue;
      assert(
        compareVersion(release.plugins[id]!, previous.plugins[id]!) >= 0,
        `${id}: Plugin version regressed`,
      );
      const old = previous.closure?.plugins[id];
      if (old && !same(snapshot, old)) {
        assert(
          compareVersion(release.plugins[id]!, previous.plugins[id]!) > 0,
          `${id}: changed Plugin source or binding must increase version`,
        );
      }
    }
  }
}

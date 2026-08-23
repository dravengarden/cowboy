interface ComponentRecord {
  id: string;
  version: string;
  publisher: string;
  sources: string[];
  digest: string;
}

interface ComponentRelease {
  version: string;
  components: ComponentRecord[];
  plugins: Record<string, string>;
}

interface ComponentRegistry {
  schema_version: number;
  active_release: string;
  releases: ComponentRelease[];
}

interface PluginManifest {
  schema_version: number;
  id: string;
  version: string;
  publisher: string;
  kind: "agent_provider" | "code_intelligence";
  entrypoint: string;
  components: Array<{ id: string; version: string }>;
}

const semverPattern = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
const registry = await readJson<ComponentRegistry>("components/registry.json");
assert(registry.schema_version === 1, "unsupported component registry schema");
assert(registry.releases.length > 0, "component registry has no releases");

const active = registry.releases.at(-1)!;
assert(
  active.version === registry.active_release,
  "active component release must be the last immutable release",
);

validateReleaseHistory(registry.releases);

export function validateReleaseHistory(releases: ComponentRelease[]): void {
  for (let index = 0; index < releases.length; index += 1) {
    const release = releases[index]!;
    assert(
      exactVersion(release.version),
      `${release.version}: invalid component release version`,
    );
    assertUnique(
      release.components.map((component) => component.id),
      `${release.version}: component`,
    );
    for (const [pluginId, version] of Object.entries(release.plugins)) {
      assert(validId(pluginId), `${pluginId}: invalid plugin id`);
      assert(
        exactVersion(version),
        `${pluginId}: invalid plugin version ${version}`,
      );
    }
    const previous = releases[index - 1];
    if (!previous) continue;
    assert(
      compareVersion(release.version, previous.version) > 0,
      `${release.version}: component releases must append in SemVer order`,
    );
    assertSameSet(
      Object.keys(release.plugins),
      Object.keys(previous.plugins),
      `${release.version}: plugin set changed; add/remove requires a new plugin-contract schema`,
    );
    for (const pluginId of Object.keys(release.plugins)) {
      assert(
        compareVersion(
          release.plugins[pluginId]!,
          previous.plugins[pluginId]!,
        ) > 0,
        `${release.version}: ${pluginId} must increase version with the component release`,
      );
    }
  }
}

const activeComponents = new Map(
  active.components.map((component) => [component.id, component]),
);
for (const component of active.components) {
  assert(
    validComponentId(component.id),
    `${component.id}: invalid component id`,
  );
  assert(
    exactVersion(component.version),
    `${component.id}: invalid component version`,
  );
  assert(
    /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(component.publisher),
    `${component.id}: invalid component publisher`,
  );
  assert(component.sources.length > 0, `${component.id}: no source roots`);
  const digest = await sourceDigest(component.sources);
  if (Deno.args.includes("--print-digests")) {
    console.log(`${component.id} ${digest}`);
  } else {
    assert(
      digest === component.digest,
      `${component.id}: source digest changed (${digest}); append a component release and bump every plugin version`,
    );
  }
}

const pluginEntries = [...Deno.readDirSync("plugins")]
  .filter((entry) =>
    entry.isDirectory && exists(`plugins/${entry.name}/plugin.json`)
  )
  .map((entry) => entry.name)
  .sort();
assertSameSet(
  pluginEntries,
  Object.keys(active.plugins),
  "active plugin registry",
);

for (const pluginId of pluginEntries) {
  const manifest = await readJson<PluginManifest>(
    `plugins/${pluginId}/plugin.json`,
  );
  assert(
    manifest.schema_version === 1,
    `${pluginId}: unsupported plugin schema`,
  );
  assert(manifest.id === pluginId, `${pluginId}: directory identity mismatch`);
  assert(manifest.publisher.length > 0, `${pluginId}: publisher is empty`);
  assert(
    manifest.version === active.plugins[pluginId],
    `${pluginId}: registry version mismatch`,
  );
  assert(
    exists(`plugins/${pluginId}/${manifest.entrypoint}`),
    `${pluginId}: missing entrypoint`,
  );
  assertUnique(
    manifest.components.map((component) => component.id),
    `${pluginId}: component dependency`,
  );

  const dependencies = new Map(
    manifest.components.map((component) => [component.id, component.version]),
  );
  assert(
    dependencies.has("cowboy.plugin-contract"),
    `${pluginId}: missing plugin contract`,
  );
  assert(
    dependencies.has("cowboy.plugin-sdk"),
    `${pluginId}: missing plugin SDK`,
  );
  for (const [componentId, version] of dependencies) {
    const component = activeComponents.get(componentId);
    assert(
      component !== undefined,
      `${pluginId}: unknown component ${componentId}`,
    );
    assert(
      component.version === version,
      `${pluginId}: stale ${componentId}@${version}`,
    );
  }

  if (manifest.kind === "agent_provider") {
    for (
      const componentId of [
        "cowboy.provider-sdk",
        "cowboy.provider-ui",
        "cowboy.provider-runtime",
      ]
    ) {
      assert(
        dependencies.has(componentId),
        `${pluginId}: missing ${componentId}`,
      );
    }
    const provider = await readJson<{ id: string; version: string }>(
      `plugins/${pluginId}/${manifest.entrypoint}`,
    );
    assert(
      provider.id === pluginId,
      `${pluginId}: Provider payload identity mismatch`,
    );
    assert(
      provider.version === manifest.version,
      `${pluginId}: Provider payload version mismatch`,
    );
  } else {
    assert(
      pluginId === "zed",
      `${pluginId}: unsupported code-intelligence plugin`,
    );
    assert(
      dependencies.has("cowboy.code-intelligence"),
      `${pluginId}: missing code contract`,
    );
    const contract = await readJson<{ id: string; version: string }>(
      `plugins/${pluginId}/${manifest.entrypoint}`,
    );
    assert(contract.id === pluginId, `${pluginId}: contract identity mismatch`);
    assert(
      contract.version === manifest.version,
      `${pluginId}: contract version mismatch`,
    );
    const cargo = await Deno.readTextFile(`plugins/${pluginId}/adapter/Cargo.toml`);
    const packageBlock = cargo.split("[dependencies]", 1)[0] ?? cargo;
    assert(
      packageBlock.includes(`version = "${manifest.version}"`),
      `${pluginId}: adapter package version mismatch`,
    );
  }
}

if (!Deno.args.includes("--print-digests")) {
  console.log(
    `plugin/component graph valid: ${active.components.length} components, ${pluginEntries.length} plugins, release ${active.version}`,
  );
}

async function sourceDigest(sources: string[]): Promise<string> {
  const files: string[] = [];
  for (const source of sources) await collectFiles(source, files);
  files.sort();
  const chunks: Uint8Array[] = [];
  let length = 0;
  for (const file of files) {
    const path = new TextEncoder().encode(`${file}\0`);
    const body = await Deno.readFile(file);
    const end = new Uint8Array([0]);
    chunks.push(path, body, end);
    length += path.length + body.length + end.length;
  }
  const input = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    input.set(chunk, offset);
    offset += chunk.length;
  }
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", input));
  return `sha256:${
    [...hash].map((byte) => byte.toString(16).padStart(2, "0")).join("")
  }`;
}

async function collectFiles(path: string, files: string[]): Promise<void> {
  const stat = await Deno.stat(path);
  if (stat.isFile) {
    files.push(path);
    return;
  }
  assert(
    stat.isDirectory,
    `${path}: component source must be a file or directory`,
  );
  for await (const entry of Deno.readDir(path)) {
    await collectFiles(`${path}/${entry.name}`, files);
  }
}

function compareVersion(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  for (let index = 0; index < 3; index += 1) {
    if (a[index]! !== b[index]!) return a[index]! - b[index]!;
  }
  return 0;
}

function parseVersion(value: string): number[] {
  const match = semverPattern.exec(value);
  assert(match !== null, `${value}: invalid SemVer`);
  return match.slice(1).map(Number);
}

function exactVersion(value: string): boolean {
  return semverPattern.test(value);
}

function validId(value: string): boolean {
  return /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value);
}

function validComponentId(value: string): boolean {
  return /^cowboy\.[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value);
}

function assertUnique(values: string[], label: string): void {
  assert(
    new Set(values).size === values.length,
    `${label}: duplicate identity`,
  );
}

function assertSameSet(left: string[], right: string[], label: string): void {
  const a = [...left].sort();
  const b = [...right].sort();
  assert(
    JSON.stringify(a) === JSON.stringify(b),
    `${label}: identity set mismatch`,
  );
}

function exists(path: string): boolean {
  try {
    return Deno.statSync(path).isFile || Deno.statSync(path).isDirectory;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return false;
    throw error;
  }
}

async function readJson<T>(path: string): Promise<T> {
  return JSON.parse(await Deno.readTextFile(path)) as T;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

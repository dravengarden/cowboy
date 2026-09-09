import {
  assert,
  assertSameSet,
  compareVersion,
  type ComponentClosure,
  type ComponentRecord,
  type ComponentRelease,
  exactVersion,
  same,
  validatePluginComponentClosure,
  validateReleaseHistory,
} from "./plugin-component-closure.ts";
export { validateReleaseHistory } from "./plugin-component-closure.ts";

interface ComponentRegistry {
  schema_version: number;
  active_release: string;
  releases: ComponentRelease[];
}

interface PluginManifest {
  schema_version: number;
  id: string;
  version: string;
  component_release: string;
  publisher: string;
  kind:
    | "agent_provider"
    | "authentication_provider"
    | "code_intelligence"
    | "telemetry_backend";
  entrypoint: string;
  components: Array<{ id: string; version: string }>;
}

export async function checkRepository(): Promise<void> {
  const registry = await readJson<ComponentRegistry>(
    "components/registry.json",
  );
  assert(
    registry.schema_version === 2 || registry.schema_version === 3,
    "unsupported component registry schema",
  );
  assert(registry.releases.length > 0, "component registry has no releases");

  const active = registry.releases.at(-1)!;
  if (Deno.args.includes("--print-closure")) {
    console.log(
      JSON.stringify(
        await repositoryClosure(
          active.components,
          [...Deno.readDirSync("plugins")].filter((entry) =>
            entry.isDirectory && exists(`plugins/${entry.name}/plugin.json`)
          ).map((entry) => entry.name).sort(),
        ),
        null,
        2,
      ),
    );
    return;
  }
  if (Deno.args.includes("--print-digests")) {
    for (const component of active.components) {
      console.log(`${component.id} ${await sourceDigest(component.sources)}`);
    }
    return;
  }
  assert(
    active.version === registry.active_release,
    "active component release must be the last immutable release",
  );
  assert(
    (registry.schema_version === 3) === (active.closure !== undefined),
    "registry schema and closure policy disagree",
  );

  validateReleaseHistory(registry.releases);

  const activeComponents = new Map(
    active.components.map((component) => [component.id, component]),
  );
  const distributableManifests = [...Deno.readDirSync("components")]
    .filter((entry) => entry.isDirectory)
    .flatMap((entry) =>
      ["package.json", "Cargo.toml"]
        .map((manifest) => `components/${entry.name}/${manifest}`)
        .filter(exists)
    )
    .sort();
  assertSameSet(
    active.components.flatMap((component) =>
      component.package ? [component.package.manifest] : []
    ),
    distributableManifests,
    "active component package registry",
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
    assert(
      component.package !== undefined,
      `${component.id}: no distributable package`,
    );
    await validatePackage(component);
    const digest = await sourceDigest(component.sources);
    assert(
      digest === component.digest,
      `${component.id}: source digest changed (${digest}); append a component release and version its affected closure`,
    );
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
    assert(
      manifest.id === pluginId,
      `${pluginId}: directory identity mismatch`,
    );
    assert(manifest.publisher.length > 0, `${pluginId}: publisher is empty`);
    validateIndependentPluginVersion(
      pluginId,
      manifest.version,
      active.plugins[pluginId]!,
    );
    validatePluginComponentClosure(registry.releases, pluginId, manifest);
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
    } else if (manifest.kind === "code_intelligence") {
      assert(
        dependencies.has("cowboy.code-intelligence"),
        `${pluginId}: missing code contract`,
      );
      const contract = await readJson<{ id: string; version: string }>(
        `plugins/${pluginId}/${manifest.entrypoint}`,
      );
      assert(
        contract.id === pluginId,
        `${pluginId}: contract identity mismatch`,
      );
      assert(
        contract.version === manifest.version,
        `${pluginId}: contract version mismatch`,
      );
      // A Rust adapter is an optional private implementation, not a named
      // Plugin identity or the only language an external engine can use.
      const cargoPath = `plugins/${pluginId}/adapter/Cargo.toml`;
      const cargo = await Deno.readTextFile(cargoPath).catch((error) => {
        if (error instanceof Deno.errors.NotFound) return undefined;
        throw error;
      });
      if (cargo !== undefined) {
        const packageBlock = cargo.split("[dependencies]", 1)[0] ?? cargo;
        assert(
          packageBlock.includes(`version = "${manifest.version}"`),
          `${pluginId}: adapter package version mismatch`,
        );
      }
    } else {
      const contract = await readJson<{ id: string; version: string }>(
        `plugins/${pluginId}/${manifest.entrypoint}`,
      );
      assert(
        contract.id === pluginId,
        `${pluginId}: Authentication Provider payload identity mismatch`,
      );
      assert(
        contract.version === manifest.version,
        `${pluginId}: Authentication Provider payload version mismatch`,
      );
    }
  }

  const closure = await repositoryClosure(active.components, pluginEntries);
  if (active.closure) {
    assert(
      same(
        closure.component_dependencies,
        active.closure.component_dependencies,
      ),
      "component dependency snapshot changed; append a component release",
    );
    for (const id of pluginEntries) {
      const manifest = await readJson<PluginManifest>(
        `plugins/${id}/plugin.json`,
      );
      if (manifest.version === active.plugins[id]) {
        assert(
          same(closure.plugins[id], active.closure.plugins[id]),
          `${id}: unchanged Plugin version has changed source or binding`,
        );
      }
    }
  }

  console.log(
    `plugin/component graph valid: ${active.components.length} components, ${pluginEntries.length} plugins, release ${active.version}`,
  );
}

if (import.meta.main) await checkRepository();

export async function sourceDigest(sources: string[]): Promise<string> {
  const files: string[] = [];
  for (const source of sources) await collectFiles(source, files);
  return filesDigest(files);
}

export async function filesDigest(sourceFiles: string[]): Promise<string> {
  const files = [...sourceFiles].sort();
  const chunks: Uint8Array[] = [];
  let length = 0;
  for (const file of files) {
    assert(
      (await Deno.lstat(file)).isFile,
      `${file}: release input must be a regular file`,
    );
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

// Match clean Git/Nix source, not an adapter's ignored target/ tree. Untracked
// non-ignored source is included so a pre-commit check observes new input too.
// The isolation gate copies this same list and compares SDK package bytes to
// the in-tree build, catching ignored files used as package/host inputs.
export async function repositorySourceFiles(
  root: string,
  cwd?: string,
): Promise<string[]> {
  const result = await new Deno.Command("git", {
    args: [
      "ls-files",
      "--cached",
      "--others",
      "--exclude-standard",
      "-z",
      "--",
      root,
    ],
    ...(cwd === undefined ? {} : { cwd }),
    stdout: "piped",
    stderr: "piped",
  }).output();
  assert(result.success, "cannot enumerate Git release sources");
  const files = new TextDecoder("utf-8", { fatal: true }).decode(result.stdout)
    .split("\0").filter(Boolean);
  assert(files.length > 0, `${root}: no release sources`);
  assertUnique(files, `${root}: release source`);
  for (const file of files) {
    assert(confined(root, file), `${root}: source escaped Plugin directory`);
  }
  return files.map((file) => cwd === undefined ? file : resolve(cwd, file));
}

export function validateIndependentPluginVersion(
  pluginId: string,
  version: string,
  componentReleaseMinimum: string,
): void {
  assert(
    exactVersion(version),
    `${pluginId}: invalid plugin version ${version}`,
  );
  assert(
    compareVersion(version, componentReleaseMinimum) >= 0,
    `${pluginId}: version predates the active component release`,
  );
}

async function validatePackage(component: ComponentRecord): Promise<void> {
  const descriptor = component.package!;
  assertSameSet(
    component.sources,
    [dirname(descriptor.manifest)],
    `${component.id}: source digest must cover its complete package directory`,
  );
  assert(
    exists(descriptor.manifest),
    `${component.id}: package manifest is missing`,
  );
  if (descriptor.kind === "npm") {
    const manifest = await readJson<{
      name?: string;
      version?: string;
      private?: boolean;
      exports?: unknown;
      dependencies?: Record<string, string>;
      peerDependencies?: Record<string, string>;
    }>(descriptor.manifest);
    assert(
      manifest.name === descriptor.name,
      `${component.id}: npm package name mismatch`,
    );
    assert(
      manifest.version === component.version,
      `${component.id}: npm package version mismatch`,
    );
    assert(
      manifest.private !== true,
      `${component.id}: npm package is private`,
    );
    assert(
      manifest.exports !== undefined,
      `${component.id}: npm package has no public exports`,
    );
    const packageRoot = dirname(descriptor.manifest);
    for (const target of exportTargets(manifest.exports)) {
      assert(
        target.startsWith("./"),
        `${component.id}: export is not package-relative`,
      );
      const exportedPath = resolve(packageRoot, target);
      assert(
        confined(packageRoot, exportedPath) && exists(exportedPath),
        `${component.id}: export escapes or is missing: ${target}`,
      );
    }
    await validateNpmSourceClosure(
      component,
      packageRoot,
      { ...manifest.dependencies, ...manifest.peerDependencies },
    );
    return;
  }
  const cargo = await Deno.readTextFile(descriptor.manifest);
  const packageBlock = cargo.split("[dependencies]", 1)[0] ?? cargo;
  assert(
    packageBlock.includes(`name = "${descriptor.name}"`),
    `${component.id}: Cargo package name mismatch`,
  );
  assert(
    packageBlock.includes(`version = "${component.version}"`),
    `${component.id}: Cargo package version mismatch`,
  );
  assert(
    !packageBlock.includes("publish = false"),
    `${component.id}: Cargo package is private`,
  );
}

// Resolve Cargo's own metadata rather than guessing TOML with a regex. This is
// offline/no-deps: it neither builds nor downloads nor mutates a lockfile.
async function cargoPackageDependencies(): Promise<
  Map<string, Array<{ name: string; req: string; path?: string }>>
> {
  const result = await new Deno.Command("cargo", {
    args: [
      "metadata",
      "--offline",
      "--locked",
      "--no-deps",
      "--format-version",
      "1",
    ],
    stdout: "piped",
    stderr: "piped",
  }).output();
  assert(
    result.success,
    `Cargo metadata failed: ${new TextDecoder().decode(result.stderr)}`,
  );
  const metadata = JSON.parse(new TextDecoder().decode(result.stdout)) as {
    packages: Array<
      {
        name: string;
        dependencies: Array<{ name: string; req: string; path?: string }>;
      }
    >;
  };
  return new Map(metadata.packages.map((pkg) => [pkg.name, pkg.dependencies]));
}

export function resolvePackagePins(
  owner: string,
  dependencies: Record<string, string>,
  components: ComponentRecord[],
): string[] {
  const result: string[] = [];
  for (const [name, version] of Object.entries(dependencies)) {
    if (!name.startsWith("@cowboy/") && !name.startsWith("cowboy-")) continue;
    const target = components.find((component) =>
      component.package?.name === name
    );
    assert(
      target !== undefined,
      `${owner}: unregistered Cowboy package ${name}`,
    );
    assert(
      exactVersion(version) && version === target.version,
      `${owner}: stale or non-exact package pin ${name}@${version}`,
    );
    result.push(target.id);
  }
  return result.sort();
}

async function repositoryClosure(
  components: ComponentRecord[],
  pluginIds: string[],
): Promise<ComponentClosure> {
  const cargoPackages = await cargoPackageDependencies();
  const graph: Record<string, string[]> = {};
  for (const component of components) {
    const pkg = component.package!;
    let dependencies: Record<string, string>;
    if (pkg.kind === "npm") {
      const manifest = await readJson<{
        dependencies?: Record<string, string>;
        peerDependencies?: Record<string, string>;
        optionalDependencies?: Record<string, string>;
        devDependencies?: Record<string, string>;
      }>(pkg.manifest);
      const groups = [
        manifest.dependencies,
        manifest.peerDependencies,
        manifest.optionalDependencies,
        manifest.devDependencies,
      ];
      // Validate each group independently: a peer must not mask a stale build pin.
      for (const group of groups) {
        resolvePackagePins(component.id, group ?? {}, components);
      }
      dependencies = Object.assign({}, ...groups);
    } else {
      const declared = cargoPackages.get(pkg.name);
      assert(
        declared !== undefined,
        `${component.id}: Cargo package is outside the metadata graph`,
      );
      dependencies = {};
      for (const dependency of declared) {
        if (!dependency.name.startsWith("cowboy-") && !dependency.path) {
          continue;
        }
        const target = components.find((candidate) =>
          candidate.package?.name === dependency.name
        );
        assert(
          target !== undefined,
          `${component.id}: unregistered local Cargo dependency`,
        );
        assert(
          dependency.req === `=${target.version}`,
          `${component.id}: Cargo dependency is not exact`,
        );
        if (dependency.path) {
          assert(
            resolve(dependency.path) ===
              resolve(dirname(target.package!.manifest)),
            `${component.id}: Cargo dependency path differs from registered package`,
          );
        }
        dependencies[dependency.name] = target.version;
      }
    }
    graph[component.id] = resolvePackagePins(
      component.id,
      dependencies,
      components,
    );
  }
  const plugins: ComponentClosure["plugins"] = {};
  for (const id of pluginIds) {
    const root = `plugins/${id}`;
    const files = await repositorySourceFiles(root);
    const manifest = await readJson<PluginManifest>(`${root}/plugin.json`);
    await validateNpmSourceClosure(
      {
        id,
        version: manifest.version,
        publisher: manifest.publisher,
        sources: [root],
        digest: "",
      },
      root,
      Object.fromEntries(manifest.components.flatMap((pin) => {
        const pkg = components.find((component) => component.id === pin.id)
          ?.package;
        return pkg ? [[pkg.name, pin.version]] : [];
      })),
      files,
    );
    plugins[id] = {
      component_release: manifest.component_release,
      components: manifest.components,
      source_digest: await filesDigest(files),
    };
  }
  return { component_dependencies: graph, plugins };
}

async function validateNpmSourceClosure(
  component: ComponentRecord,
  packageRoot: string,
  declaredDependencies: Record<string, string>,
  ownedFiles?: string[],
): Promise<void> {
  const files: string[] = ownedFiles ?? [];
  if (!ownedFiles) await collectFiles(packageRoot, files);
  const imports = /(?:from\s*|import\s*(?:\(\s*)?)["']([^"']+)["']/g;
  for (const file of files.filter((path) => /\.[cm]?tsx?$/.test(path))) {
    const source = await Deno.readTextFile(file);
    for (const match of source.matchAll(imports)) {
      const specifier = match[1]!;
      if (specifier.startsWith(".")) {
        assert(
          confined(packageRoot, resolve(dirname(file), specifier)),
          `${component.id}: relative import escapes package: ${file} -> ${specifier}`,
        );
      } else if (specifier.startsWith("@cowboy/")) {
        const packageName = specifier.split("/").slice(0, 2).join("/");
        assert(
          exactVersion(declaredDependencies[packageName] ?? ""),
          `${component.id}: Cowboy package dependency is not exact: ${specifier}`,
        );
      } else {
        assert(
          !specifier.startsWith("npm:@cowboy/"),
          `${component.id}: Cowboy imports must use declared package names, not inline npm aliases`,
        );
      }
    }
  }
}

function exportTargets(value: unknown): string[] {
  if (typeof value === "string") return [value];
  assert(value !== null && typeof value === "object", "invalid npm exports");
  return Object.values(value as Record<string, unknown>).flatMap(exportTargets);
}

function confined(root: string, path: string): boolean {
  const pathFromRoot = relative(resolve(root), resolve(path));
  return pathFromRoot === "" ||
    (!pathFromRoot.startsWith("..") && !pathFromRoot.startsWith("/"));
}

async function collectFiles(path: string, files: string[]): Promise<void> {
  const stat = await Deno.lstat(path);
  assert(!stat.isSymlink, `${path}: symlink is not a release source`);
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

function validComponentId(value: string): boolean {
  return /^cowboy\.[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value);
}

function assertUnique(values: string[], label: string): void {
  assert(
    new Set(values).size === values.length,
    `${label}: duplicate identity`,
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

import { dirname, relative, resolve } from "node:path";

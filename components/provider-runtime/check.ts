import { createHash } from "node:crypto";
import { lstat, readdir, readFile } from "node:fs/promises";
interface ExactDependency {
  id: string;
  version: string;
  source: string;
  integrity: string;
  private: boolean;
}

type OperatingSystem = "linux" | "macos";
type Architecture = "x86_64" | "aarch64";
type TargetKey = `${OperatingSystem}-${Architecture}`;

interface PrivateComponent {
  dependency: string;
}

interface ProviderSource {
  id: string;
  version: string;
  runtime: {
    dependencies: ExactDependency[];
    platforms: Array<{
      os: OperatingSystem;
      architecture: Architecture;
      private_components: PrivateComponent[];
    }>;
  };
}

interface DownloadTarget {
  url: string;
  integrity?: string;
  sha256?: string;
  sha512?: string;
  entrypoint: string;
}

interface NpmArchiveRecipe {
  kind: "npm_archive" | "npm_brotli";
  version: string;
  targets: Partial<Record<TargetKey, DownloadTarget>>;
}

interface NodeNpmRecipe extends OwnedNodeRecipe {
  kind: "node_npm";
  version: string;
  package_dir: string;
  script: string;
  launcher?: string;
  include_optional?: boolean;
}

interface GitGoRecipe {
  kind: "git_go_static";
  version: string;
  repository: string;
  revision: string;
  subdir: string;
  source_archive_sha256: string;
  files?: string[];
}

interface VendorArchiveRecipe {
  kind: "vendor_archive";
  version: string;
  release: string;
  targets: Partial<Record<TargetKey, DownloadTarget>>;
}

interface OwnedArchiveRecipe {
  kind: "owned_archive";
  version: string;
  package_dir: string;
  launcher: string;
  files: string[];
}

type ComponentRecipe =
  | NpmArchiveRecipe
  | NodeNpmRecipe
  | GitGoRecipe
  | VendorArchiveRecipe
  | OwnedArchiveRecipe;

interface RuntimeLock {
  schema_version: number;
  node: {
    version: string;
    targets: Partial<Record<TargetKey, DownloadTarget>>;
  };
  components: Record<string, ComponentRecipe>;
}

interface NpmPackage {
  version: string;
  private: boolean;
  dependencies: Record<string, string>;
}

interface NpmLock {
  lockfileVersion: number;
  packages: Record<string, {
    version?: string;
    resolved?: string;
    integrity?: string;
    dependencies?: Record<string, string>;
  }>;
}

const runtimeRoot = dirname(fileURLToPath(import.meta.url));
const runtimeLock = await readJson<RuntimeLock>(join(runtimeRoot, "lock.json"));
assert(runtimeLock.schema_version === 1, "unsupported runtime lock schema");
assert(exactVersion(runtimeLock.node.version), "Node.js version is not exact");

const pluginRoots: string[] = [...process.argv.slice(2)];
if (pluginRoots.length === 0) {
  for (const entry of await readdir("plugins", { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    if (await exists(`plugins/${entry.name}/provider.json`)) {
      pluginRoots.push(`plugins/${entry.name}`);
    }
  }
}
pluginRoots.sort();
assert(pluginRoots.length > 0, "no Provider manifests found");

const validatedNpmRecipes = new Set<string>();
const providerIds: string[] = [];
for (const pluginRoot of pluginRoots) {
  const source = await readJson<ProviderSource>(
    join(pluginRoot, "provider.json"),
  );
  const providerId = source.id;
  assert(/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(providerId), "invalid Provider id");
  providerIds.push(providerId);
  assert(exactVersion(source.version), `${providerId}: version is not exact`);

  const dependencies = new Map<string, ExactDependency>();
  for (const dependency of source.runtime.dependencies) {
    assert(
      dependency.private,
      `${providerId}: ${dependency.id} must be private`,
    );
    assert(
      !dependencies.has(dependency.id),
      `${providerId}: duplicate dependency ${dependency.id}`,
    );
    assert(
      exactVersion(dependency.version),
      `${providerId}: ${dependency.id} version is not exact`,
    );
    dependencies.set(dependency.id, dependency);
  }

  const targets = new Set<TargetKey>();
  const dependencyTargets = new Map<string, Set<TargetKey>>();
  for (const platform of source.runtime.platforms) {
    const target = `${platform.os}-${platform.architecture}` as TargetKey;
    assert(!targets.has(target), `${providerId}: duplicate target ${target}`);
    targets.add(target);
    for (const component of platform.private_components) {
      assert(
        dependencies.has(component.dependency),
        `${providerId}: ${target} references unknown dependency ${component.dependency}`,
      );
      const usedTargets = dependencyTargets.get(component.dependency) ??
        new Set<TargetKey>();
      usedTargets.add(target);
      dependencyTargets.set(component.dependency, usedTargets);
    }
  }

  for (const [dependencyId, dependency] of dependencies) {
    const usedTargets = dependencyTargets.get(dependencyId);
    assert(
      usedTargets?.size,
      `${providerId}: unused dependency ${dependencyId}`,
    );
    const recipe = runtimeLock.components[dependencyId];
    assert(recipe, `${providerId}: missing runtime recipe for ${dependencyId}`);
    assert(
      recipe.version === dependency.version,
      `${providerId}: runtime lock version mismatch for ${dependencyId}`,
    );
    await validateDependencySource(providerId, dependency, recipe);

    for (const target of usedTargets) {
      switch (recipe.kind) {
        case "npm_archive":
        case "npm_brotli":
          validateDownloadTarget(
            `${providerId}: ${dependencyId} ${target}`,
            required(recipe.targets[target], "missing platform archive"),
            "sri",
          );
          break;
        case "node_npm":
          validateDownloadTarget(
            `${providerId}: Node.js ${target}`,
            required(
              runtimeLock.node.targets[target],
              "missing Node.js archive",
            ),
            "sha256",
          );
          if (!validatedNpmRecipes.has(dependencyId)) {
            await validateNpmRecipe(providerId, dependency, recipe);
            validatedNpmRecipes.add(dependencyId);
          }
          break;
        case "owned_archive":
          break;
        case "vendor_archive": {
          const locked = required(
            recipe.targets[target],
            `${providerId}: ${dependencyId} has no vendor archive for ${target}`,
          );
          validateDownloadTarget(
            `${providerId}: ${dependencyId} ${target}`,
            locked,
            "sha256",
          );
          assert(
            locked.url.startsWith(recipe.release) &&
              /^[a-f0-9]{128}$/.test(locked.sha512 ?? ""),
            `${providerId}: ${dependencyId} ${target} vendor archive is not pinned below its release`,
          );
          break;
        }
        case "git_go_static":
          assert(
            target.startsWith("linux-") || target === "macos-aarch64",
            `${providerId}: ${dependencyId} cannot build for ${target}`,
          );
          assert(
            (recipe.files ?? []).every((file) =>
              /^[a-z0-9][a-z0-9._-]*$/.test(file)
            ) &&
              new Set(recipe.files ?? []).size === (recipe.files ?? []).length,
            `${providerId}: ${dependencyId} component files must be distinct plain names`,
          );
          break;
      }
    }
  }
}

console.log(
  JSON.stringify({
    runtime_lock_schema: runtimeLock.schema_version,
    node_version: runtimeLock.node.version,
    providers: providerIds,
  }),
);

async function validateDependencySource(
  providerId: string,
  dependency: ExactDependency,
  recipe: ComponentRecipe,
): Promise<void> {
  if (recipe.kind === "git_go_static") {
    const repository = recipe.repository.replace(
      /^git@([^:]+):/,
      "git@$1/",
    );
    const expectedSource =
      `git+ssh://${repository}#${recipe.revision}:${recipe.subdir}`;
    assert(
      dependency.source === expectedSource,
      `${providerId}: Git source mismatch for ${dependency.id}`,
    );
    assert(
      dependency.integrity === `sha256:${recipe.source_archive_sha256}`,
      `${providerId}: Git source digest mismatch for ${dependency.id}`,
    );
    assert(
      /^[a-f0-9]{40}$/.test(recipe.revision),
      `${providerId}: Git revision is not a commit`,
    );
    assert(
      /^[a-f0-9]{64}$/.test(recipe.source_archive_sha256),
      `${providerId}: Git archive digest is invalid`,
    );
    return;
  }

  if (recipe.kind === "owned_archive") {
    assert(
      /^packages\/[a-z0-9-]+$/.test(recipe.package_dir) &&
        dependency.source ===
          `cowboy:components/provider-runtime/${recipe.package_dir}`,
      `${providerId}: owned archive source mismatch for ${dependency.id}`,
    );
    assert(
      dependency.integrity ===
        await ownedArchiveIntegrity(
          join(runtimeRoot, recipe.package_dir),
          recipe.launcher,
          recipe.files,
        ),
      `${providerId}: owned archive integrity mismatch for ${dependency.id}`,
    );
    return;
  }
  if (recipe.kind === "vendor_archive") {
    // The vendor's versioned release directory is the source; the integrity
    // binds every pinned target, so a lock edit cannot drift from the
    // signed Provider manifest.
    assert(
      recipe.release.startsWith("https://") &&
        recipe.release.endsWith("/") &&
        recipe.release.includes(`/${dependency.version}`) &&
        !recipe.release.includes("latest") &&
        dependency.source === recipe.release,
      `${providerId}: vendor release mismatch for ${dependency.id}`,
    );
    assert(
      dependency.integrity === vendorIntegrity(recipe),
      `${providerId}: vendor integrity mismatch for ${dependency.id}`,
    );
    return;
  }

  if (recipe.kind === "node_npm" && dependency.source.startsWith("cowboy:")) {
    assert(
      dependency.source === ownedNodeSource(recipe) &&
        dependency.integrity === await ownedNodeIntegrity(runtimeRoot, recipe),
      `${providerId}: owned node package integrity mismatch for ${dependency.id}`,
    );
    return;
  }
  assert(
    dependency.source.startsWith("https://registry.npmjs.org/") &&
      dependency.source.endsWith(`-${dependency.version}.tgz`),
    `${providerId}: npm source is not an exact registry tarball for ${dependency.id}`,
  );
  assert(
    /^sha512-[A-Za-z0-9+/]+={0,2}$/.test(dependency.integrity),
    `${providerId}: npm integrity is invalid for ${dependency.id}`,
  );
}

function validateDownloadTarget(
  label: string,
  target: DownloadTarget,
  integrityKind: "sri" | "sha256",
): void {
  assert(
    target.url.startsWith("https://") && !target.url.includes("latest"),
    `${label}: URL is not immutable HTTPS`,
  );
  assert(
    safeEntrypoint(target.entrypoint),
    `${label}: archive entrypoint is unsafe`,
  );
  if (integrityKind === "sri") {
    assert(
      /^sha512-[A-Za-z0-9+/]+={0,2}$/.test(target.integrity ?? ""),
      `${label}: npm SRI is invalid`,
    );
  } else {
    assert(
      /^[a-f0-9]{64}$/.test(target.sha256 ?? ""),
      `${label}: SHA-256 is invalid`,
    );
  }
}

async function validateNpmRecipe(
  providerId: string,
  dependency: ExactDependency,
  recipe: NodeNpmRecipe,
): Promise<void> {
  assert(
    recipe.include_optional === undefined || recipe.include_optional === true,
    `${providerId}: ${dependency.id} include_optional must be true or absent`,
  );
  if (recipe.launcher !== undefined) {
    assert(
      /^packages\/[a-z0-9-]+\/launch\.mjs$/.test(recipe.launcher) &&
        (await lstat(join(runtimeRoot, recipe.launcher))).isFile(),
      `${providerId}: ${dependency.id} launcher is not an owned regular source`,
    );
  }
  assert(
    safeEntrypoint(recipe.script),
    `${providerId}: ${dependency.id} script is unsafe`,
  );
  const packageJson = await readJson<NpmPackage>(
    join(runtimeRoot, recipe.package_dir, "package.json"),
  );
  const lock = await readJson<NpmLock>(
    join(runtimeRoot, recipe.package_dir, "package-lock.json"),
  );
  assert(
    packageJson.private,
    `${providerId}: runtime npm package must be private`,
  );
  assert(
    packageJson.version === recipe.version,
    `${providerId}: runtime npm package version mismatch for ${dependency.id}`,
  );
  const directDependencies = Object.entries(packageJson.dependencies);
  assert(lock.lockfileVersion === 3, `${providerId}: npm lockfile must be v3`);
  // A Cowboy-owned program has no primary registry package; its owned
  // inputs are bound by the dependency integrity instead.
  const owned = dependency.source === ownedNodeSource(recipe);
  const primary = owned ? [] : directDependencies.filter(([name, version]) =>
    version === dependency.version &&
    lock.packages[`node_modules/${name}`]?.resolved === dependency.source
  );
  assert(
    owned || primary.length === 1,
    `${providerId}: ${dependency.id} runtime package must have one exact primary dependency`,
  );
  // Private transport libraries belong to this adapter's complete npm lock,
  // not to a Machine-wide installation or a new executable component. Every
  // direct input must be exact and agree with the integrity-bound lock.
  assert(
    JSON.stringify(Object.keys(packageJson.dependencies).sort()) ===
      JSON.stringify(Object.keys(lock.packages[""]?.dependencies ?? {}).sort()),
    `${providerId}: npm lock root dependency set differs`,
  );
  for (const [name, version] of directDependencies) {
    const locked = lock.packages[`node_modules/${name}`];
    assert(
      exactVersion(version) &&
        lock.packages[""]?.dependencies?.[name] === version &&
        locked?.version === version &&
        locked.resolved?.startsWith("https://") &&
        /^sha512-[A-Za-z0-9+/]+={0,2}$/.test(locked.integrity ?? ""),
      `${providerId}: private npm input ${name} is not exactly integrity-pinned`,
    );
  }
  if (owned) {
    ownedNodeInputs(recipe);
    return;
  }
  const [packageName, packageVersion] = required(
    primary[0],
    "missing npm dependency",
  );
  assert(
    packageVersion === dependency.version,
    `${providerId}: package.json does not pin ${dependency.id}`,
  );
  assert(
    lock.packages[""]?.dependencies?.[packageName] === dependency.version,
    `${providerId}: npm lock root does not pin ${dependency.id}`,
  );
  const lockedPackage = lock.packages[`node_modules/${packageName}`];
  assert(
    lockedPackage?.version === dependency.version &&
      lockedPackage.resolved === dependency.source &&
      lockedPackage.integrity === dependency.integrity,
    `${providerId}: npm lock payload mismatch for ${dependency.id}`,
  );
}

/** SHA-256 over the sorted pinned targets of a vendor archive recipe. */
function vendorIntegrity(recipe: VendorArchiveRecipe): string {
  const lines = Object.entries(recipe.targets)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([target, locked]) =>
      `${target} ${locked?.url} ${locked?.sha512} ${locked?.sha256} ${locked?.entrypoint}\n`
    )
    .join("");
  return `sha256:${createHash("sha256").update(lines).digest("hex")}`;
}

function safeEntrypoint(value: string): boolean {
  return value.length > 0 && !value.startsWith("/") &&
    !value.split("/").some((segment) => segment === "" || segment === "..");
}

function exactVersion(value: string): boolean {
  return /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(value);
}

async function readJson<T>(path: string): Promise<T> {
  return JSON.parse(await readFile(path, "utf8")) as T;
}

async function exists(path: string): Promise<boolean> {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if ((error as { code?: string }).code === "ENOENT") return false;
    throw error;
  }
}

function required<T>(value: T | undefined, message: string): T {
  assert(value !== undefined, message);
  return value;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  ownedArchiveIntegrity,
  ownedNodeInputs,
  ownedNodeIntegrity,
  ownedNodeSource,
  type OwnedNodeRecipe,
} from "./owned-sources.ts";

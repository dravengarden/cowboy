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
  entrypoint: string;
}

interface NpmArchiveRecipe {
  kind: "npm_archive" | "npm_brotli";
  version: string;
  targets: Partial<Record<TargetKey, DownloadTarget>>;
}

interface NodeNpmRecipe {
  kind: "node_npm";
  version: string;
  package_dir: string;
  script: string;
}

interface GitGoRecipe {
  kind: "git_go_static";
  version: string;
  repository: string;
  revision: string;
  subdir: string;
  source_archive_sha256: string;
}

type ComponentRecipe = NpmArchiveRecipe | NodeNpmRecipe | GitGoRecipe;

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

const runtimeLock = await readJson<RuntimeLock>("providers/runtime-lock.json");
assert(runtimeLock.schema_version === 1, "unsupported runtime lock schema");
assert(exactVersion(runtimeLock.node.version), "Node.js version is not exact");

const providerIds: string[] = [];
for await (const entry of Deno.readDir("providers")) {
  if (!entry.isDirectory) continue;
  if (await exists(`providers/${entry.name}/provider.json`)) {
    providerIds.push(entry.name);
  }
}
providerIds.sort();
assert(providerIds.length > 0, "no Provider manifests found");

const validatedNpmRecipes = new Set<string>();
for (const providerId of providerIds) {
  const source = await readJson<ProviderSource>(
    `providers/${providerId}/provider.json`,
  );
  assert(
    source.id === providerId,
    `${providerId}: directory identity mismatch`,
  );
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
    validateDependencySource(providerId, dependency, recipe);

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
        case "git_go_static":
          assert(
            target.startsWith("linux-") || target === "macos-aarch64",
            `${providerId}: ${dependencyId} cannot build for ${target}`,
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

function validateDependencySource(
  providerId: string,
  dependency: ExactDependency,
  recipe: ComponentRecipe,
): void {
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
    safeEntrypoint(recipe.script),
    `${providerId}: ${dependency.id} script is unsafe`,
  );
  const packageJson = await readJson<NpmPackage>(
    `${recipe.package_dir}/package.json`,
  );
  const lock = await readJson<NpmLock>(
    `${recipe.package_dir}/package-lock.json`,
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
  assert(
    directDependencies.length === 1,
    `${providerId}: ${dependency.id} runtime package must have one direct dependency`,
  );
  const [packageName, packageVersion] = required(
    directDependencies[0],
    "missing npm dependency",
  );
  assert(
    packageVersion === dependency.version,
    `${providerId}: package.json does not pin ${dependency.id}`,
  );
  assert(lock.lockfileVersion === 3, `${providerId}: npm lockfile must be v3`);
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

function safeEntrypoint(value: string): boolean {
  return value.length > 0 && !value.startsWith("/") &&
    !value.split("/").some((segment) => segment === "" || segment === "..");
}

function exactVersion(value: string): boolean {
  return /^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(value);
}

async function readJson<T>(path: string): Promise<T> {
  return JSON.parse(await Deno.readTextFile(path)) as T;
}

async function exists(path: string): Promise<boolean> {
  try {
    await Deno.lstat(path);
    return true;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return false;
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

interface ExactDependency {
  id: string;
  version: string;
  source: string;
  integrity: string;
}

type OperatingSystem = "linux" | "macos";
type Architecture = "x86_64" | "aarch64";
type TargetKey = `${OperatingSystem}-${Architecture}`;

interface PrivateComponent {
  kind: string;
  slot: string;
  dependency: string;
  command: string;
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
  probe: string[];
  targets: Partial<Record<TargetKey, DownloadTarget>>;
}

interface NodeNpmRecipe {
  kind: "node_npm";
  version: string;
  package_dir: string;
  script: string;
  probe: string[];
}

interface GitGoRecipe {
  kind: "git_go_static";
  version: string;
  repository: string;
  revision: string;
  subdir: string;
  source_archive_sha256: string;
  binary: string;
  probe: string[];
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

interface ReleasedComponent {
  kind: string;
  slot: string;
  dependency: string;
  version: string;
  command: string;
  artifact_url: string;
  artifact_digest: string;
  artifact_format: "raw" | "tar_gz";
  entrypoint?: string;
  probe: { args: string[]; timeout_ms: number };
}

const providerId = Deno.args[0] ?? "";
const baseUrl =
  (Deno.args[1] ?? "https://cowboy.stormbird.xyz/provider-artifacts")
    .replace(/\/+$/, "");
if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(providerId)) {
  throw new Error("Provider id must use lowercase kebab-case");
}
if (!baseUrl.startsWith("https://") || baseUrl.includes("latest")) {
  throw new Error("Provider artifact base URL must be immutable HTTPS");
}

const source = await readJson<ProviderSource>(
  `providers/${providerId}/provider.json`,
);
if (source.id !== providerId) {
  throw new Error("Provider directory identity mismatch");
}
const runtimeLock = await readJson<RuntimeLock>("providers/runtime-lock.json");
if (runtimeLock.schema_version !== 1) {
  throw new Error("unsupported runtime lock schema");
}

const outputRoot = `dist/providers/${providerId}/runtime`;
const cacheRoot = "dist/provider-runtime-cache";
await Deno.mkdir(cacheRoot, { recursive: true });
await Deno.remove(outputRoot, { recursive: true }).catch((error) => {
  if (!(error instanceof Deno.errors.NotFound)) throw error;
});
await Deno.mkdir(outputRoot, { recursive: true });
const temporaryDirectory = await Deno.makeTempDir({
  dir: "dist",
  prefix: ".provider-runtime-",
});
const temporary = await Deno.realPath(temporaryDirectory);

try {
  const matrix: Array<{
    os: OperatingSystem;
    architecture: Architecture;
    components: ReleasedComponent[];
  }> = [];
  for (const platform of source.runtime.platforms) {
    const target = `${platform.os}-${platform.architecture}` as TargetKey;
    const components: ReleasedComponent[] = [];
    for (const requirement of platform.private_components) {
      const dependency = source.runtime.dependencies.find((candidate) =>
        candidate.id === requirement.dependency
      );
      if (!dependency) {
        throw new Error(`missing dependency ${requirement.dependency}`);
      }
      const recipe = runtimeLock.components[dependency.id];
      if (!recipe || recipe.version !== dependency.version) {
        throw new Error(
          `runtime lock does not match ${dependency.id}@${dependency.version}`,
        );
      }
      const built = await buildComponent(
        recipe,
        dependency,
        requirement.command,
        target,
        `${outputRoot}/${target}`,
      );
      const digest = await sha256(built.path);
      const name = built.path.slice(built.path.lastIndexOf("/") + 1);
      const released: ReleasedComponent = {
        kind: requirement.kind,
        slot: requirement.slot,
        dependency: dependency.id,
        version: dependency.version,
        command: requirement.command,
        artifact_url: `${baseUrl}/${digest}/${name}`,
        artifact_digest: `sha256:${digest}`,
        artifact_format: built.format,
        probe: { args: recipe.probe, timeout_ms: 30_000 },
      };
      if (built.entrypoint) released.entrypoint = built.entrypoint;
      components.push(released);
      if (target === "linux-x86_64") {
        await probeArtifact(
          built.path,
          built.format,
          built.entrypoint,
          recipe.probe,
        );
      }
    }
    matrix.push({
      os: platform.os,
      architecture: platform.architecture,
      components,
    });
  }
  const manifestPath = `${outputRoot}/runtime-artifacts.json`;
  await Deno.writeTextFile(
    manifestPath,
    `${JSON.stringify(matrix, null, 2)}\n`,
  );
  console.log(JSON.stringify({
    provider_id: providerId,
    provider_version: source.version,
    runtime_manifest: manifestPath,
    targets: matrix.map((entry) => `${entry.os}-${entry.architecture}`),
  }));
} finally {
  await Deno.remove(temporary, { recursive: true }).catch(() => undefined);
}

async function buildComponent(
  recipe: ComponentRecipe,
  dependency: ExactDependency,
  command: string,
  target: TargetKey,
  destination: string,
): Promise<{ path: string; format: "raw" | "tar_gz"; entrypoint?: string }> {
  await Deno.mkdir(destination, { recursive: true });
  switch (recipe.kind) {
    case "npm_archive": {
      const locked = requiredTarget(recipe.targets, target, dependency.id);
      const archive = await downloadSRI(
        locked.url,
        required(locked.integrity, "npm integrity"),
      );
      const output = `${destination}/${command}.tar.gz`;
      await Deno.copyFile(archive, output);
      return { path: output, format: "tar_gz", entrypoint: locked.entrypoint };
    }
    case "npm_brotli": {
      const locked = requiredTarget(recipe.targets, target, dependency.id);
      const archive = await downloadSRI(
        locked.url,
        required(locked.integrity, "npm integrity"),
      );
      const stage = await Deno.makeTempDir({
        dir: temporary,
        prefix: `${command}-`,
      });
      await run("tar", ["-xzf", archive, "-C", stage, locked.entrypoint]);
      await Deno.mkdir(`${stage}/bin`, { recursive: true });
      await run("brotli", [
        "--decompress",
        `--output=${stage}/bin/${command}`,
        `${stage}/${locked.entrypoint}`,
      ]);
      await Deno.chmod(`${stage}/bin/${command}`, 0o755);
      await Deno.remove(`${stage}/package`, { recursive: true });
      await writeProvenance(
        stage,
        dependency,
        target,
        runtimeLock.node.version,
      );
      const output = `${destination}/${command}.tar.gz`;
      await deterministicTarGz(stage, output);
      return { path: output, format: "tar_gz", entrypoint: `bin/${command}` };
    }
    case "node_npm": {
      const node = requiredTarget(runtimeLock.node.targets, target, "Node.js");
      const nodeArchive = await downloadSha256(
        node.url,
        required(node.sha256, "Node SHA-256"),
      );
      const stage = await Deno.makeTempDir({
        dir: temporary,
        prefix: `${command}-`,
      });
      await Deno.mkdir(`${stage}/runtime`, { recursive: true });
      await run("tar", [
        "-xzf",
        nodeArchive,
        "-C",
        `${stage}/runtime`,
        "--strip-components=2",
        node.entrypoint,
      ]);
      await Deno.mkdir(`${stage}/app`, { recursive: true });
      await Deno.copyFile(
        `${recipe.package_dir}/package.json`,
        `${stage}/app/package.json`,
      );
      await Deno.copyFile(
        `${recipe.package_dir}/package-lock.json`,
        `${stage}/app/package-lock.json`,
      );
      const [npmOs, npmCpu] = target === "macos-aarch64"
        ? ["darwin", "arm64"]
        : target === "linux-aarch64"
        ? ["linux", "arm64"]
        : ["linux", "x64"];
      await run("npm", [
        "ci",
        "--ignore-scripts",
        "--omit=optional",
        "--no-audit",
        "--no-fund",
        `--os=${npmOs}`,
        `--cpu=${npmCpu}`,
      ], { cwd: `${stage}/app` });
      await Deno.remove(`${stage}/app/node_modules/.bin`, { recursive: true })
        .catch((error) => {
          if (!(error instanceof Deno.errors.NotFound)) throw error;
        });
      await Deno.mkdir(`${stage}/bin`, { recursive: true });
      await Deno.writeTextFile(
        `${stage}/bin/${command}`,
        `#!/bin/sh\nset -eu\ncase "$0" in */*) cowboy_dir=\${0%/*} ;; *) cowboy_dir=. ;; esac\ncowboy_root=$(CDPATH= cd -- "$cowboy_dir/.." && pwd)\nexec "$cowboy_root/runtime/node" "$cowboy_root/app/${recipe.script}" "$@"\n`,
        { mode: 0o755 },
      );
      await writeProvenance(
        stage,
        dependency,
        target,
        runtimeLock.node.version,
      );
      await rejectLinks(stage);
      const output = `${destination}/${command}.tar.gz`;
      await deterministicTarGz(stage, output);
      return { path: output, format: "tar_gz", entrypoint: `bin/${command}` };
    }
    case "git_go_static": {
      verifyGitDependency(dependency, recipe);
      const stage = await Deno.makeTempDir({
        dir: temporary,
        prefix: `${command}-`,
      });
      const sourceRoot = await materializeGitSource(recipe, stage);
      const goArchitecture = target.endsWith("aarch64") ? "arm64" : "amd64";
      if (!target.startsWith("linux-")) {
        throw new Error(`${dependency.id} has no ${target} build contract`);
      }
      const output = `${await Deno.realPath(destination)}/${command}`;
      await run("nix", [
        "develop",
        `path:${sourceRoot}`,
        "-c",
        "env",
        "CGO_ENABLED=0",
        "GOOS=linux",
        `GOARCH=${goArchitecture}`,
        "go",
        "build",
        "-buildvcs=false",
        "-trimpath",
        "-ldflags=-s -w -buildid=",
        "-o",
        output,
        `./cmd/${recipe.binary}`,
      ], { cwd: sourceRoot });
      await Deno.chmod(output, 0o755);
      return { path: output, format: "raw" };
    }
  }
}

async function materializeGitSource(
  recipe: GitGoRecipe,
  stage: string,
): Promise<string> {
  let repository = Deno.env.get("COLUMBUS_ROOT") ?? "/home/draven/columbus";
  if (!(await exists(`${repository}/.git`))) {
    repository = `${stage}/repository`;
    await run("git", [
      "clone",
      "--filter=blob:none",
      "--no-checkout",
      recipe.repository,
      repository,
    ]);
  }
  await run("git", ["cat-file", "-e", `${recipe.revision}^{commit}`], {
    cwd: repository,
  });
  const archive = `${stage}/source.tar`;
  await run("git", [
    "archive",
    "--format=tar",
    `--output=${archive}`,
    recipe.revision,
    recipe.subdir,
  ], { cwd: repository });
  const digest = await sha256(archive);
  if (digest !== recipe.source_archive_sha256) {
    throw new Error(`Git source digest mismatch for ${recipe.binary}`);
  }
  const extracted = `${stage}/source`;
  await Deno.mkdir(extracted, { recursive: true });
  await run("tar", ["-xf", archive, "-C", extracted]);
  return `${extracted}/${recipe.subdir}`;
}

function verifyGitDependency(
  dependency: ExactDependency,
  recipe: GitGoRecipe,
): void {
  const expected = `git+ssh://${
    recipe.repository.replace(/^git@([^:]+):/, "git@$1/")
  }#${recipe.revision}:${recipe.subdir}`;
  if (dependency.source !== expected) {
    throw new Error(`Git dependency source mismatch for ${dependency.id}`);
  }
  if (dependency.integrity !== `sha256:${recipe.source_archive_sha256}`) {
    throw new Error(
      `Git dependency source integrity mismatch for ${dependency.id}`,
    );
  }
}

async function writeProvenance(
  stage: string,
  dependency: ExactDependency,
  target: TargetKey,
  nodeVersion: string,
): Promise<void> {
  await Deno.writeTextFile(
    `${stage}/cowboy-runtime.json`,
    `${
      JSON.stringify(
        {
          schema_version: 1,
          dependency: dependency.id,
          version: dependency.version,
          target,
          node_version: nodeVersion,
        },
        null,
        2,
      )
    }\n`,
  );
}

async function probeArtifact(
  artifact: string,
  format: "raw" | "tar_gz",
  entrypoint: string | undefined,
  args: string[],
): Promise<void> {
  const probeRoot = await Deno.makeTempDir({
    dir: temporary,
    prefix: "probe-",
  });
  const executablePath = format === "raw"
    ? artifact
    : `${probeRoot}/${required(entrypoint, "archive entrypoint")}`;
  if (format === "tar_gz") {
    await run("tar", ["-xzf", artifact, "-C", probeRoot]);
    await Deno.chmod(executablePath, 0o755);
  }
  const executable = await Deno.realPath(executablePath);
  const home = `${probeRoot}/home`;
  await Deno.mkdir(home, { recursive: true });
  const child = new Deno.Command(executable, {
    args,
    cwd: probeRoot,
    env: {
      HOME: home,
      XDG_CONFIG_HOME: `${home}/.config`,
      GROK_HOME: `${home}/.grok`,
    },
    stdin: "null",
    stdout: "inherit",
    stderr: "inherit",
  }).spawn();
  const timeout = setTimeout(() => child.kill("SIGKILL"), 30_000);
  const status = await child.status;
  clearTimeout(timeout);
  if (!status.success) throw new Error(`runtime probe failed for ${artifact}`);
}

async function deterministicTarGz(
  stage: string,
  output: string,
): Promise<void> {
  const names = [];
  for await (const entry of Deno.readDir(stage)) names.push(entry.name);
  names.sort();
  const tarPath = `${output}.tar`;
  await run("tar", [
    "--sort=name",
    "--mtime=@0",
    "--owner=0",
    "--group=0",
    "--numeric-owner",
    "--format=gnu",
    "-cf",
    tarPath,
    "-C",
    stage,
    ...names,
  ]);
  await run("gzip", ["-n", "-9", "-f", tarPath]);
  await Deno.rename(`${tarPath}.gz`, output);
}

async function rejectLinks(root: string): Promise<void> {
  for await (const entry of Deno.readDir(root)) {
    const path = `${root}/${entry.name}`;
    const metadata = await Deno.lstat(path);
    if (metadata.isSymlink) {
      throw new Error(`runtime archive contains symlink ${path}`);
    }
    if (metadata.isDirectory) await rejectLinks(path);
  }
}

async function downloadSRI(url: string, integrity: string): Promise<string> {
  const match = /^sha512-([A-Za-z0-9+/]+={0,2})$/.exec(integrity);
  if (!match) throw new Error(`unsupported npm integrity for ${url}`);
  const expected = base64Hex(match[1]);
  return await downloadVerified(url, "sha512", expected);
}

async function downloadSha256(url: string, expected: string): Promise<string> {
  if (!/^[a-f0-9]{64}$/.test(expected)) {
    throw new Error(`invalid SHA-256 for ${url}`);
  }
  return await downloadVerified(url, "sha256", expected);
}

async function downloadVerified(
  url: string,
  algorithm: "sha256" | "sha512",
  expected: string,
): Promise<string> {
  const path = `${cacheRoot}/${algorithm}-${expected}`;
  if (!(await exists(path))) {
    const partial = `${path}.${Deno.pid}.partial`;
    await run("curl", [
      "--fail",
      "--location",
      "--retry",
      "3",
      "--output",
      partial,
      url,
    ]);
    const actual = await digestFile(partial, algorithm);
    if (actual !== expected) {
      await Deno.remove(partial).catch(() => undefined);
      throw new Error(`download digest mismatch for ${url}`);
    }
    await Deno.rename(partial, path).catch(async (error) => {
      if (!(error instanceof Deno.errors.AlreadyExists)) throw error;
      await Deno.remove(partial);
    });
  }
  const actual = await digestFile(path, algorithm);
  if (actual !== expected) {
    throw new Error(`cached download digest mismatch for ${url}`);
  }
  return path;
}

async function sha256(path: string): Promise<string> {
  return await digestFile(path, "sha256");
}

async function digestFile(
  path: string,
  algorithm: "sha256" | "sha512",
): Promise<string> {
  const output = await new Deno.Command(`${algorithm}sum`, { args: [path] })
    .output();
  if (!output.success) throw new Error(`${algorithm}sum failed for ${path}`);
  return new TextDecoder().decode(output.stdout).trim().split(/\s+/)[0]
    .toLowerCase();
}

function base64Hex(value: string): string {
  const bytes = Uint8Array.from(
    atob(value),
    (character) => character.charCodeAt(0),
  );
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

async function run(
  command: string,
  args: string[],
  options: { cwd?: string } = {},
): Promise<void> {
  const status = await new Deno.Command(command, {
    args,
    cwd: options.cwd,
    stdin: "null",
    stdout: "inherit",
    stderr: "inherit",
  }).spawn().status;
  if (!status.success) throw new Error(`${command} exited ${status.code}`);
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

function required<T>(value: T | undefined, label: string): T {
  if (value === undefined) throw new Error(`missing ${label}`);
  return value;
}

function requiredTarget<T>(
  targets: Partial<Record<TargetKey, T>>,
  target: TargetKey,
  label: string,
): T {
  return required(targets[target], `${label} runtime lock for ${target}`);
}

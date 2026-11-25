import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
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
  sha512?: string;
  entrypoint: string;
}

interface NpmArchiveRecipe {
  kind: "npm_archive" | "npm_brotli";
  version: string;
  probe: string[];
  targets: Partial<Record<TargetKey, DownloadTarget>>;
}

interface NodeRuntimeRecipe {
  kind: "node_runtime";
  version: string;
  probe: string[];
}

interface NodeNpmRecipe extends OwnedNodeRecipe {
  kind: "node_npm";
  version: string;
  package_dir: string;
  script: string;
  launcher?: string;
  /** Keep platform-filtered optional native bindings the package requires. */
  include_optional?: boolean;
  /** Set false when the Provider declares a separate `acp_runtime` `node` component. */
  bundle_node?: boolean;
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
  /** Regular files from the source subdir shipped beside the binary. */
  files?: string[];
  probe: string[];
}

/**
 * A vendor-hosted native archive Cowboy must not redistribute. Each Machine
 * downloads it from the vendor's immutable versioned URL; the build only
 * verifies the vendor-published SHA-512 and the pinned SHA-256 and probes it.
 */
interface VendorArchiveRecipe {
  kind: "vendor_archive";
  version: string;
  /** Vendor's versioned release directory; every target URL lies below it. */
  release: string;
  probe: string[];
  targets: Partial<Record<TargetKey, DownloadTarget>>;
}

/**
 * A small Cowboy-owned launcher shipped as a deterministic archive: one
 * POSIX shell entrypoint plus read-only data files from `package_dir`.
 */
interface OwnedArchiveRecipe {
  kind: "owned_archive";
  version: string;
  package_dir: string;
  launcher: string;
  files: string[];
  probe: string[];
}

type ComponentRecipe =
  | NpmArchiveRecipe
  | NodeRuntimeRecipe
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

const runtimeRoot = dirname(fileURLToPath(import.meta.url));
const pluginRoot = process.argv.slice(2)[0] ?? "";
const baseUrl = (process.argv.slice(2)[1] ?? "").replace(/\/+$/, "");
if (!baseUrl.startsWith("https://") || baseUrl.includes("latest")) {
  throw new Error("Provider artifact base URL must be immutable HTTPS");
}

const source = await readJson<ProviderSource>(
  join(pluginRoot, "provider.json"),
);
const providerId = source.id;
if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(providerId)) {
  throw new Error("Provider id must use lowercase kebab-case");
}
const runtimeLock = await readJson<RuntimeLock>(join(runtimeRoot, "lock.json"));
if (runtimeLock.schema_version !== 1) {
  throw new Error("unsupported runtime lock schema");
}

const outputBase = process.argv.slice(2)[2] ?? "dist/plugins";
const outputRoot = join(outputBase, providerId, "runtime");
const cacheRoot = join(outputBase, ".runtime-cache");
await mkdir(cacheRoot, { recursive: true });
await rm(outputRoot, { recursive: true }).catch((error) => {
  if (!((error as { code?: string }).code === "ENOENT")) throw error;
});
await mkdir(outputRoot, { recursive: true });
const temporaryDirectory = await mkdtemp(
  join(outputBase, ".provider-runtime-"),
);
const temporary = await realpath(temporaryDirectory);

try {
  const matrix: Array<{
    os: OperatingSystem;
    architecture: Architecture;
    components: ReleasedComponent[];
  }> = [];
  for (const platform of source.runtime.platforms) {
    const target = `${platform.os}-${platform.architecture}` as TargetKey;
    const components: ReleasedComponent[] = [];
    const probeItems: Array<{
      requirement: PrivateComponent;
      path: string;
      digest: string;
      format: "raw" | "tar_gz";
      entrypoint?: string;
      probe: string[];
    }> = [];
    for (const requirement of platform.private_components) {
      validateComponentRef(requirement);
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
      if (built.vendorUrl !== undefined) {
        // Never copied into the Catalog: the signed release binds the
        // vendor URL and digest, and each Machine verifies both itself.
        if (built.vendorSha256 !== digest) {
          throw new Error(`vendor archive SHA-256 mismatch for ${dependency.id}`);
        }
      }
      const released: ReleasedComponent = {
        kind: pluginComponentKind(requirement.kind),
        slot: requirement.slot,
        dependency: dependency.id,
        version: dependency.version,
        command: requirement.command,
        artifact_url: built.vendorUrl ?? `${baseUrl}/${digest}/${name}`,
        artifact_digest: `sha256:${digest}`,
        artifact_format: built.format,
        probe: { args: recipe.probe, timeout_ms: 30_000 },
      };
      if (built.entrypoint) released.entrypoint = built.entrypoint;
      components.push(released);
      if (target === "linux-x86_64") {
        probeItems.push({
          requirement,
          path: built.path,
          digest,
          format: built.format,
          entrypoint: built.entrypoint,
          probe: recipe.probe,
        });
      }
    }
    if (target === "linux-x86_64" && probeItems.length > 0) {
      await probePlatformArtifacts(temporary, probeItems);
    }
    matrix.push({
      os: platform.os,
      architecture: platform.architecture,
      components,
    });
  }
  const manifestPath = `${outputRoot}/runtime-artifacts.json`;
  await writeFile(
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
  await rm(temporary, { recursive: true }).catch(() => undefined);
}

function pluginComponentKind(kind: string): string {
  const mapped: Record<string, string> = {
    provider_cli: "agent_cli",
    provider_adapter: "agent_adapter",
    provider_gateway: "agent_gateway",
    acp_runtime: "acp_runtime",
  };
  const value = mapped[kind];
  if (!value) {
    throw new Error(`unsupported Agent Plugin component kind ${kind}`);
  }
  return value;
}

async function buildComponent(
  recipe: ComponentRecipe,
  dependency: ExactDependency,
  command: string,
  target: TargetKey,
  destination: string,
): Promise<{
  path: string;
  format: "raw" | "tar_gz";
  entrypoint?: string;
  vendorUrl?: string;
  vendorSha256?: string;
}> {
  await mkdir(destination, { recursive: true });
  switch (recipe.kind) {
    case "owned_archive": {
      const sourceDir = join(runtimeRoot, recipe.package_dir);
      if (
        !/^packages\/[a-z0-9-]+$/.test(recipe.package_dir) ||
        dependency.source !== `cowboy:components/provider-runtime/${recipe.package_dir}` ||
        dependency.integrity !==
          await ownedArchiveIntegrity(sourceDir, recipe.launcher, recipe.files)
      ) {
        throw new Error(`owned archive source mismatch for ${dependency.id}`);
      }
      const stage = await mkdtemp(join(temporary, `${command}-`));
      await mkdir(`${stage}/bin`, { recursive: true });
      await mkdir(`${stage}/share`, { recursive: true });
      await copyFile(join(sourceDir, recipe.launcher), `${stage}/bin/${command}`);
      await chmod(`${stage}/bin/${command}`, 0o755);
      for (const file of recipe.files) {
        await copyFile(join(sourceDir, file), `${stage}/share/${file}`);
        await chmod(`${stage}/share/${file}`, 0o444);
      }
      await rejectLinks(stage);
      const output = `${destination}/${command}.tar.gz`;
      await deterministicTarGz(stage, output);
      return { path: output, format: "tar_gz", entrypoint: `bin/${command}` };
    }
    case "vendor_archive": {
      const locked = requiredTarget(recipe.targets, target, dependency.id);
      if (
        !locked.url.startsWith(recipe.release) ||
        dependency.source !== recipe.release
      ) {
        throw new Error(`vendor archive source mismatch for ${dependency.id}`);
      }
      const sha512 = required(locked.sha512, "vendor SHA-512");
      if (!/^[a-f0-9]{128}$/.test(sha512)) {
        throw new Error(`invalid vendor SHA-512 for ${locked.url}`);
      }
      const archive = await downloadVerified(cacheRoot, locked.url, "sha512", sha512);
      return {
        path: archive,
        format: "tar_gz",
        entrypoint: locked.entrypoint,
        vendorUrl: locked.url,
        vendorSha256: required(locked.sha256, "vendor SHA-256"),
      };
    }
    case "npm_archive": {
      const locked = requiredTarget(recipe.targets, target, dependency.id);
      const archive = await downloadSRI(cacheRoot, 
        locked.url,
        required(locked.integrity, "npm integrity"),
      );
      const output = `${destination}/${command}.tar.gz`;
      await copyFile(archive, output);
      return { path: output, format: "tar_gz", entrypoint: locked.entrypoint };
    }
    case "npm_brotli": {
      const locked = requiredTarget(recipe.targets, target, dependency.id);
      const archive = await downloadSRI(cacheRoot, 
        locked.url,
        required(locked.integrity, "npm integrity"),
      );
      const stage = await mkdtemp(join(temporary, `${command}-`));
      await run("tar", ["-xzf", archive, "-C", stage, locked.entrypoint]);
      await mkdir(`${stage}/bin`, { recursive: true });
      await run("brotli", [
        "--decompress",
        `--output=${stage}/bin/${command}`,
        `${stage}/${locked.entrypoint}`,
      ]);
      await chmod(`${stage}/bin/${command}`, 0o755);
      await rm(`${stage}/package`, { recursive: true });
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
    case "node_runtime": {
      if (
        recipe.version !== runtimeLock.node.version ||
        dependency.source !== `https://nodejs.org/dist/v${runtimeLock.node.version}/` ||
        dependency.integrity !== nodeRuntimeIntegrity(runtimeLock.node)
      ) {
        throw new Error(`Node.js runtime source mismatch for ${dependency.id}`);
      }
      const node = requiredTarget(runtimeLock.node.targets, target, "Node.js");
      const nodeArchive = await downloadSha256(cacheRoot, 
        node.url,
        required(node.sha256, "Node SHA-256"),
      );
      const stage = await mkdtemp(join(temporary, `${command}-`));
      await run("tar", [
        "-xzf",
        nodeArchive,
        "-C",
        stage,
        "--strip-components=2",
        node.entrypoint,
      ]);
      const output = `${await realpath(destination)}/${command}`;
      await copyFile(`${stage}/node`, output);
      await chmod(output, 0o755);
      return { path: output, format: "raw" };
    }
    case "node_npm": {
      const stage = await mkdtemp(join(temporary, `${command}-`));
      if (recipe.bundle_node !== false) {
        const node = requiredTarget(runtimeLock.node.targets, target, "Node.js");
        const nodeArchive = await downloadSha256(cacheRoot, 
          node.url,
          required(node.sha256, "Node SHA-256"),
        );
        await mkdir(`${stage}/runtime`, { recursive: true });
        await run("tar", [
          "-xzf",
          nodeArchive,
          "-C",
          `${stage}/runtime`,
          "--strip-components=2",
          node.entrypoint,
        ]);
      }
      await mkdir(`${stage}/app`, { recursive: true });
      await copyFile(
        join(runtimeRoot, recipe.package_dir, "package.json"),
        `${stage}/app/package.json`,
      );
      await copyFile(
        join(runtimeRoot, recipe.package_dir, "package-lock.json"),
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
        ...(recipe.include_optional === true ? [] : ["--omit=optional"]),
        "--no-audit",
        "--no-fund",
        `--os=${npmOs}`,
        `--cpu=${npmCpu}`,
      ], { cwd: `${stage}/app` });
      await rm(`${stage}/app/node_modules/.bin`, { recursive: true })
        .catch((error) => {
          if (!((error as { code?: string }).code === "ENOENT")) throw error;
        });
      await mkdir(`${stage}/bin`, { recursive: true });
      const nodeResolver = recipe.bundle_node === false
        ? 'if [ -x "$cowboy_root/runtime/node" ]; then\n' +
          '  cowboy_node="$cowboy_root/runtime/node"\n' +
          'elif [ -x "$cowboy_root/../../acp_runtime-node/bin" ]; then\n' +
          '  cowboy_node="$cowboy_root/../../acp_runtime-node/bin"\n' +
          'else\n' +
          '  echo "Cowboy Node.js runtime missing" >&2\n' +
          '  exit 127\n' +
          'fi\n'
        : 'cowboy_node="$cowboy_root/runtime/node"\n';
      if (recipe.launcher) {
        if (!/^packages\/[a-z0-9-]+\/launch\.mjs$/.test(recipe.launcher)) {
          throw new Error("unsafe private launcher source");
        }
        await copyOwnedNode(runtimeRoot, recipe, dependency, `${stage}/app`);
        await writeFile(
          `${stage}/bin/cowboy-configured-cli`,
          '#!/bin/sh\nset -eu\ncase "$0" in */*) cowboy_dir=${0%/*} ;; *) cowboy_dir=. ;; esac\ncowboy_root=$(CDPATH= cd -- "$cowboy_dir/.." && pwd)\n' +
            nodeResolver +
            'exec "$cowboy_node" "$cowboy_root/app/cowboy-launch.mjs" --cowboy-private-cli "$@"\n',
          { mode: 0o755 },
        );
      }
      await writeFile(
        `${stage}/bin/${command}`,
        `#!/bin/sh\nset -eu\ncase "$0" in */*) cowboy_dir=\${0%/*} ;; *) cowboy_dir=. ;; esac\ncowboy_root=$(CDPATH= cd -- "$cowboy_dir/.." && pwd)\n${nodeResolver}exec "$cowboy_node" "$cowboy_root/app/${
          recipe.launcher ? "cowboy-launch.mjs" : recipe.script
        }" "$@"\n`,
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
      const stage = await mkdtemp(join(temporary, `${command}-`));
      const sourceRoot = await materializeGitSource(recipe, stage);
      const goArchitecture = target.endsWith("aarch64") ? "arm64" : "amd64";
      const goOperatingSystem = target.startsWith("linux-")
        ? "linux"
        : target.startsWith("macos-")
        ? "darwin"
        : undefined;
      if (!goOperatingSystem) {
        throw new Error(`${dependency.id} has no ${target} build contract`);
      }
      const files = recipe.files ?? [];
      const stage2 = files.length > 0
        ? await mkdtemp(join(temporary, `${command}-files-`))
        : undefined;
      if (stage2) await mkdir(`${stage2}/bin`, { recursive: true });
      const output = stage2
        ? `${stage2}/bin/${command}`
        : `${await realpath(destination)}/${command}`;
      await run("nix", [
        "develop",
        `path:${sourceRoot}`,
        "-c",
        "env",
        "CGO_ENABLED=0",
        `GOOS=${goOperatingSystem}`,
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
      await chmod(output, 0o755);
      if (!stage2) return { path: output, format: "raw" };
      // Data the component reads at run time (e.g. a model catalog) ships
      // beside its command from the same digest-verified source tree.
      for (const file of files) {
        if (!/^[a-z0-9][a-z0-9._-]*$/.test(file) || file === command) {
          throw new Error(`unsafe component file ${file}`);
        }
        const source = join(sourceRoot, file);
        if (!(await lstat(source)).isFile()) {
          throw new Error(`component file ${file} is not a regular source file`);
        }
        await copyFile(source, `${stage2}/bin/${file}`);
        await chmod(`${stage2}/bin/${file}`, 0o444);
      }
      await rejectLinks(stage2);
      const archive = `${destination}/${command}.tar.gz`;
      await deterministicTarGz(stage2, archive);
      return { path: archive, format: "tar_gz", entrypoint: `bin/${command}` };
    }
  }
}

async function materializeGitSource(
  recipe: GitGoRecipe,
  stage: string,
): Promise<string> {
  const repository = `${stage}/repository`;
  await run("git", [
    "clone",
    "--filter=blob:none",
    "--no-checkout",
    recipe.repository,
    repository,
  ]);
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
  await mkdir(extracted, { recursive: true });
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
  await writeFile(
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


import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  copyOwnedNode,
  nodeRuntimeIntegrity,
  ownedArchiveIntegrity,
  type OwnedNodeRecipe,
} from "./owned-sources.ts";
import {
  deterministicTarGz,
  downloadSha256,
  downloadSRI,
  downloadVerified,
  probePlatformArtifacts,
  readJson,
  rejectLinks,
  required,
  requiredTarget,
  run,
  sha256,
  validateComponentRef,
} from "./build-helpers.ts";

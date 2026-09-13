// Codex-owned source patching. The shared component still builds the exact
// upstream runtime graph; only unpublished candidate archives are replaced.
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const pluginRoot = resolve(root, "..");
const repository = resolve(root, "../../..");
const baseUrl = (Deno.args[0] ?? "").replace(/\/+$/, "");
if (!baseUrl.startsWith("https://") || baseUrl.includes("latest")) {
  throw new Error("Provider artifact base URL must be immutable HTTPS");
}
const source = JSON.parse(await Deno.readTextFile(`${root}/source.json`));
const manifest = JSON.parse(
  await Deno.readTextFile(`${pluginRoot}/provider.json`),
);
const adapter = source.adapter;
const dependency = manifest.runtime.dependencies.find((value: { id: string }) =>
  value.id === "codex-acp"
);
if (source.schema_version !== 1 || adapter.version !== dependency?.version) {
  throw new Error("Codex adapter source does not match the Provider pin");
}
if (
  !/^[a-f0-9]{40}$/.test(adapter.revision) ||
  adapter.archive_url !==
    `https://codeload.github.com/agentclientprotocol/codex-acp/tar.gz/${adapter.revision}` ||
  adapter.patch !== "adapter.patch"
) throw new Error("Invalid pinned Codex adapter source");

const output = `${repository}/dist/plugins/codex/runtime`;
const cache = `${repository}/dist/plugins/.runtime-cache`;
await Deno.mkdir(cache, { recursive: true });
const archive = `${cache}/codex-acp-${adapter.archive_sha256}.tar.gz`;
try {
  await Deno.stat(archive);
} catch (error) {
  if (!(error instanceof Deno.errors.NotFound)) throw error;
  const partial = `${archive}.${Deno.pid}.partial`;
  await run("curl", [
    "--fail",
    "--location",
    "--output",
    partial,
    adapter.archive_url,
  ]);
  await verify(partial, adapter.archive_sha256);
  await Deno.rename(partial, archive);
}
await verify(archive, adapter.archive_sha256);
const stage = await Deno.makeTempDir({ dir: cache, prefix: "codex-source-" });
const matrixPath = `${output}/runtime-artifacts.json`;
let bindingComplete = false;
try {
  const sourceRoot = `${stage}/source`;
  await Deno.mkdir(sourceRoot);
  await run("tar", ["-xzf", archive, "--strip-components=1", "-C", sourceRoot]);
  await verify(`${sourceRoot}/package-lock.json`, adapter.npm_lock_sha256);
  await run("patch", [
    "--batch",
    "--fuzz=0",
    "-p1",
    "-i",
    `${root}/adapter.patch`,
  ], sourceRoot);
  await run(
    "npm",
    ["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
    sourceRoot,
  );
  await run("npm", ["run", "typecheck"], sourceRoot);
  await run("npm", ["test"], sourceRoot);
  await run("npm", ["run", "build"], sourceRoot);

  // Preserve the existing component's download verification, exact Node pins,
  // target matrix, archive checks, and unmodified native CLI artifacts.
  await run(Deno.execPath(), [
    "run",
    "--allow-read",
    "--allow-write=dist",
    "--allow-net",
    "--allow-run",
    "components/provider-runtime/build.ts",
    "plugins/codex",
    baseUrl,
  ], repository);
  const matrix = JSON.parse(await Deno.readTextFile(matrixPath));
  // The shared builder emits an upstream binding. It must not remain bindable
  // if any later source patch, target repack, or probe fails.
  await Deno.remove(matrixPath);
  for (const target of matrix) {
    const component = target.components.find((value: { dependency: string }) =>
      value.dependency === "codex-acp"
    );
    if (!component || component.version !== adapter.version) {
      throw new Error("Missing exact Codex adapter target");
    }
    const targetName = `${target.os}-${target.architecture}`;
    const targetRoot = `${stage}/${targetName}`;
    const targetArchive = `${output}/${targetName}/codex-acp.tar.gz`;
    await Deno.mkdir(targetRoot);
    await run("tar", ["-xzf", targetArchive, "-C", targetRoot]);
    await Deno.copyFile(
      `${sourceRoot}/dist/index.js`,
      `${targetRoot}/app/node_modules/@agentclientprotocol/codex-acp/dist/index.js`,
    );
    await Deno.copyFile(
      `${root}/launch.mjs`,
      `${targetRoot}/app/cowboy-launch.mjs`,
    );
    await Deno.remove(`${targetRoot}/bin/cowboy-configured-cli`);
    const provenance = {
      schema: "cowboy.codex-source-patch/v1",
      upstream: adapter,
      patch_sha256: await sha256(`${root}/adapter.patch`),
      launcher_sha256: await sha256(`${root}/launch.mjs`),
      bundle_sha256: await sha256(`${sourceRoot}/dist/index.js`),
    };
    await Deno.writeTextFile(
      `${targetRoot}/codex-source.json`,
      JSON.stringify(provenance, null, 2) + "\n",
    );
    await Deno.copyFile(
      `${root}/adapter.patch`,
      `${targetRoot}/codex-adapter.patch`,
    );
    const names = [...Deno.readDirSync(targetRoot)].map((entry) => entry.name)
      .sort();
    await run("tar", [
      "--sort=name",
      "--mtime=@0",
      "--owner=0",
      "--group=0",
      "--numeric-owner",
      "--format=gnu",
      "-cf",
      `${targetRoot}.tar`,
      "-C",
      targetRoot,
      ...names,
    ]);
    await run("gzip", ["-n", "-9", "-f", `${targetRoot}.tar`]);
    await Deno.rename(`${targetRoot}.tar.gz`, targetArchive);
    const digest = await sha256(targetArchive);
    component.artifact_digest = `sha256:${digest}`;
    component.artifact_url = `${baseUrl}/${digest}/codex-acp.tar.gz`;
    if (targetName === "linux-x86_64") {
      await run(`${targetRoot}/bin/codex-acp`, ["--version"]);
    }
  }
  // Publish the binding only after every patched target was built successfully.
  const candidateMatrix = `${stage}/runtime-artifacts.json`;
  await Deno.writeTextFile(
    candidateMatrix,
    JSON.stringify(matrix, null, 2) + "\n",
  );
  await Deno.rename(candidateMatrix, matrixPath);
  bindingComplete = true;
  console.log(
    JSON.stringify({
      provider: "codex",
      version: manifest.version,
      runtime_manifest: matrixPath,
    }),
  );
} finally {
  if (!bindingComplete) {
    await Deno.remove(matrixPath).catch((error: unknown) => {
      if (!(error instanceof Deno.errors.NotFound)) throw error;
    });
  }
  await Deno.remove(stage, { recursive: true });
}

async function sha256(path: string): Promise<string> {
  const result = await new Deno.Command("sha256sum", { args: [path] }).output();
  if (!result.success) throw new Error("Could not hash source artifact");
  return new TextDecoder().decode(result.stdout).split(/\s/)[0];
}

async function verify(path: string, expected: string): Promise<void> {
  if (!/^[a-f0-9]{64}$/.test(expected) || await sha256(path) !== expected) {
    throw new Error("Codex source artifact digest mismatch");
  }
}

async function run(
  command: string,
  args: string[],
  cwd = repository,
): Promise<void> {
  const status = await new Deno.Command(command, {
    args,
    cwd,
    stdin: "null",
    stdout: "inherit",
    stderr: "inherit",
  }).spawn().status;
  if (!status.success) throw new Error(`${command} failed (${status.code})`);
}

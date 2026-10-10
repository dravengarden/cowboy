// Codex-owned source patching. The shared component still builds the exact
// upstream runtime graph; only unpublished candidate archives are replaced.
import { readdirSync } from "node:fs";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const pluginRoot = resolve(root, "..");
const repository = resolve(root, "../../..");
const baseUrl = (process.argv.slice(2)[0] ?? "").replace(/\/+$/, "");
// Build and probe dependencies without inheriting the invoking Agent's private
// arguments, credentials, homes or environment endpoint. Preserve only tools
// and CA trust; a disposable home is assigned before package scripts run.
const buildEnvironment: Record<string, string> = {};
for (
  const key of [
    "PATH",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NIX_SSL_CERT_FILE",
    "TMPDIR",
    "TMP",
    "TEMP",
  ]
) {
  const value = process.env[key];
  if (value !== undefined) buildEnvironment[key] = value;
}
if (!baseUrl.startsWith("https://") || baseUrl.includes("latest")) {
  throw new Error("Provider artifact base URL must be immutable HTTPS");
}
const source = JSON.parse(await readFile(`${root}/source.json`, "utf8"));
const manifest = JSON.parse(
  await readFile(`${pluginRoot}/provider.json`, "utf8"),
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
await mkdir(cache, { recursive: true });
const archive = `${cache}/codex-acp-${adapter.archive_sha256}.tar.gz`;
try {
  await stat(archive);
} catch (error) {
  if (!((error as { code?: string }).code === "ENOENT")) throw error;
  const partial = `${archive}.${process.pid}.partial`;
  await run("curl", [
    "--fail",
    "--location",
    "--output",
    partial,
    adapter.archive_url,
  ]);
  await verify(partial, adapter.archive_sha256);
  await rename(partial, archive);
}
await verify(archive, adapter.archive_sha256);
const stage = await mkdtemp(join(cache, "codex-source-"));
buildEnvironment.HOME = `${stage}/home`;
buildEnvironment.XDG_CONFIG_HOME = `${stage}/home/.config`;
await mkdir(buildEnvironment.HOME);
const matrixPath = `${output}/runtime-artifacts.json`;
let bindingComplete = false;
try {
  const sourceRoot = `${stage}/source`;
  await mkdir(sourceRoot);
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
  await run(process.execPath, [
    "run",
    "--allow-read",
    "--allow-write=dist",
    "--allow-net",
    "--allow-run",
    "components/provider-runtime/build.ts",
    "plugins/codex",
    baseUrl,
  ], repository);
  const matrix = JSON.parse(await readFile(matrixPath, "utf8"));
  // The shared builder emits an upstream binding. It must not remain bindable
  // if any later source patch, target repack, or probe fails.
  await rm(matrixPath);
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
    await mkdir(targetRoot);
    await run("tar", ["-xzf", targetArchive, "-C", targetRoot]);
    await copyFile(
      `${sourceRoot}/dist/index.js`,
      `${targetRoot}/app/node_modules/@agentclientprotocol/codex-acp/dist/index.js`,
    );
    await copyFile(
      `${root}/launch.mjs`,
      `${targetRoot}/app/cowboy-launch.mjs`,
    );
    await copyFile(
      `${repository}/components/provider-runtime/packages/codex-acp/launch.mjs`,
      `${targetRoot}/app/cowboy-execution.mjs`,
    );
    await writeFile(
      `${targetRoot}/app/memory.mjs`,
      (await readFile(`${root}/memory.mjs`, "utf8")).replace(
        "@cowboy/memory-client",
        "./matrix-client.mjs",
      ),
    );
    await copyFile(
      `${repository}/components/memory-client/index.mjs`,
      `${targetRoot}/app/matrix-client.mjs`,
    );
    await rm(`${targetRoot}/bin/cowboy-configured-cli`);
    const provenance = {
      schema: "cowboy.codex-source-patch/v1",
      upstream: adapter,
      patch_sha256: await sha256(`${root}/adapter.patch`),
      launcher_sha256: await sha256(`${root}/launch.mjs`),
      memory_bridge_sha256: await sha256(`${targetRoot}/app/memory.mjs`),
      memory_client_sha256: await sha256(`${targetRoot}/app/matrix-client.mjs`),
      execution_bridge_sha256: await sha256(
        `${targetRoot}/app/cowboy-execution.mjs`,
      ),
      bundle_sha256: await sha256(`${sourceRoot}/dist/index.js`),
    };
    await writeFile(
      `${targetRoot}/codex-source.json`,
      JSON.stringify(provenance, null, 2) + "\n",
    );
    await copyFile(
      `${root}/adapter.patch`,
      `${targetRoot}/codex-adapter.patch`,
    );
    const names = [...readdirSync(targetRoot, { withFileTypes: true })].map((
      entry,
    ) => entry.name)
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
    await rename(`${targetRoot}.tar.gz`, targetArchive);
    const digest = await sha256(targetArchive);
    component.artifact_digest = `sha256:${digest}`;
    component.artifact_url = `${baseUrl}/${digest}/codex-acp.tar.gz`;
    if (targetName === "linux-x86_64") {
      await run(`${targetRoot}/bin/codex-acp`, ["--version"]);
    }
  }
  // Publish the binding only after every patched target was built successfully.
  const candidateMatrix = `${stage}/runtime-artifacts.json`;
  await writeFile(
    candidateMatrix,
    JSON.stringify(matrix, null, 2) + "\n",
  );
  await rename(candidateMatrix, matrixPath);
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
    await rm(matrixPath).catch((error: unknown) => {
      if (!((error as { code?: string }).code === "ENOENT")) throw error;
    });
  }
  await rm(stage, { recursive: true });
}

async function sha256(path: string): Promise<string> {
  const child = Bun.spawn({
    cmd: ["sha256sum", path],
    stdin: "ignore",
    stdout: "pipe",
    stderr: "ignore",
  });
  const output = await new Response(child.stdout).text();
  if (await child.exited !== 0) {
    throw new Error("Could not hash source artifact");
  }
  return output.split(/\s/)[0];
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
  const code = await Bun.spawn({
    cmd: [command, ...args],
    cwd,
    env: buildEnvironment,
    stdin: "ignore",
    stdout: "inherit",
    stderr: "inherit",
  }).exited;
  if (code !== 0) throw new Error(`${command} failed (${code})`);
}

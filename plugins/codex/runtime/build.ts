// Codex-owned source patching. The shared component still builds the exact
// upstream runtime graph; only unpublished candidate archives are replaced.
import { readdirSync } from "node:fs";
import { copyFile, cp, mkdir, mkdtemp, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
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
for (const key of [
  "PATH",
  "SSL_CERT_FILE",
  "SSL_CERT_DIR",
  "NIX_SSL_CERT_FILE",
  "TMPDIR",
  "TMP",
  "TEMP",
]) {
  const value = process.env[key];
  if (value !== undefined) buildEnvironment[key] = value;
}
if (!baseUrl.startsWith("https://") || baseUrl.includes("latest")) {
  throw new Error("Provider artifact base URL must be immutable HTTPS");
}
const manifest = JSON.parse(await readFile(`${pluginRoot}/provider.json`, "utf8"));
const dependency = manifest.runtime.dependencies.find(
  (value: { id: string }) => value.id === "codex-acp",
);
const output = `${repository}/dist/plugins/codex/runtime`;
const cache = `${repository}/dist/plugins/.runtime-cache`;
await mkdir(cache, { recursive: true });
const stage = await mkdtemp(join(cache, "codex-source-"));
buildEnvironment.HOME = `${stage}/home`;
buildEnvironment.XDG_CONFIG_HOME = `${stage}/home/.config`;
await mkdir(buildEnvironment.HOME);
// Upstream tests create a Codex home per case under os.tmpdir() and never
// remove it. A short private directory keeps socket paths valid and is removed
// with the stage.
const temporary = await mkdtemp(join(tmpdir(), "codex-build-"));
for (const key of ["TMPDIR", "TMP", "TEMP"]) buildEnvironment[key] = temporary;
const matrixPath = `${output}/runtime-artifacts.json`;
let bindingComplete = false;
try {
  const glue = `${stage}/glue`;
  await mkdir(glue);
  for (const name of ["package.json", "package-lock.json"]) {
    await copyFile(`${root}/${name}`, `${glue}/${name}`);
  }
  await run("npm", ["ci", "--ignore-scripts", "--no-audit", "--no-fund"], glue);
  // The pinned upstream release plus Cowboy's delta series (third_party/codex-acp).
  const sourceRoot = `${stage}/source`;
  const prepared = Bun.spawn({
    cmd: [process.execPath, "tools/third-party.ts", "prepare", "codex-acp", sourceRoot, cache],
    cwd: repository,
    env: buildEnvironment,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "inherit",
  });
  const preparedOutput = await new Response(prepared.stdout).text();
  if ((await prepared.exited) !== 0) throw new Error("Could not prepare the Codex adapter source");
  const { upstream, series } = JSON.parse(preparedOutput) as {
    upstream: {
      version: string;
      repository: string;
      tag: string;
      commit: string;
      archive_sha256: string;
    };
    series: Array<{ id: string; file: string; sha256: string }>;
  };
  if (upstream.version !== dependency?.version) {
    throw new Error("Codex adapter source does not match the Provider pin");
  }
  const deltas = `${stage}/codex-adapter.patch`;
  await writeFile(
    deltas,
    (
      await Promise.all(
        series.map((delta) =>
          readFile(`${repository}/third_party/codex-acp/patches/${delta.file}`, "utf8"),
        ),
      )
    ).join(""),
  );
  await run("npm", ["ci", "--ignore-scripts", "--no-audit", "--no-fund"], sourceRoot);
  await run("npm", ["run", "typecheck"], sourceRoot);
  await run("npm", ["test"], sourceRoot);
  await run("npm", ["run", "build"], sourceRoot);

  // Preserve the existing component's download verification, exact Node pins,
  // target matrix, archive checks, and unmodified native CLI artifacts.
  await run(
    process.execPath,
    [
      "run",
      "--allow-read",
      "--allow-write=dist",
      "--allow-net",
      "--allow-run",
      "components/provider-runtime/build.ts",
      "plugins/codex",
      baseUrl,
    ],
    repository,
  );
  const matrix = JSON.parse(await readFile(matrixPath, "utf8"));
  // The shared builder emits an upstream binding. It must not remain bindable
  // if any later source patch, target repack, or probe fails.
  await rm(matrixPath);
  for (const target of matrix) {
    const component = target.components.find(
      (value: { dependency: string }) => value.dependency === "codex-acp",
    );
    if (!component || component.version !== upstream.version) {
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
    await copyFile(`${root}/launch.mjs`, `${targetRoot}/app/cowboy-launch.mjs`);
    await copyFile(`${root}/execution-mcp.mjs`, `${targetRoot}/app/execution-mcp.mjs`);
    await writeFile(
      `${targetRoot}/app/bindings.mjs`,
      (await readFile(`${root}/bindings.mjs`, "utf8")).replaceAll(
        '"../../../components/provider-runtime/',
        '"./',
      ),
    );
    for (const file of ["mcp-bindings.mjs", "execution-probe.mjs", "execution-connection.mjs"]) {
      await copyFile(
        `${repository}/components/provider-runtime/${file}`,
        `${targetRoot}/app/${file}`,
      );
    }
    await cp(`${glue}/node_modules/ws`, `${targetRoot}/app/node_modules/ws`, {
      recursive: true,
    });
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
      schema: "cowboy.codex-source-patch/v2",
      upstream: {
        repository: upstream.repository,
        tag: upstream.tag,
        commit: upstream.commit,
        archive_sha256: upstream.archive_sha256,
      },
      deltas: series.map((delta) => ({ id: delta.id, file: delta.file, sha256: delta.sha256 })),
      patch_sha256: await sha256(deltas),
      launcher_sha256: await sha256(`${root}/launch.mjs`),
      memory_bridge_sha256: await sha256(`${targetRoot}/app/memory.mjs`),
      binding_launcher_sha256: await sha256(`${targetRoot}/app/bindings.mjs`),
      binding_plan_sha256: await sha256(`${targetRoot}/app/mcp-bindings.mjs`),
      binding_probe_sha256: await sha256(`${targetRoot}/app/execution-probe.mjs`),
      execution_connection_sha256: await sha256(`${targetRoot}/app/execution-connection.mjs`),
      mcp_process_bridge_sha256: await sha256(`${targetRoot}/app/execution-mcp.mjs`),
      glue_lock_sha256: await sha256(`${root}/package-lock.json`),
      memory_client_sha256: await sha256(`${targetRoot}/app/matrix-client.mjs`),
      execution_bridge_sha256: await sha256(`${targetRoot}/app/cowboy-execution.mjs`),
      bundle_sha256: await sha256(`${sourceRoot}/dist/index.js`),
    };
    await writeFile(`${targetRoot}/codex-source.json`, JSON.stringify(provenance, null, 2) + "\n");
    await copyFile(deltas, `${targetRoot}/codex-adapter.patch`);
    const names = [...readdirSync(targetRoot, { withFileTypes: true })]
      .map((entry) => entry.name)
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
  await writeFile(candidateMatrix, JSON.stringify(matrix, null, 2) + "\n");
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
  try {
    if (!bindingComplete) {
      await rm(matrixPath).catch((error: unknown) => {
        if (!((error as { code?: string }).code === "ENOENT")) throw error;
      });
    }
    await rm(stage, { recursive: true });
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function sha256(path: string): Promise<string> {
  const child = Bun.spawn({
    cmd: ["sha256sum", path],
    stdin: "ignore",
    stdout: "pipe",
    stderr: "ignore",
  });
  const output = await new Response(child.stdout).text();
  if ((await child.exited) !== 0) {
    throw new Error("Could not hash source artifact");
  }
  return output.split(/\s/)[0];
}

async function run(command: string, args: string[], cwd = repository): Promise<void> {
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

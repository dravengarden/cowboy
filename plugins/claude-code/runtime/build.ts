// The existing Agent Plugin owns its native integration. All upstream binaries
// and npm dependencies still pass the shared immutable runtime builder.
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const repository = resolve(root, "../../..");
const base = (Deno.args[0] ?? "").replace(/\/+$/, "");
if (!base.startsWith("https://") || base.includes("latest")) {
  throw new Error("Immutable HTTPS base required");
}
const output = `${repository}/dist/plugins/claude-code/runtime`;
const matrixPath = `${output}/runtime-artifacts.json`;
const manifest = JSON.parse(
  await Deno.readTextFile(`${root}/../provider.json`),
);
if (
  manifest.runtime.dependencies.find((item: { id: string }) =>
    item.id === "anthropic-claude-code"
  )?.version !== "2.1.287"
) {
  throw new Error(
    "Claude execution requires the accepted exact native version",
  );
}
const environment: Record<string, string> = {};
for (
  const name of [
    "PATH",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NIX_SSL_CERT_FILE",
    "TMPDIR",
  ]
) {
  const value = Deno.env.get(name);
  if (value !== undefined) environment[name] = value;
}
const stage = await Deno.makeTempDir({
  dir: `${repository}/dist`,
  prefix: "claude-runtime-",
});
environment.HOME = `${stage}/home`;
environment.XDG_CONFIG_HOME = `${stage}/home/.config`;
await Deno.mkdir(environment.HOME);
let complete = false;
try {
  await run(Deno.execPath(), [
    "run",
    "--allow-read",
    "--allow-write=dist",
    "--allow-net",
    "--allow-run",
    "components/provider-runtime/build.ts",
    "plugins/claude-code",
    base,
  ], repository);
  const matrix = JSON.parse(await Deno.readTextFile(matrixPath));
  await Deno.remove(matrixPath);
  for (const target of matrix) {
    const name = `${target.os}-${target.architecture}`;
    const destination = `${stage}/${name}`;
    const archive = `${output}/${name}/claude-agent-acp.tar.gz`;
    await Deno.mkdir(destination);
    await run("tar", ["-xzf", archive, "-C", destination]);
    const sources = [
      "launch.mjs",
      "connection.mjs",
      "mod-bridge.mjs",
      "tools.mjs",
      "read-range.mjs",
      "context-mod.js",
      "hook-proxy.mjs",
      "task-wait.mjs",
      "instructions.mjs",
      "skills.mjs",
      "memory.mjs",
    ];
    const digests: Record<string, string> = {};
    for (const source of sources) {
      const filename = source === "launch.mjs" ? "cowboy-launch.mjs" : source;
      await Deno.copyFile(
        `${root}/${source}`,
        `${destination}/app/${filename}`,
      );
      digests[source] = await sha256(`${root}/${source}`);
    }
    await Deno.writeTextFile(
      `${destination}/app/memory.mjs`,
      (await Deno.readTextFile(`${root}/memory.mjs`)).replace(
        "@cowboy/memory-client",
        "./matrix-client.mjs",
      ),
    );
    await Deno.copyFile(
      `${repository}/components/memory-client/index.mjs`,
      `${destination}/app/matrix-client.mjs`,
    );
    digests["matrix-client.mjs"] = await sha256(
      `${destination}/app/matrix-client.mjs`,
    );
    const prefix =
      '#!/bin/sh\nset -eu\ncowboy_dir=${0%/*}\ncowboy_root=$(CDPATH= cd -- "$cowboy_dir/.." && pwd)\n';
    await Deno.writeTextFile(
      `${destination}/bin/claude-agent-acp`,
      prefix +
        'exec "$cowboy_root/runtime/node" "$cowboy_root/app/cowboy-launch.mjs" "$@"\n',
      { mode: 0o755 },
    );
    await Deno.writeTextFile(
      `${destination}/bin/cowboy-configured-cli`,
      prefix +
        'exec "$cowboy_root/runtime/node" "$cowboy_root/app/cowboy-launch.mjs" --cowboy-private-cli "$@"\n',
      { mode: 0o755 },
    );
    await Deno.writeTextFile(
      `${destination}/claude-execution.json`,
      JSON.stringify(
        {
          schema: "cowboy.claude-execution/v1",
          native_version: "2.1.287",
          sources: digests,
        },
        null,
        2,
      ) + "\n",
    );
    const names = [...Deno.readDirSync(destination)].map((entry) => entry.name)
      .sort();
    await run("tar", [
      "--sort=name",
      "--mtime=@0",
      "--owner=0",
      "--group=0",
      "--numeric-owner",
      "--format=gnu",
      "-cf",
      `${destination}.tar`,
      "-C",
      destination,
      ...names,
    ]);
    await run("gzip", ["-n", "-9", "-f", `${destination}.tar`]);
    await Deno.rename(`${destination}.tar.gz`, archive);
    const digest = await sha256(archive);
    const component = target.components.find((item: { dependency: string }) =>
      item.dependency === "claude-agent-acp"
    );
    if (!component) throw new Error("Claude adapter component missing");
    component.artifact_digest = `sha256:${digest}`;
    component.artifact_url = `${base}/${digest}/claude-agent-acp.tar.gz`;
    if (name === "linux-x86_64") {
      await run(`${destination}/bin/claude-agent-acp`, ["--version"]);
    }
  }
  await Deno.writeTextFile(matrixPath, JSON.stringify(matrix, null, 2) + "\n");
  complete = true;
} finally {
  if (!complete) {
    await Deno.remove(matrixPath).catch((error) => {
      if (!(error instanceof Deno.errors.NotFound)) throw error;
    });
  }
  await Deno.remove(stage, { recursive: true });
}

async function run(command: string, args: string[], cwd = repository) {
  const result = await new Deno.Command(command, {
    args,
    cwd,
    env: environment,
    clearEnv: true,
    stdin: "null",
  }).spawn().status;
  if (!result.success) throw new Error(`${command} failed (${result.code})`);
}
async function sha256(path: string) {
  const digest = new Uint8Array(
    await crypto.subtle.digest("SHA-256", await Deno.readFile(path)),
  );
  return [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

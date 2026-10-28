// The existing Agent Plugin owns its native integration. All upstream binaries
// and npm dependencies still pass the shared immutable runtime builder.
import { readdirSync } from "node:fs";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
// One arena and a one-second dirty decay: on OVH (Node 24, a transcript-like
// stream through the real adapter modules) 45 -> 34 MiB anonymous memory at
// +6% CPU; a zero decay saved 2 MiB more for +31% CPU. No abort_conf: an
// older host library warns about an option instead of failing the launch.
const JEMALLOC_CONF =
  "background_thread:true,narenas:1,dirty_decay_ms:1000,muzzy_decay_ms:0,thp:never";
const repository = resolve(root, "../../..");
const base = (process.argv.slice(2)[0] ?? "").replace(/\/+$/, "");
if (!base.startsWith("https://") || base.includes("latest")) {
  throw new Error("Immutable HTTPS base required");
}
const output = `${repository}/dist/plugins/claude-code/runtime`;
const matrixPath = `${output}/runtime-artifacts.json`;
const manifest = JSON.parse(
  await readFile(`${root}/../provider.json`, "utf8"),
);
if (
  manifest.runtime.dependencies.find((item: { id: string }) =>
    item.id === "anthropic-claude-code"
  )?.version !== "2.1.312"
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
  const value = process.env[name];
  if (value !== undefined) environment[name] = value;
}
const stage = await mkdtemp(join(`${repository}/dist`, "claude-runtime-"));
environment.HOME = `${stage}/home`;
environment.XDG_CONFIG_HOME = `${stage}/home/.config`;
await mkdir(environment.HOME);
let complete = false;
try {
  await run(process.execPath, [
    "run",
    "--allow-read",
    "--allow-write=dist",
    "--allow-net",
    "--allow-run",
    "components/provider-runtime/build.ts",
    "plugins/claude-code",
    base,
  ], repository);
  // The native MCP relay (relay/): a static Linux executable. Other targets
  // keep the Node relay, which launch.mjs selects when it is absent.
  const relayBuild = JSON.parse(
    await output_of("nix", [
      "build",
      "--no-link",
      "--json",
      ".#cowboy-mcp-relay-portable",
    ]),
  );
  const relay = `${relayBuild[0].outputs.out}/bin/cowboy-mcp-relay`;
  const matrix = JSON.parse(await readFile(matrixPath, "utf8"));
  await rm(matrixPath);
  for (const target of matrix) {
    const name = `${target.os}-${target.architecture}`;
    const destination = `${stage}/${name}`;
    const archive = `${output}/${name}/claude-agent-acp.tar.gz`;
    await mkdir(destination);
    await run("tar", ["-xzf", archive, "-C", destination]);
    const sources = [
      "launch.mjs",
      "steering.mjs",
      "connection.mjs",
      "mod-bridge.mjs",
      "tools.mjs",
      "context-mod.js",
      "hook-proxy.mjs",
      "task-wait.mjs",
      "instructions.mjs",
      "skills.mjs",
      "mcp.mjs",
      "mcp-proxy.mjs",
      "memory.mjs",
      "managed.mjs",
      "agent-calls.mjs",
    ];
    const digests: Record<string, string> = {};
    for (const source of sources) {
      const filename = source === "launch.mjs" ? "cowboy-launch.mjs" : source;
      // The packaged app has no node_modules: every module importing the
      // memory client (memory.mjs, agent-calls.mjs) reads the bundled copy.
      await writeFile(
        `${destination}/app/${filename}`,
        (await readFile(`${root}/${source}`, "utf8")).replaceAll(
          '"@cowboy/memory-client"',
          '"./matrix-client.mjs"',
        ),
      );
      digests[source] = await sha256(`${root}/${source}`);
    }
    await copyFile(
      `${repository}/components/memory-client/index.mjs`,
      `${destination}/app/matrix-client.mjs`,
    );
    digests["matrix-client.mjs"] = await sha256(
      `${destination}/app/matrix-client.mjs`,
    );
    // Where the host ships jemalloc (Ubuntu's libjemalloc2 on OVH), the two
    // resident Node processes return freed native memory within a second
    // instead of keeping glibc's fragmented peak. launch.mjs removes the
    // preload again, so nothing either process starts inherits it.
    const prefix =
      '#!/bin/sh\nset -eu\ncowboy_dir=${0%/*}\ncowboy_root=$(CDPATH= cd -- "$cowboy_dir/.." && pwd)\n' +
      'if [ -z "${LD_PRELOAD-}" ] && [ -z "${MALLOC_CONF-}" ]; then\n' +
      "  for cowboy_jemalloc in /usr/lib/x86_64-linux-gnu/libjemalloc.so.2 /usr/lib/aarch64-linux-gnu/libjemalloc.so.2; do\n" +
      '    if [ -r "$cowboy_jemalloc" ]; then\n' +
      `      LD_PRELOAD=$cowboy_jemalloc MALLOC_CONF=${JEMALLOC_CONF} COWBOY_NODE_ALLOCATOR=jemalloc\n` +
      "      export LD_PRELOAD MALLOC_CONF COWBOY_NODE_ALLOCATOR\n" +
      "      break\n" +
      "    fi\n" +
      "  done\n" +
      "fi\n";
    await writeFile(
      `${destination}/bin/claude-agent-acp`,
      prefix +
        'exec "$cowboy_root/runtime/node" --optimize-for-size "$cowboy_root/app/cowboy-launch.mjs" "$@"\n',
      { mode: 0o755 },
    );
    await writeFile(
      `${destination}/bin/cowboy-configured-cli`,
      prefix +
        'exec "$cowboy_root/runtime/node" --optimize-for-size "$cowboy_root/app/cowboy-launch.mjs" --cowboy-private-cli "$@"\n',
      { mode: 0o755 },
    );
    if (name === "linux-x86_64") {
      await copyFile(relay, `${destination}/bin/cowboy-mcp-relay`);
      await chmod(`${destination}/bin/cowboy-mcp-relay`, 0o755);
      digests["cowboy-mcp-relay"] = await sha256(relay);
      await run(`${destination}/bin/cowboy-mcp-relay`, ["--version"]);
    }
    await writeFile(
      `${destination}/claude-execution.json`,
      JSON.stringify(
        {
          schema: "cowboy.claude-execution/v1",
          native_version: "2.1.312",
          sources: digests,
        },
        null,
        2,
      ) + "\n",
    );
    const names = [...readdirSync(destination, { withFileTypes: true })].map((
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
      `${destination}.tar`,
      "-C",
      destination,
      ...names,
    ]);
    await run("gzip", ["-n", "-9", "-f", `${destination}.tar`]);
    await rename(`${destination}.tar.gz`, archive);
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
  await writeFile(matrixPath, JSON.stringify(matrix, null, 2) + "\n");
  complete = true;
} finally {
  if (!complete) {
    await rm(matrixPath).catch((error) => {
      if (!((error as { code?: string }).code === "ENOENT")) throw error;
    });
  }
  await rm(stage, { recursive: true });
}

async function run(command: string, args: string[], cwd = repository) {
  const code = await Bun.spawn({
    cmd: [command, ...args],
    cwd,
    env: environment,
    stdin: "ignore",
    stdout: "inherit",
    stderr: "inherit",
  }).exited;
  if (code !== 0) throw new Error(`${command} failed (${code})`);
}
async function output_of(command: string, args: string[]) {
  const child = Bun.spawn({
    cmd: [command, ...args],
    cwd: repository,
    env: environment,
    stdin: "ignore",
    stdout: "pipe",
    stderr: "inherit",
  });
  const text = await new Response(child.stdout).text();
  if (await child.exited !== 0) throw new Error(`${command} failed`);
  return text;
}
async function sha256(path: string) {
  const digest = new Uint8Array(
    await crypto.subtle.digest("SHA-256", await readFile(path)),
  );
  return [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

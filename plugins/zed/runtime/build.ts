// Package-owned, credential-free release builder. This is not a Machine
// install/update command and never touches the user's ordinary Zed instance.
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
interface CodeContract {
  schema_version: number;
  id: string;
  version: string;
  supported_platforms: Array<{ os: string; architecture: string }>;
  runtime: {
    components: Array<{
      kind: string;
      slot: string;
      dependency: string;
      version: string;
      command: string;
    }>;
  };
}

const base = new URL(process.argv.slice(2)[0] ?? "");
if (
  base.protocol !== "https:" || base.username || base.password || base.search ||
  base.hash || base.pathname.replace(/\/$/, "") !== "/plugin-artifacts"
) throw new Error("Zed artifact base must be an HTTPS /plugin-artifacts route");

if (process.platform !== "linux" || process.arch !== "x64") {
  throw new Error("The current Zed Plugin declares only Linux x86_64");
}
if ((await run("git", ["status", "--porcelain"])).trim()) {
  throw new Error(
    "Final Zed runtime builds require a clean committed worktree",
  );
}
const revision = (await run("git", ["rev-parse", "HEAD"])).trim();
const contract: CodeContract = JSON.parse(
  await readFile("plugins/zed/contract.json", "utf8"),
);
if (
  contract.schema_version !== 2 || contract.id !== "zed" ||
  contract.supported_platforms.length !== 1 ||
  contract.supported_platforms[0].os !== "linux" ||
  contract.supported_platforms[0].architecture !== "x86_64" ||
  contract.runtime.components.length !== 2
) throw new Error("Zed runtime builder does not cover this exact contract");

const recipes = [
  {
    command: "cowboy-zed-adapter",
    output: "cowboy-zed-adapter-portable",
    probe: ["--help"],
  },
  {
    command: "cowboy-zed-server",
    output: "cowboy-zed-server",
    probe: ["version"],
  },
];
const outputRoot = "dist/plugins/zed/runtime";
const target = `${outputRoot}/linux-x86_64`;
await mkdir(target, { recursive: true });
const temporary = await mkdtemp(join(tmpdir(), "cowboy-zed-runtime-probe-"));
try {
  const components = [];
  const builds = [];
  for (const recipe of recipes) {
    const requirement = contract.runtime.components.find((value) =>
      value.command === recipe.command
    );
    if (!requirement) {
      throw new Error(`Missing declared runtime ${recipe.command}`);
    }
    const build = JSON.parse(
      await run("nix", ["build", "--no-link", "--json", `.#${recipe.output}`]),
    );
    if (build.length !== 1 || typeof build[0].outputs.out !== "string") {
      throw new Error("Unexpected Nix build result");
    }
    const immutable = build[0].outputs.out as string;
    if (!immutable.endsWith(`-${requirement.version}`)) {
      throw new Error("Nix output does not match the declared runtime version");
    }
    const binary = `${immutable}/bin/${recipe.command}`;
    if (
      /\bINTERP\b/.test(await run("readelf", ["-l", binary])) ||
      /\bNEEDED\b/.test(await run("readelf", ["-d", binary]))
    ) {
      throw new Error(`${recipe.command} is not a standalone Linux executable`);
    }
    const destination = `${target}/${recipe.command}`;
    await copyFile(binary, destination);
    await chmod(destination, 0o700);
    const home = `${temporary}/${recipe.command}`;
    await mkdir(home, { mode: 0o700 });
    const child = Bun.spawn({
      cmd: [await realpath(destination), ...recipe.probe],
      cwd: home,
      env: { HOME: home, XDG_CONFIG_HOME: home, XDG_CACHE_HOME: home },
      stdin: "ignore",
      stdout: "ignore",
      stderr: "ignore",
    });
    const timer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch { /* already exited */ }
    }, 30_000);
    try {
      if (await child.exited !== 0) {
        throw new Error(`${recipe.command} failed its isolated probe`);
      }
    } finally {
      clearTimeout(timer);
    }
    const digest = hex(
      await crypto.subtle.digest("SHA-256", await readFile(destination)),
    );
    components.push({
      ...requirement,
      artifact_url: `${
        base.href.replace(/\/$/, "")
      }/${digest}/${recipe.command}`,
      artifact_digest: `sha256:${digest}`,
      artifact_format: "raw",
      probe: { args: recipe.probe, timeout_ms: 30_000 },
    });
    builds.push({
      command: recipe.command,
      output: immutable,
      artifact_digest: `sha256:${digest}`,
    });
  }
  const matrix = [{ os: "linux", architecture: "x86_64", components }];
  await writeFile(
    `${outputRoot}/runtime-artifacts.json`,
    `${JSON.stringify(matrix, null, 2)}\n`,
  );
  const receipt = {
    schema_version: 1,
    plugin_id: "zed",
    plugin_version: contract.version,
    source_revision: revision,
    builds,
  };
  await writeFile(
    `${outputRoot}/build-receipt.json`,
    `${JSON.stringify(receipt, null, 2)}\n`,
  );
  console.log(JSON.stringify(receipt));
} finally {
  await rm(temporary, { recursive: true });
}

async function run(command: string, args: string[]): Promise<string> {
  const child = Bun.spawn({
    cmd: [command, ...args],
    stdin: "ignore",
    stdout: "pipe",
    stderr: "inherit",
  });
  const output = await new Response(child.stdout).text();
  const code = await child.exited;
  if (code !== 0) throw new Error(`${command} exited ${code}`);
  return output;
}

function hex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)].map((value) =>
    value.toString(16).padStart(2, "0")
  ).join("");
}

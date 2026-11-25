import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  realpath,
  rename,
  rm,
} from "node:fs/promises";
import { join, relative } from "node:path";

export interface PrivateComponentRef {
  kind: string;
  slot: string;
  dependency: string;
  command: string;
}

const SDK_IDENTIFIER = /^[a-z0-9._-]{1,128}$/;
const ALLOWED_KINDS = new Set([
  "provider_cli",
  "provider_adapter",
  "provider_gateway",
  "acp_runtime",
]);
const ALLOWED_PROBE_ENV = [
  "PATH",
  "SSL_CERT_FILE",
  "SSL_CERT_DIR",
  "NIX_SSL_CERT_FILE",
  "LANG",
  "LC_ALL",
];

function isSdkIdentifier(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value !== "." &&
    value !== ".." &&
    SDK_IDENTIFIER.test(value)
  );
}

export function validateComponentRef(requirement: PrivateComponentRef): void {
  if (typeof requirement.kind !== "string" || !ALLOWED_KINDS.has(requirement.kind)) {
    throw new Error(`unsupported private component kind ${String(requirement.kind)}`);
  }
  if (!isSdkIdentifier(requirement.slot)) {
    throw new Error(`invalid private component slot ${String(requirement.slot)}`);
  }
  if (!isSdkIdentifier(requirement.dependency)) {
    throw new Error(`invalid private component dependency ${String(requirement.dependency)}`);
  }
  if (!isSdkIdentifier(requirement.command)) {
    throw new Error(`invalid private component command ${String(requirement.command)}`);
  }
}

function ensureWithinDir(parent: string, child: string, label: string): void {
  const rel = relative(parent, child);
  if (rel === "" || rel.startsWith("..") || rel.startsWith("/")) {
    throw new Error(`${label} escapes probe root`);
  }
}

function closedProbeEnv(probeRoot: string, home: string, tmp: string): Record<string, string> {
  const env: Record<string, string> = {
    HOME: home,
    XDG_CONFIG_HOME: `${home}/.config`,
    GROK_HOME: `${home}/.grok`,
    CODEX_HOME: `${home}/.codex`,
    CLAUDE_CONFIG_DIR: `${home}/.claude`,
    TMPDIR: tmp,
    TMP: tmp,
    TEMP: tmp,
  };
  for (const key of ALLOWED_PROBE_ENV) {
    const value = process.env[key];
    if (value !== undefined) env[key] = value;
  }
  // Ensure cwd-relative tools never inherit ambient credentials or profiles.
  void probeRoot;
  return env;
}

export async function probePlatformArtifacts(
  temporary: string,
  items: Array<{
    requirement: PrivateComponentRef;
    path: string;
    digest: string;
    format: "raw" | "tar_gz";
    entrypoint?: string;
    probe: string[];
  }>,
): Promise<void> {
  const nodeItem = items.find(
    (candidate) =>
      candidate.requirement.kind === "acp_runtime" &&
      candidate.requirement.slot === "node" &&
      candidate.format === "raw",
  );
  // Probe each component in its own fresh isolated tree and closed environment
  // while re-verifying every artifact against its recorded digest so an earlier
  // probe can never mutate bytes validated or published for another component.
  for (const item of items) {
    validateComponentRef(item.requirement);
    if (await sha256(item.path) !== item.digest) {
      throw new Error(`runtime artifact digest drifted before probe: ${item.path}`);
    }
    const probeRoot = await mkdtemp(join(temporary, "probe-"));
    try {
      const runtimeDir = await realpath(
        await mkdir(`${probeRoot}/runtime`, { recursive: true }).then(() => `${probeRoot}/runtime`),
      );
      if (nodeItem && nodeItem !== item) {
        if (await sha256(nodeItem.path) !== nodeItem.digest) {
          throw new Error(`Node runtime digest drifted before probe: ${nodeItem.path}`);
        }
        const nodeDir = join(runtimeDir, "acp_runtime-node");
        ensureWithinDir(runtimeDir, nodeDir, "acp_runtime-node");
        await mkdir(nodeDir, { recursive: true });
        const stagedNode = join(nodeDir, "bin");
        await copyFile(nodeItem.path, stagedNode);
        if (await sha256(stagedNode) !== nodeItem.digest) {
          throw new Error("staged Node runtime digest mismatch");
        }
        await chmod(stagedNode, 0o500);
      }
      const componentDir = join(
        runtimeDir,
        `${item.requirement.kind}-${item.requirement.slot}`,
      );
      ensureWithinDir(runtimeDir, componentDir, "component dir");
      await mkdir(componentDir, { recursive: true });
      let executable: string;
      if (item.format === "raw") {
        const stagedBin = join(componentDir, "bin");
        await copyFile(item.path, stagedBin);
        if (await sha256(stagedBin) !== item.digest) {
          throw new Error(`staged raw artifact digest mismatch: ${item.path}`);
        }
        executable = await realpath(stagedBin);
        ensureWithinDir(componentDir, executable, "resolved raw executable");
        await chmod(executable, 0o500);
      } else {
        const stagedArchive = join(probeRoot, "archive.tar.gz");
        await copyFile(item.path, stagedArchive);
        if (await sha256(stagedArchive) !== item.digest) {
          throw new Error(`staged archive digest mismatch: ${item.path}`);
        }
        const contentDir = await realpath(
          await mkdir(join(componentDir, "content"), { recursive: true }).then(() =>
            join(componentDir, "content")
          ),
        );
        await run("tar", ["-xzf", stagedArchive, "-C", contentDir]);
        await rejectLinks(contentDir);
        const entry = required(item.entrypoint, "archive entrypoint");
        if (entry.startsWith("/") || entry.split("/").some((s) => s === "" || s === "..")) {
          throw new Error(`unsafe archive entrypoint ${entry}`);
        }
        const candidate = join(contentDir, entry);
        ensureWithinDir(contentDir, candidate, "archive entrypoint");
        const entryStat = await lstat(candidate);
        if (!entryStat.isFile()) {
          throw new Error(`archive entrypoint is not a regular file: ${entry}`);
        }
        executable = await realpath(candidate);
        ensureWithinDir(contentDir, executable, "resolved archive entrypoint");
        await chmod(executable, 0o500);
      }
      const home = join(probeRoot, "home");
      const probeTmp = join(probeRoot, "tmp");
      await mkdir(home, { recursive: true });
      await mkdir(probeTmp, { recursive: true });
      const child = Bun.spawn({
        cmd: [executable, ...item.probe],
        cwd: probeRoot,
        env: closedProbeEnv(probeRoot, home, probeTmp),
        stdin: "ignore",
        stdout: "inherit",
        stderr: "inherit",
      });
      const timeout = setTimeout(() => child.kill("SIGKILL"), 30_000);
      const code = await child.exited;
      clearTimeout(timeout);
      if (code !== 0) throw new Error(`runtime probe failed for ${item.path}`);
    } finally {
      await rm(probeRoot, { recursive: true, force: true }).catch(() => undefined);
    }
  }
  // Confirm no probe altered any built artifact in the output directory.
  for (const item of items) {
    if (await sha256(item.path) !== item.digest) {
      throw new Error(`runtime artifact digest changed after probing: ${item.path}`);
    }
  }
}

export async function deterministicTarGz(
  stage: string,
  output: string,
): Promise<void> {
  const names = [];
  for (const entry of await readdir(stage, { withFileTypes: true })) {
    names.push(entry.name);
  }
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
  await rename(`${tarPath}.gz`, output);
}

export async function rejectLinks(root: string): Promise<void> {
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = `${root}/${entry.name}`;
    const metadata = await lstat(path);
    if (metadata.isSymbolicLink()) {
      throw new Error(`runtime archive contains symlink ${path}`);
    }
    if (metadata.isFile() && metadata.nlink > 1) {
      throw new Error(`runtime archive contains hardlink ${path}`);
    }
    if (metadata.isDirectory()) await rejectLinks(path);
  }
}

export async function downloadSRI(cacheRoot: string, url: string, integrity: string): Promise<string> {
  const match = /^sha512-([A-Za-z0-9+/]+={0,2})$/.exec(integrity);
  if (!match) throw new Error(`unsupported npm integrity for ${url}`);
  const expected = base64Hex(match[1]);
  return await downloadVerified(cacheRoot, url, "sha512", expected);
}

export async function downloadSha256(cacheRoot: string, url: string, expected: string): Promise<string> {
  if (!/^[a-f0-9]{64}$/.test(expected)) {
    throw new Error(`invalid SHA-256 for ${url}`);
  }
  return await downloadVerified(cacheRoot, url, "sha256", expected);
}

export async function downloadVerified(
  cacheRoot: string,
  url: string,
  algorithm: "sha256" | "sha512",
  expected: string,
): Promise<string> {
  const path = `${cacheRoot}/${algorithm}-${expected}`;
  if (!(await exists(path))) {
    const partial = `${path}.${process.pid}.partial`;
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
      await rm(partial).catch(() => undefined);
      throw new Error(`download digest mismatch for ${url}`);
    }
    await rename(partial, path).catch(async (error) => {
      if (!((error as { code?: string }).code === "EEXIST")) throw error;
      await rm(partial);
    });
  }
  const actual = await digestFile(path, algorithm);
  if (actual !== expected) {
    throw new Error(`cached download digest mismatch for ${url}`);
  }
  return path;
}

export async function sha256(path: string): Promise<string> {
  return await digestFile(path, "sha256");
}

async function digestFile(
  path: string,
  algorithm: "sha256" | "sha512",
): Promise<string> {
  const child = Bun.spawn({
    cmd: [`${algorithm}sum`, path],
    stdin: "ignore",
    stdout: "pipe",
    stderr: "ignore",
  });
  const output = await new Response(child.stdout).text();
  if (await child.exited !== 0) {
    throw new Error(`${algorithm}sum failed for ${path}`);
  }
  return output.trim().split(/\s+/)[0].toLowerCase();
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

export async function run(
  command: string,
  args: string[],
  options: { cwd?: string } = {},
): Promise<void> {
  const code = await Bun.spawn({
    cmd: [command, ...args],
    ...(options.cwd === undefined ? {} : { cwd: options.cwd }),
    stdin: "ignore",
    stdout: "inherit",
    stderr: "inherit",
  }).exited;
  if (code !== 0) throw new Error(`${command} exited ${code}`);
}

export async function readJson<T>(path: string): Promise<T> {
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

export function required<T>(value: T | undefined, label: string): T {
  if (value === undefined) throw new Error(`missing ${label}`);
  return value;
}

export function requiredTarget<T>(
  targets: Partial<Record<string, T>>,
  target: string,
  label: string,
): T {
  return required(targets[target], `${label} runtime lock for ${target}`);
}

if (import.meta.main && process.argv[2] === "--probe") {
  const specPath = process.argv[3];
  if (!specPath) throw new Error("missing probe spec path");
  const spec = await readJson<{
    temporary: string;
    items: Array<{
      requirement: PrivateComponentRef;
      path: string;
      digest: string;
      format: "raw" | "tar_gz";
      entrypoint?: string;
      probe: string[];
    }>;
  }>(specPath);
  await probePlatformArtifacts(spec.temporary, spec.items);
}

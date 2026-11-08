// The lock: the exact Ubuntu packages (URL, version, size, SHA-256) and
// Chrome .deb the artifact is assembled from, the contract they must agree
// with, and their verified download into the deb cache.
import { createHash } from "node:crypto";
import { open, rename, rm, stat, unlink } from "node:fs/promises";
import { join } from "node:path";

export interface Contract {
  schema_version: number;
  id: string;
  version: string;
  supported_platforms: Array<{ os: string; architecture: string }>;
  runtime: {
    artifact: string;
    launcher: string;
    browser: string;
    browser_version: string;
    display_server: string;
    helpers: string[];
    policy: { file: string; directory: string };
  };
  launch: { protocol: string; readiness_timeout_ms: number };
}

export interface LockedPackage {
  name: string;
  version: string;
  filename?: string;
  size: number;
  sha256: string;
  urls: string[];
}

export interface Lock {
  schema_version: number;
  generated_at: string;
  distribution: { id: string; version_id: string; architecture: string; mirror: string };
  browser: LockedPackage & { package: string; browser_version: string };
  requested: string[];
  excluded: Array<{ name: string; reason: string }>;
  packages: LockedPackage[];
}

const DOWNLOAD_CONCURRENCY = 6;

export function checkContract(contract: Contract, lock: Lock) {
  if (
    contract.schema_version !== 1 ||
    contract.id !== "machine-browser" ||
    contract.supported_platforms.length !== 1 ||
    contract.supported_platforms[0].os !== "linux" ||
    contract.supported_platforms[0].architecture !== "x86_64" ||
    contract.launch.protocol !== "cowboy-browser-control-v1"
  ) {
    throw new Error("The runtime builder does not cover this exact contract");
  }
  if (lock.schema_version !== 1 || lock.distribution.architecture !== "amd64") {
    throw new Error("Unsupported packages.lock.json");
  }
  if (contract.runtime.browser_version !== lock.browser.browser_version) {
    throw new Error(
      `contract.json pins browser ${contract.runtime.browser_version} but the lock holds ${lock.browser.browser_version}; rerun lock.sh`,
    );
  }
  for (const path of [
    contract.runtime.launcher,
    contract.runtime.browser,
    contract.runtime.display_server,
    contract.runtime.policy.file,
    ...contract.runtime.helpers,
  ]) {
    if (
      path.startsWith("/") ||
      path.split("/").some((part) => part === "" || part === "." || part === "..")
    ) {
      throw new Error(`contract path ${path} is not a normalized relative path`);
    }
  }
  const seen = new Set<string>();
  for (const locked of [...lock.packages, lock.browser]) {
    if (!/^[0-9a-f]{64}$/.test(locked.sha256)) throw new Error(`${locked.name}: invalid sha256`);
    if (seen.has(locked.sha256)) throw new Error(`${locked.name}: duplicate package digest`);
    seen.add(locked.sha256);
    if (!locked.urls.length || locked.urls.some((url) => !/^https?:\/\//.test(url))) {
      throw new Error(`${locked.name}: invalid download URLs`);
    }
  }
}

export async function downloadAll(
  packages: LockedPackage[],
  cache: string,
): Promise<Map<string, string>> {
  const paths = new Map<string, string>();
  const queue = [...packages];
  const workers = Array.from({ length: DOWNLOAD_CONCURRENCY }, async () => {
    while (queue.length) {
      const locked = queue.shift()!;
      paths.set(locked.sha256, await fetchPackage(locked, cache));
    }
  });
  await Promise.all(workers);
  return paths;
}

async function fetchPackage(locked: LockedPackage, cache: string): Promise<string> {
  const destination = join(cache, `${locked.sha256}.deb`);
  try {
    const info = await stat(destination);
    if (info.size === locked.size && (await sha256File(destination)) === locked.sha256) {
      return destination;
    }
    await unlink(destination);
  } catch {
    // not cached
  }
  const errors: string[] = [];
  for (const url of locked.urls) {
    try {
      const response = await fetch(url, { redirect: "follow" });
      if (!response.ok || !response.body) {
        errors.push(`${url}: HTTP ${response.status}`);
        continue;
      }
      const temporary = `${destination}.part-${process.pid}`;
      const hash = createHash("sha256");
      let size = 0;
      const handle = await open(temporary, "w", 0o644);
      try {
        for await (const chunk of response.body) {
          hash.update(chunk);
          size += chunk.length;
          if (size > locked.size) break;
          await handle.write(chunk);
        }
      } finally {
        await handle.close();
      }
      const digest = hash.digest("hex");
      if (size !== locked.size || digest !== locked.sha256) {
        await rm(temporary, { force: true });
        errors.push(
          `${url}: expected ${locked.size} bytes ${locked.sha256}, got ${size} bytes ${digest}`,
        );
        continue;
      }
      await rename(temporary, destination);
      return destination;
    } catch (error) {
      errors.push(`${url}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }
  throw new Error(
    `${locked.name} ${locked.version} could not be fetched:\n  ${errors.join("\n  ")}`,
  );
}

export async function sha256File(path: string): Promise<string> {
  const hash = createHash("sha256");
  for await (const chunk of Bun.file(path).stream()) {
    hash.update(chunk);
  }
  return hash.digest("hex");
}

export async function run(command: string, args: string[]): Promise<string> {
  const child = Bun.spawn({
    cmd: [command, ...args],
    stdin: "ignore",
    stdout: "pipe",
    stderr: "inherit",
  });
  const output = await new Response(child.stdout).text();
  const code = await child.exited;
  if (code !== 0) throw new Error(`${command} ${args.slice(0, 2).join(" ")} exited ${code}`);
  return output;
}

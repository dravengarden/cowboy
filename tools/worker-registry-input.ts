// Derive the component-registry input that can affect detached workers.
// A tail changing only the unconsumed Web app shell is not a worker rollout.

import { readFileSync, writeFileSync } from "node:fs";
interface Release extends Record<string, unknown> {
  version: string;
  components: Array<{ id: string; [key: string]: unknown }>;
  closure?: {
    component_dependencies: Record<string, string[]>;
    plugins: Record<string, { components: Array<{ id: string }> }>;
  };
}

interface Registry {
  schema_version: number;
  active_release: string;
  releases: Release[];
}

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${
      Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
        .map(([key, entry]) => `${JSON.stringify(key)}:${canonical(entry)}`)
        .join(",")
    }}`;
  }
  const encoded = JSON.stringify(value);
  if (encoded === undefined) throw new Error("invalid worker registry value");
  return encoded;
}

function workerRecord(release: Release): unknown {
  const record: Record<string, unknown> = { ...release };
  delete record.version;
  delete record.component_additions;
  delete record.plugin_additions;
  record.components = release.components.filter((entry) =>
    entry.id !== "cowboy.app-shell"
  );
  if (release.closure) {
    record.closure = {
      ...release.closure,
      component_dependencies: Object.fromEntries(
        Object.entries(release.closure.component_dependencies)
          .filter(([id]) => id !== "cowboy.app-shell"),
      ),
    };
  }
  return record;
}

export function workerRegistryProjection(
  source: string,
): { release: string; source: string } {
  const registry = JSON.parse(source) as Registry;
  const latest = registry.releases.at(-1);
  if (!latest || latest.version !== registry.active_release) {
    throw new Error("worker registry requires the active immutable release");
  }
  // A future internal consumer closes this exemption. Ordinary component and
  // Plugin closure checks still run independently before release publication.
  const shellConsumed = latest.closure === undefined ||
    Object.values(latest.closure.component_dependencies).some((edges) =>
      edges.includes("cowboy.app-shell")
    ) ||
    Object.values(latest.closure.plugins).some((plugin) =>
      plugin.components.some((entry) => entry.id === "cowboy.app-shell")
    );
  let selected = registry.releases.length - 1;
  if (!shellConsumed) {
    while (
      selected > 0 && canonical(workerRecord(registry.releases[selected]!)) ===
        canonical(workerRecord(registry.releases[selected - 1]!))
    ) {
      selected--;
    }
  }
  const release = registry.releases[selected]!.version;
  return {
    release,
    // Preserve the historical two-space registry encoding as the existing
    // hash input. No magic generation label or administrator-selected pin.
    source: JSON.stringify(
      {
        ...registry,
        active_release: release,
        releases: registry.releases.slice(0, selected + 1),
      },
      null,
      2,
    ) + "\n",
  };
}

export async function workerRegistryInput(source: string) {
  const projected = workerRegistryProjection(source);
  const digest = new Uint8Array(
    await crypto.subtle.digest(
      "SHA-256",
      new TextEncoder().encode(projected.source),
    ),
  );
  return {
    schema: 1,
    component_release: projected.release,
    registry_sha256: Array.from(
      digest,
      (byte) => byte.toString(16).padStart(2, "0"),
    ).join(""),
  };
}

if (import.meta.main) {
  const expected = JSON.stringify(
    await workerRegistryInput(
      readFileSync("components/registry.json", "utf8"),
    ),
    null,
    2,
  ) + "\n";
  const path = "components/worker-registry-input.json";
  if (
    process.argv.slice(2).length === 1 && process.argv.slice(2)[0] === "--write"
  ) {
    writeFileSync(path, expected);
  } else if (process.argv.slice(2).length !== 0) {
    throw new Error("usage: worker-registry-input.ts [--write]");
  } else if (readFileSync(path, "utf8") !== expected) {
    throw new Error(
      "worker registry input is stale; regenerate the derived input before building",
    );
  }
}

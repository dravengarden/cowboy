// The component registry is one ordered, append-only history split across two
// files: components/registry.json keeps the active release and every release
// an in-tree Plugin or the worker pin still references; older releases live
// unchanged in components/registry-archive.json. Every reader validates the
// merged history, so archiving a release never changes what it proves.

import { readFileSync } from "node:fs";

export const LIVE_REGISTRY = "components/registry.json";
export const ARCHIVE_REGISTRY = "components/registry-archive.json";

interface Versioned {
  version: string;
}

interface RegistryFile<Release extends Versioned> {
  schema_version: number;
  releases: Release[];
}

function versionKey(version: string): number[] {
  const parts = version.split(".").map((part) => {
    if (!/^(0|[1-9][0-9]*)$/.test(part)) throw new Error(`invalid component release ${version}`);
    return Number(part);
  });
  if (parts.length !== 3) throw new Error(`invalid component release ${version}`);
  return parts;
}

function compareVersions(a: string, b: string): number {
  const [x, y] = [versionKey(a), versionKey(b)];
  for (let index = 0; index < 3; index++) {
    if (x[index] !== y[index]) return x[index]! - y[index]!;
  }
  return 0;
}

/** Merge the live registry with its archive into the complete ordered
 *  history. The live file's own keys (and their order) are preserved. */
export function mergeComponentRegistry<
  Release extends Versioned,
  Live extends RegistryFile<Release>,
>(live: Live, archive: RegistryFile<Release>): Live {
  if (archive.schema_version !== live.schema_version) {
    throw new Error("component registry archive schema differs from the live registry");
  }
  const releases = [...archive.releases, ...live.releases].sort((a, b) =>
    compareVersions(a.version, b.version),
  );
  for (let index = 1; index < releases.length; index++) {
    if (compareVersions(releases[index - 1]!.version, releases[index]!.version) === 0) {
      throw new Error(`component release ${releases[index]!.version} is recorded twice`);
    }
  }
  if (releases.at(-1) !== live.releases.at(-1)) {
    throw new Error("the active component release must be the newest live release");
  }
  return { ...live, releases };
}

export function readComponentRegistrySources(root = "."): { live: string; archive: string } {
  return {
    live: readFileSync(`${root}/${LIVE_REGISTRY}`, "utf8"),
    archive: readFileSync(`${root}/${ARCHIVE_REGISTRY}`, "utf8"),
  };
}

/** The merged registry, re-encoded in the historical two-space form. */
export function mergedComponentRegistrySource(sources: { live: string; archive: string }): string {
  return (
    JSON.stringify(
      mergeComponentRegistry(JSON.parse(sources.live), JSON.parse(sources.archive)),
      null,
      2,
    ) + "\n"
  );
}

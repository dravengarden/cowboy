import { readFile } from "node:fs/promises";

/**
 * Vendor archives Cowboy must not redistribute stay at the vendor's pinned
 * URL; each Machine downloads and verifies them itself. Only an exact match
 * with a reviewed `vendor_archive` runtime lock recipe qualifies, so any other
 * runtime artifact must still be a Catalog publication route.
 */
export interface VendorCandidate {
  dependency: string;
  version: string;
  command: string;
  artifact_url: string;
  artifact_digest: string;
}

interface RuntimeLock {
  components: Record<string, {
    kind: string;
    version: string;
    targets?: Record<string, { url: string; sha256?: string }>;
  }>;
}

const runtimeLock = JSON.parse(
  await readFile(
    new URL("../components/provider-runtime/lock.json", import.meta.url),
    "utf8",
  ),
) as RuntimeLock;

export function vendorHosted(
  target: { os: string; architecture: string },
  component: VendorCandidate,
  lock: RuntimeLock = runtimeLock,
): boolean {
  const recipe = lock.components[component.dependency];
  if (recipe?.kind !== "vendor_archive") return false;
  const locked = recipe.targets?.[`${target.os}-${target.architecture}`];
  if (
    recipe.version !== component.version || locked === undefined ||
    locked.url !== component.artifact_url ||
    component.artifact_digest !== `sha256:${locked.sha256}`
  ) {
    throw new Error(
      `vendor runtime artifact ${component.command} does not match the runtime lock`,
    );
  }
  return true;
}
